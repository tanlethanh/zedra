use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;
use zedra_rpc::proto::*;

use crate::agent_utils::{
    command_on_path, empty_session_live, file_size_bytes, home_path, resume_summary, session_title,
};

const LIST_HEAD_SCAN_MAX_LINES: usize = 32;

pub struct SessionCounts {
    pub total: usize,
    pub resumable: usize,
    pub latest_session_id: Option<String>,
    pub latest_session_title: Option<String>,
    pub last_activity_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
struct PiSessionFile {
    path: PathBuf,
    session_id: String,
    cwd: Option<String>,
    created_at: Option<DateTime<Utc>>,
    last_activity_at: Option<DateTime<Utc>>,
    title: Option<String>,
    message_count: u64,
    malformed_line_count: u64,
}

pub fn cli_available() -> bool {
    command_on_path("pi") || pi_sessions_root().is_dir()
}

pub fn normalize_event(_event_name: &str) -> Option<(AgentEventKind, AgentLifecycleStatus)> {
    None
}

pub fn session_counts(workdir: &Path) -> Result<SessionCounts, String> {
    let files = collect_session_files(workdir, Some(1), true).map_err(|e| e.to_string())?;
    let total = count_session_files(workdir).map_err(|e| e.to_string())?;
    let latest = files.first();
    Ok(SessionCounts {
        total,
        resumable: total,
        latest_session_id: latest.map(|f| f.session_id.clone()),
        latest_session_title: latest.and_then(|f| f.title.clone()),
        last_activity_at: latest.and_then(|f| f.last_activity_at),
    })
}

pub fn sessions(
    workdir: &Path,
    cli: &AgentCliSummary,
    limit: usize,
) -> Result<(Vec<AgentSessionSummary>, usize), String> {
    let total = count_session_files(workdir).map_err(|e| e.to_string())?;
    let files = collect_session_files(workdir, Some(limit), true).map_err(|e| e.to_string())?;
    let summaries = files
        .iter()
        .map(|file| session_summary(file, cli))
        .collect();
    Ok((summaries, total))
}

pub fn account_fields() -> Vec<AgentInfoField> {
    let mut fields = Vec::new();
    fields.push(AgentInfoField {
        label: "Logged in".to_string(),
        value: if pi_sessions_root().is_dir() {
            "yes"
        } else {
            "no"
        }
        .to_string(),
    });
    fields
}

pub fn subscription_plan_fields() -> Option<Vec<AgentInfoField>> {
    None
}

pub fn fetch_account_usage() -> Option<AgentUsageSnapshot> {
    None
}

// ---------------------------------------------------------------------------
// File-system scan
// ---------------------------------------------------------------------------

fn pi_sessions_root() -> PathBuf {
    home_path(&[".pi", "agent", "sessions"])
}

fn encoded_project_dir(workdir: &Path) -> String {
    let encoded: String = workdir
        .to_string_lossy()
        .chars()
        .map(|ch| match ch {
            '/' | '\\' => '-',
            _ => ch,
        })
        .collect();
    format!("-{encoded}--")
}

fn project_dir(workdir: &Path) -> PathBuf {
    pi_sessions_root().join(encoded_project_dir(workdir))
}

fn count_session_files(workdir: &Path) -> Result<usize> {
    let dir = project_dir(workdir);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(err) => {
            return Err(err)
                .with_context(|| format!("failed to read Pi project dir {}", dir.display()));
        }
    };
    let mut count = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            count += 1;
        }
    }
    Ok(count)
}

fn collect_session_files(
    workdir: &Path,
    limit: Option<usize>,
    head_only: bool,
) -> Result<Vec<PiSessionFile>> {
    let dir = project_dir(workdir);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => {
            return Err(err)
                .with_context(|| format!("failed to read Pi project dir {}", dir.display()));
        }
    };

    let mut candidates: Vec<(PathBuf, Option<u64>)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            continue;
        }
        candidates.push((path.clone(), mtime_unix_secs(&path)));
    }
    candidates.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.0.cmp(&a.0)));

    let take = limit.unwrap_or(candidates.len());
    let mut files = Vec::with_capacity(take.min(candidates.len()));
    for (path, mtime) in candidates.into_iter().take(take) {
        let file = read_session_file(&path, mtime, head_only)?;
        files.push(file);
    }
    Ok(files)
}

fn read_session_file(
    path: &Path,
    mtime_unix_secs: Option<u64>,
    head_only: bool,
) -> Result<PiSessionFile> {
    let file = File::open(path)
        .with_context(|| format!("failed to read Pi transcript {}", path.display()))?;
    let mut session_id = String::new();
    let mut cwd = None;
    let mut created_at = None;
    let mut last_timestamp: Option<DateTime<Utc>> = None;
    let mut title: Option<String> = None;
    let mut message_count: u64 = 0;
    let mut malformed_line_count: u64 = 0;
    let mut scanned_lines = 0usize;

    for line in BufReader::new(file).lines() {
        let line = line.with_context(|| format!("failed to read line in {}", path.display()))?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let record = match serde_json::from_str::<Value>(trimmed) {
            Ok(Value::Object(record)) => Value::Object(record),
            _ => {
                malformed_line_count += 1;
                continue;
            }
        };
        scanned_lines += 1;

        let record_type = string_field(&record, &["type"]).unwrap_or("");
        match record_type {
            "session" => {
                if session_id.is_empty() {
                    if let Some(id) = string_field(&record, &["id"]) {
                        session_id = id.to_string();
                    }
                }
                if cwd.is_none() {
                    cwd = string_field(&record, &["cwd"]).map(str::to_string);
                }
                if created_at.is_none() {
                    if let Some(ts) = string_field(&record, &["timestamp"]) {
                        created_at = parse_timestamp(ts);
                    }
                }
            }
            "session_info" => {
                if let Some(name) = string_field(&record, &["displayName", "display_name", "name"])
                {
                    title = Some(name.to_string());
                }
            }
            "label" => {
                if title.is_none() {
                    if let Some(label) = string_field(&record, &["label", "text", "value"]) {
                        title = Some(label.to_string());
                    }
                }
            }
            "message" => {
                message_count += 1;
                if title.is_none() {
                    if let Some(text) = first_user_text(&record) {
                        title = Some(truncate_title(&text));
                    }
                }
            }
            _ => {}
        }

        if let Some(ts) = string_field(&record, &["timestamp"]).and_then(parse_timestamp) {
            last_timestamp = match last_timestamp {
                Some(current) if current >= ts => Some(current),
                _ => Some(ts),
            };
        }

        if head_only
            && !session_id.is_empty()
            && title.is_some()
            && scanned_lines >= LIST_HEAD_SCAN_MAX_LINES
        {
            break;
        }
    }

    if session_id.is_empty() {
        session_id = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("unknown")
            .to_string();
    }

    let last_activity_at = last_timestamp.or_else(|| {
        mtime_unix_secs.and_then(|secs| DateTime::<Utc>::from_timestamp(secs as i64, 0))
    });

    Ok(PiSessionFile {
        path: path.to_path_buf(),
        session_id,
        cwd,
        created_at,
        last_activity_at,
        title,
        message_count,
        malformed_line_count,
    })
}

fn session_summary(file: &PiSessionFile, cli: &AgentCliSummary) -> AgentSessionSummary {
    AgentSessionSummary {
        kind: ManagedAgentKind::Pi,
        session_id: file.session_id.clone(),
        title: session_title(file.title.clone()),
        cwd: file.cwd.clone(),
        created_at: file.created_at,
        last_activity_at: file.last_activity_at,
        resume: resume_summary(ManagedAgentKind::Pi, &file.session_id),
        live: empty_session_live(),
        provider: AgentProviderSessionInfo {
            model: None,
            permission_mode: None,
            cli_version: cli.version.clone(),
            origin: None,
            source: None,
            entrypoint: None,
            native_project_id: None,
            model_provider: None,
        },
        git: None,
        usage: None,
        counters: AgentSessionCounters {
            record_count: file.message_count,
            message_count: file.message_count,
            turn_count: 0,
            tool_count: 0,
            tool_failure_count: 0,
            hook_success_count: 0,
            hook_failure_count: 0,
            malformed_record_count: file.malformed_line_count,
        },
        flags: AgentSessionFlags {
            is_sidechain: false,
            is_subagent: false,
            is_archived: false,
            historical_only: true,
            live_bound: false,
        },
        data_sources: vec![AgentDataSource::HistoricalScan],
        warnings: crate::agent_utils::malformed_warning(file.malformed_line_count as usize),
        transcript_size_bytes: file_size_bytes(&file.path),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn string_field<'a>(record: &'a Value, names: &[&str]) -> Option<&'a str> {
    names
        .iter()
        .find_map(|name| record.get(*name)?.as_str())
        .filter(|value| !value.is_empty())
}

fn first_user_text(record: &Value) -> Option<String> {
    let message = record.get("message").unwrap_or(record);
    let role = string_field(message, &["role"]).unwrap_or("");
    if role != "user" {
        return None;
    }
    let content = message.get("content")?;
    let text = if let Some(text) = content.as_str() {
        text.to_string()
    } else if let Some(array) = content.as_array() {
        array.iter().find_map(|part| {
            if string_field(part, &["type"]) == Some("text") {
                string_field(part, &["text"]).map(str::to_string)
            } else {
                None
            }
        })?
    } else {
        return None;
    };
    let text = text.trim();
    if text.is_empty() || text.starts_with('<') {
        return None;
    }
    Some(text.to_string())
}

fn truncate_title(text: &str) -> String {
    const MAX_LEN: usize = 80;
    if text.chars().count() <= MAX_LEN {
        return text.to_string();
    }
    let mut end = 0;
    for (index, _) in text.char_indices().take(MAX_LEN) {
        end = index;
    }
    format!("{}…", &text[..end])
}

fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn mtime_unix_secs(path: &Path) -> Option<u64> {
    std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_project_dir_with_pi_wrapping() {
        assert_eq!(
            encoded_project_dir(Path::new("/Users/me/project")),
            "--Users-me-project--"
        );
    }

    #[test]
    fn reads_session_header_and_first_user_message() {
        let dir = tempfile::tempdir().unwrap();
        let workdir = Path::new("/Users/me/project");
        let sessions = dir.path().join(encoded_project_dir(workdir));
        std::fs::create_dir_all(&sessions).unwrap();
        let path = sessions.join("2026-05-28_abc.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"session","version":3,"id":"abc","timestamp":"2026-05-28T10:00:00Z","cwd":"/Users/me/project"}
{"type":"message","id":"a","message":{"role":"user","content":[{"type":"text","text":"Refactor terminal scrollback"}]}}
{"type":"message","id":"b","message":{"role":"assistant","content":[]}}
"#,
        )
        .unwrap();

        let file = read_session_file(&path, None, false).unwrap();
        assert_eq!(file.session_id, "abc");
        assert_eq!(file.cwd.as_deref(), Some("/Users/me/project"));
        assert_eq!(file.title.as_deref(), Some("Refactor terminal scrollback"));
        assert_eq!(file.message_count, 2);
        assert_eq!(file.malformed_line_count, 0);
    }

    #[test]
    fn falls_back_to_filename_when_session_id_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("2026-05-28_xyz.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"message","message":{"role":"user","content":[{"type":"text","text":"hi"}]}}
"#,
        )
        .unwrap();
        let file = read_session_file(&path, None, false).unwrap();
        assert_eq!(file.session_id, "2026-05-28_xyz");
    }
}
