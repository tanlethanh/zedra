use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;
use zedra_rpc::proto::*;

use super::utils::{
    command_on_path, cwd_matches, file_size_bytes, home_path, info_field, mtime_unix_secs,
    resume_summary, session_title, string_field,
};

/// Maki appends a `meta` record after every turn, so the freshest title and
/// `updated_at` live in the last record. Tail-read (doubling, capped) finds it
/// without scanning a whole transcript.
const META_TAIL_START: u64 = 4096;
const META_TAIL_MAX: u64 = 256 * 1024;

#[derive(Debug, Clone)]
struct MakiSessionFile {
    path: PathBuf,
    session_id: String,
    cwd: Option<String>,
    created_at: Option<DateTime<Utc>>,
    last_activity_at: Option<DateTime<Utc>>,
    title: Option<String>,
}

impl MakiActor {
    pub fn cli_available() -> bool {
        command_on_path("maki") || Self::sessions_dir().is_dir()
    }

    pub fn session_counts(workdir: &Path) -> Result<super::SessionCounts, String> {
        let (files, total) = Self::scan_sessions_in(&Self::sessions_dir(), workdir, Some(1))
            .map_err(|e| e.to_string())?;
        let latest = files.into_iter().next();
        Ok(super::SessionCounts::from_latest(
            total,
            latest.as_ref().map(|f| f.session_id.clone()),
            latest.as_ref().and_then(|f| f.title.clone()),
            latest.and_then(|f| f.last_activity_at),
        ))
    }

    pub fn sessions(
        workdir: &Path,
        cli: &AgentCliSummary,
        limit: usize,
    ) -> Result<(Vec<AgentSessionSummary>, usize), String> {
        let (files, total) = Self::scan_sessions_in(&Self::sessions_dir(), workdir, Some(limit))
            .map_err(|e| e.to_string())?;
        let summaries = files
            .iter()
            .map(|file| Self::session_summary(file, cli))
            .collect();
        Ok((summaries, total))
    }

    pub fn account_fields(workdir: &Path) -> Vec<AgentInfoField> {
        let mut fields = Vec::new();
        if let Some(model) = Self::default_model(workdir) {
            fields.push(info_field("Default model", &model));
        }
        let has_config =
            Self::config_dir().join("config.toml").is_file() || home_path(&[".maki"]).is_dir();
        fields.push(info_field("Config", if has_config { "yes" } else { "no" }));
        fields
    }

    // ---------------------------------------------------------------------------
    // File-system scan
    // ---------------------------------------------------------------------------

    /// Maki's state dir mirrors `maki-storage::paths`: XDG state (`~/.local/state`
    /// or `$XDG_STATE_HOME`) joined with `maki`. No fallback to legacy `~/.maki`.
    fn state_dir() -> PathBuf {
        if let Some(xdg) = std::env::var_os("XDG_STATE_HOME")
            .filter(|v| !v.is_empty() && Path::new(v).is_absolute())
        {
            return PathBuf::from(xdg).join("maki");
        }
        home_path(&[".local", "state", "maki"])
    }

    fn config_dir() -> PathBuf {
        if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty() && Path::new(v).is_absolute())
        {
            return PathBuf::from(xdg).join("maki");
        }
        home_path(&[".config", "maki"])
    }

    fn sessions_dir() -> PathBuf {
        Self::state_dir().join("sessions")
    }

    /// One directory pass: newest-mtime candidates first, filtered to this
    /// workdir by header `cwd`. Returns up to `limit` parsed files plus the
    /// total matched count (every header match counts, even when limit caps the
    /// parsed set).
    fn scan_sessions_in(
        dir: &Path,
        workdir: &Path,
        limit: Option<usize>,
    ) -> Result<(Vec<MakiSessionFile>, usize)> {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok((Vec::new(), 0));
            }
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("failed to read maki sessions {}", dir.display()));
            }
        };

        // mtime is a cheap recency proxy that avoids opening files to sort; the
        // parsed `updated_at` refines ordering for the collected files.
        let mut candidates: Vec<(PathBuf, Option<u64>)> = Vec::new();
        for entry in entries {
            let entry = entry.with_context(|| {
                format!("failed to read maki session entry in {}", dir.display())
            })?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
                continue;
            }
            candidates.push((path.clone(), mtime_unix_secs(&path)));
        }
        candidates.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.0.cmp(&a.0)));

        let mut files = Vec::new();
        let mut total = 0;
        for (path, mtime) in candidates {
            // Skip transcripts whose header cwd does not match this workspace.
            if !Self::session_matches_workdir(&path, workdir) {
                continue;
            }
            total += 1;
            if limit.is_some_and(|n| files.len() >= n) {
                continue;
            }
            if let Some(file) = Self::read_session_file(&path, mtime)? {
                files.push(file);
            }
        }
        Ok((files, total))
    }

    /// True if the transcript's header `cwd` matches `workdir`. Reads only the
    /// first line so a directory of unrelated sessions stays cheap to filter.
    fn session_matches_workdir(path: &Path, workdir: &Path) -> bool {
        let Ok(file) = File::open(path) else {
            return false;
        };
        let mut reader = std::io::BufReader::new(file);
        let mut line = String::new();
        if std::io::BufRead::read_line(&mut reader, &mut line).is_err() {
            return false;
        }
        let Ok(record) = serde_json::from_str::<Value>(line.trim()) else {
            return false;
        };
        // Only the header record carries `cwd`; other record types would
        // otherwise mismatch every session.
        if string_field(&record, &["t"]) != Some("header") {
            return false;
        }
        string_field(&record, &["cwd"])
            .map(|cwd| cwd_matches(workdir, Some(cwd)))
            .unwrap_or(false)
    }

    fn read_session_file(path: &Path, mtime: Option<u64>) -> Result<Option<MakiSessionFile>> {
        let mut file = File::open(path)
            .with_context(|| format!("failed to read maki transcript {}", path.display()))?;

        let header = Self::read_header(&mut file)?;
        let (id, cwd, created_at) = match header {
            Some(header) => header,
            // Not a valid maki transcript header (or empty); skip rather than guess.
            None => return Ok(None),
        };

        let (title, updated_at) = Self::read_last_meta(&mut file).unwrap_or((None, None));

        let mtime_activity = mtime.and_then(|secs| DateTime::<Utc>::from_timestamp(secs as i64, 0));
        let last_activity_at = updated_at.or(mtime_activity);

        Ok(Some(MakiSessionFile {
            path: path.to_path_buf(),
            session_id: id,
            cwd,
            created_at,
            last_activity_at,
            title,
        }))
    }

    /// Parse the first JSONL record as a maki header:
    /// `{"t":"header","id":..,"cwd":..,"created_at":..}`.
    fn read_header(
        file: &mut File,
    ) -> Result<Option<(String, Option<String>, Option<DateTime<Utc>>)>> {
        let mut reader = std::io::BufReader::new(&mut *file);
        let mut line = String::new();
        std::io::BufRead::read_line(&mut reader, &mut line)
            .with_context(|| "failed to read maki header line")?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }
        let Ok(record) = serde_json::from_str::<Value>(trimmed) else {
            return Ok(None);
        };
        if string_field(&record, &["t"]) != Some("header") {
            return Ok(None);
        }
        // The id is required to build a valid resume command.
        let Some(id) = string_field(&record, &["id"])
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
        else {
            return Ok(None);
        };
        let cwd = string_field(&record, &["cwd"]).map(str::to_string);
        let created_at = record
            .get("created_at")
            .and_then(Value::as_u64)
            .and_then(|secs| DateTime::<Utc>::from_timestamp(secs as i64, 0));
        Ok(Some((id, cwd, created_at)))
    }

    /// Tail-read the file (doubling the window up to [`META_TAIL_MAX`]) to find
    /// the last `meta` record, which carries the freshest `title` and
    /// `updated_at`.
    fn read_last_meta(file: &mut File) -> Option<(Option<String>, Option<DateTime<Utc>>)> {
        let len = file.seek(SeekFrom::End(0)).ok()?;
        if len == 0 {
            return None;
        }
        let mut tail = META_TAIL_START.min(len);
        loop {
            file.seek(SeekFrom::End(-(tail as i64))).ok()?;
            let mut buf = vec![0u8; tail as usize];
            file.read_exact(&mut buf).ok()?;

            let content = buf.strip_suffix(b"\n").unwrap_or(&buf);
            // Walk whole records from the end; the newest meta record wins.
            let mut start = 0;
            if content.first() != Some(&b'\n') && tail < len {
                // The first byte may be a partial line when we didn't read from
                // offset 0; skip up to the first newline.
                if let Some(nl) = content.iter().position(|&b| b == b'\n') {
                    start = nl + 1;
                }
            }

            let mut found = None;
            let mut idx = start;
            while idx < content.len() {
                let end = content[idx..]
                    .iter()
                    .position(|&b| b == b'\n')
                    .map(|p| idx + p)
                    .unwrap_or(content.len());
                let slice = &content[idx..end];
                if let Ok(record) =
                    serde_json::from_str::<Value>(std::str::from_utf8(slice).unwrap_or("").trim())
                {
                    if string_field(&record, &["t"]) == Some("meta") {
                        let title = string_field(&record, &["title"]).map(str::to_string);
                        let updated_at = record
                            .get("updated_at")
                            .and_then(Value::as_u64)
                            .and_then(|secs| DateTime::<Utc>::from_timestamp(secs as i64, 0));
                        found = Some((title, updated_at));
                    }
                }
                if end == content.len() {
                    break;
                }
                idx = end + 1;
            }
            if found.is_some() {
                return found;
            }

            // Stop once the whole file is scanned, or we hit the read cap — a
            // meta record sits near EOF (appended every turn), so anything past
            // META_TAIL_MAX is not worth reading.
            if tail >= len || tail >= META_TAIL_MAX {
                return None;
            }
            tail = (tail * 2).min(len).min(META_TAIL_MAX);
        }
    }

    fn session_summary(file: &MakiSessionFile, _cli: &AgentCliSummary) -> AgentSessionSummary {
        AgentSessionSummary {
            slug: "maki".to_string(),
            session_id: file.session_id.clone(),
            title: session_title(file.title.clone()),
            cwd: file.cwd.clone(),
            created_at: file.created_at,
            last_activity_at: file.last_activity_at,
            resume: resume_summary("maki", &file.session_id),
            git: None,
            usage: None,
            transcript_size_bytes: file_size_bytes(&file.path),
        }
    }

    // ---------------------------------------------------------------------------
    // Config (account info)
    // ---------------------------------------------------------------------------

    fn config_paths(workdir: &Path) -> Vec<PathBuf> {
        vec![
            workdir.join(".maki").join("init.lua"),
            Self::config_dir().join("init.lua"),
            workdir.join(".maki").join("config.toml"),
            Self::config_dir().join("config.toml"),
            home_path(&[".maki", "config.toml"]),
        ]
    }

    /// Default model from `[provider] default_model` in maki's TOML config. We
    /// parse the relevant line directly rather than pulling in a TOML dependency
    /// for one field, matching the lightweight approach the other agents take.
    fn default_model(workdir: &Path) -> Option<String> {
        Self::config_paths(workdir)
            .iter()
            .find_map(|path| Self::read_default_model(path))
    }

    fn read_default_model(path: &Path) -> Option<String> {
        let contents = std::fs::read_to_string(path).ok()?;
        let mut in_provider = false;
        for line in contents.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') {
                in_provider = trimmed == "[provider]";
                continue;
            }
            if in_provider {
                if let Some((key, value)) = trimmed.split_once('=') {
                    if key.trim() == "default_model" {
                        let model = value.trim().trim_matches('"').to_string();
                        if !model.is_empty() {
                            return Some(model);
                        }
                    }
                }
            }
        }
        None
    }
}

use super::{AgentActor, ScanCtx, SessionCounts as ActorSessionCounts};

pub(super) struct MakiActor;

impl AgentActor for MakiActor {
    fn shows_detail(&self) -> bool {
        true
    }

    fn slug(&self) -> &'static str {
        "maki"
    }
    fn display_name(&self) -> &'static str {
        "Maki"
    }
    fn icon_name(&self) -> &'static str {
        "maki"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["maki"]
    }

    fn detect_aliases(&self) -> &'static [&'static str] {
        &["maki"]
    }

    fn cli_available(&self, _workdir: &Path) -> bool {
        Self::cli_available()
    }

    fn session_counts(&self, ctx: &ScanCtx) -> Result<ActorSessionCounts, String> {
        Self::session_counts(ctx.workdir)
    }

    fn sessions(
        &self,
        ctx: &ScanCtx,
        limit: usize,
    ) -> Result<(Vec<AgentSessionSummary>, usize), String> {
        Self::sessions(ctx.workdir, ctx.cli, limit)
    }

    fn account_fields(&self, workdir: &Path) -> Vec<AgentInfoField> {
        Self::account_fields(workdir)
    }

    fn resume_launch_command(&self, quoted: &str) -> Option<String> {
        Some(format!("maki --resume {quoted}"))
    }

    // No hooks, setup, or remote plan/usage endpoint: maki auth is provider-local
    // (env keys / `maki auth login`). The trait defaults cover all of these.
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_session(dir: &Path, id: &str, cwd: &str, title: &str, updated_at: u64) -> PathBuf {
        let path = dir.join(format!("{id}.jsonl"));
        let header = serde_json::json!({
            "t": "header", "v": 2, "id": id, "model": "anthropic/claude-sonnet-4",
            "cwd": cwd, "created_at": updated_at - 100,
        });
        let msg = serde_json::json!({"t":"msg","d":{"role":"user","content":[{"type":"text","text":"Refactor scrollback"}]}});
        let meta = serde_json::json!({
            "t": "meta", "title": title, "token_usage": {}, "updated_at": updated_at,
            "mode": "build", "context_size": 0, "plan_written": false,
        });
        std::fs::write(&path, format!("{}\n{}\n{}\n", header, msg, meta)).unwrap();
        path
    }

    #[test]
    fn reads_header_title_and_activity() {
        let dir = tempfile::tempdir().unwrap();
        let sessions = dir.path().join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        let path = write_session(
            &sessions,
            "abc-123",
            "/home/me/repo",
            "Refactor scrollback",
            1_800_000_000,
        );

        let file = MakiActor::read_session_file(&path, None).unwrap().unwrap();
        assert_eq!(file.session_id, "abc-123");
        assert_eq!(file.cwd.as_deref(), Some("/home/me/repo"));
        assert_eq!(file.title.as_deref(), Some("Refactor scrollback"));
        assert_eq!(
            file.last_activity_at,
            DateTime::<Utc>::from_timestamp(1_800_000_000, 0)
        );
    }

    #[test]
    fn tail_meta_finds_latest_title_across_turns() {
        let dir = tempfile::tempdir().unwrap();
        let sessions = dir.path().join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        let path = sessions.join("multi.jsonl");
        let header =
            serde_json::json!({"t":"header","v":2,"id":"multi","cwd":"/p","created_at":100});
        let meta_v1 =
            serde_json::json!({"t":"meta","title":"v1","updated_at":200,"token_usage":{}});
        let meta_v2 =
            serde_json::json!({"t":"meta","title":"v2","updated_at":300,"token_usage":{}});
        std::fs::write(&path, format!("{header}\n{meta_v1}\n{meta_v2}\n")).unwrap();

        let file = MakiActor::read_session_file(&path, None).unwrap().unwrap();
        assert_eq!(file.title.as_deref(), Some("v2"));
        assert_eq!(
            file.last_activity_at,
            DateTime::<Utc>::from_timestamp(300, 0)
        );
    }

    #[test]
    fn non_header_first_line_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stray.jsonl");
        std::fs::write(&path, "{\"t\":\"msg\",\"d\":{}}\n").unwrap();
        assert!(MakiActor::read_session_file(&path, None).unwrap().is_none());
    }

    #[test]
    fn scan_sessions_filters_by_workdir_and_counts_total() {
        let dir = tempfile::tempdir().unwrap();
        let sessions = dir.path().join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        write_session(&sessions, "a", "/home/me/repo", "alpha", 1_800_000_010);
        write_session(&sessions, "b", "/home/me/repo", "beta", 1_800_000_020);
        write_session(&sessions, "c", "/other/project", "gamma", 1_800_000_030);

        let workdir = Path::new("/home/me/repo");
        let (files, total) = MakiActor::scan_sessions_in(&sessions, workdir, None).unwrap();
        assert_eq!(total, 2);
        assert_eq!(files.len(), 2);
        // Newest mtime first.
        assert_eq!(files[0].session_id, "b");
        assert_eq!(files[1].session_id, "a");

        // Limit caps the parsed set but not the total.
        let (files, total) = MakiActor::scan_sessions_in(&sessions, workdir, Some(1)).unwrap();
        assert_eq!(total, 2);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].session_id, "b");
    }

    #[test]
    fn reads_default_model_from_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[provider]\ndefault_model = \"anthropic/claude-sonnet-4-6\"\n[agent]\nmax_output_lines = 100\n",
        )
        .unwrap();
        assert_eq!(
            MakiActor::read_default_model(&path).as_deref(),
            Some("anthropic/claude-sonnet-4-6")
        );
    }
}
