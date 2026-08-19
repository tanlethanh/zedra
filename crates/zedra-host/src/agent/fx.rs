use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::Deserialize;
use zedra_rpc::proto::*;

use super::utils::{
    command_on_path, cwd_matches, file_size_bytes, home_path, info_field, resume_summary,
    session_title,
};
use super::{AgentActor, ScanCtx, SessionCounts as ActorSessionCounts};

/// fx keeps one index of every saved session, so a workspace scan is a single
/// file read; per-session files are only touched for transcript size.
#[derive(Debug, Deserialize)]
struct SessionIndex {
    #[serde(default)]
    sessions: Vec<IndexEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct IndexEntry {
    id: String,
    #[serde(default)]
    workspace_root: Option<String>,
    #[serde(default)]
    created_at_ms: Option<i64>,
    #[serde(default)]
    updated_at_ms: Option<i64>,
    #[serde(default)]
    title: Option<String>,
}

fn timestamp(ms: Option<i64>) -> Option<DateTime<Utc>> {
    ms.and_then(DateTime::<Utc>::from_timestamp_millis)
}

pub(super) struct FxActor;

impl FxActor {
    fn state_dir() -> PathBuf {
        home_path(&[".fx"])
    }

    fn sessions_dir() -> PathBuf {
        Self::state_dir().join("sessions")
    }

    fn cli_available() -> bool {
        command_on_path("fx") || Self::state_dir().is_dir()
    }

    /// Index entries for `workdir`, newest first. `total` is every match; the
    /// returned vec is capped by `limit`.
    fn scan_sessions(
        dir: &Path,
        workdir: &Path,
        limit: Option<usize>,
    ) -> Result<(Vec<IndexEntry>, usize), String> {
        let path = dir.join("index.json");
        let contents = match std::fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok((Vec::new(), 0)),
            Err(err) => return Err(format!("failed to read {}: {err}", path.display())),
        };
        let index: SessionIndex = serde_json::from_str(&contents)
            .map_err(|err| format!("failed to parse {}: {err}", path.display()))?;

        let mut matched: Vec<IndexEntry> = index
            .sessions
            .into_iter()
            .filter(|entry| {
                !entry.id.trim().is_empty() && cwd_matches(workdir, entry.workspace_root.as_deref())
            })
            .collect();
        matched.sort_by(|a, b| {
            b.updated_at_ms
                .cmp(&a.updated_at_ms)
                .then_with(|| b.id.cmp(&a.id))
        });

        let total = matched.len();
        if let Some(limit) = limit {
            matched.truncate(limit);
        }
        Ok((matched, total))
    }

    fn session_summary(&self, entry: &IndexEntry) -> AgentSessionSummary {
        let transcript = Self::sessions_dir().join(&entry.id).join("events.jsonl");
        AgentSessionSummary {
            slug: "fx".to_string(),
            session_id: entry.id.clone(),
            title: session_title(entry.title.clone()),
            cwd: entry.workspace_root.clone(),
            created_at: timestamp(entry.created_at_ms),
            last_activity_at: timestamp(entry.updated_at_ms),
            resume: resume_summary("fx", &entry.id),
            git: None,
            usage: None,
            transcript_size_bytes: file_size_bytes(&transcript),
        }
    }

    /// Global preferences; workspace permission rules live under the same file
    /// keyed by workspace root.
    fn settings() -> Option<serde_json::Value> {
        let contents = std::fs::read_to_string(Self::state_dir().join("settings.json")).ok()?;
        serde_json::from_str(&contents).ok()
    }

    fn team_slug() -> Option<String> {
        let contents = std::fs::read_to_string(Self::state_dir().join("auth.json")).ok()?;
        let auth: serde_json::Value = serde_json::from_str(&contents).ok()?;
        auth.get("team_slug")
            .and_then(serde_json::Value::as_str)
            .filter(|slug| !slug.is_empty())
            .map(str::to_string)
    }
}

impl AgentActor for FxActor {
    fn shows_detail(&self) -> bool {
        true
    }

    fn slug(&self) -> &'static str {
        "fx"
    }
    fn display_name(&self) -> &'static str {
        "fx"
    }
    fn icon_name(&self) -> &'static str {
        "fx"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["fx"]
    }

    // `fx` is too short to match inside a command line without false hits.
    fn detect_exact(&self) -> &'static [&'static str] {
        &["fx"]
    }

    fn cli_available(&self, _workdir: &Path) -> bool {
        Self::cli_available()
    }

    fn session_counts(&self, ctx: &ScanCtx) -> Result<ActorSessionCounts, String> {
        let (entries, total) = Self::scan_sessions(&Self::sessions_dir(), ctx.workdir, Some(1))?;
        let latest = entries.into_iter().next();
        Ok(ActorSessionCounts::from_latest(
            total,
            latest.as_ref().map(|entry| entry.id.clone()),
            latest.as_ref().and_then(|entry| entry.title.clone()),
            latest.and_then(|entry| timestamp(entry.updated_at_ms)),
        ))
    }

    fn sessions(
        &self,
        ctx: &ScanCtx,
        limit: usize,
    ) -> Result<(Vec<AgentSessionSummary>, usize), String> {
        let (entries, total) =
            Self::scan_sessions(&Self::sessions_dir(), ctx.workdir, Some(limit))?;
        let summaries = entries
            .iter()
            .map(|entry| self.session_summary(entry))
            .collect();
        Ok((summaries, total))
    }

    fn account_fields(&self, _workdir: &Path) -> Vec<AgentInfoField> {
        let mut fields = Vec::new();
        if let Some(source) = Self::settings()
            .as_ref()
            .and_then(|settings| settings.get("credential_source"))
            .and_then(serde_json::Value::as_str)
        {
            let label = match source {
                "fx_login" => "Vercel login",
                "api_key" => "AI Gateway API key",
                other => other,
            };
            fields.push(info_field("Auth", label));
        }
        if let Some(team) = Self::team_slug() {
            fields.push(info_field("Team", &team));
        }
        if let Ok(model) = std::env::var("FX_MODEL") {
            if !model.is_empty() {
                fields.push(info_field("Model", &model));
            }
        }
        fields
    }

    // fx reads the permission mode from the environment; `yolo` is its bypass.
    fn default_launch_command(&self) -> Option<String> {
        self.resolved_program()
            .map(|program| format!("FX_PERMISSION_MODE=yolo {program}"))
    }

    fn resume_launch_command(&self, quoted: &str) -> Option<String> {
        Some(format!("FX_PERMISSION_MODE=yolo fx --resume {quoted}"))
    }
}

#[cfg(test)]
mod tests {
    use super::super::utils::shell_quote;
    use super::*;

    fn write_index(dir: &Path, entries: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("index.json"),
            format!("{{\"schema_version\":3,\"sessions\":[{entries}]}}"),
        )
        .unwrap();
    }

    #[test]
    fn scans_index_filtered_by_workspace_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        write_index(
            dir.path(),
            r#"
            {"id":"a","workspace_root":"/repo","created_at_ms":1000,"updated_at_ms":2000,"title":"alpha"},
            {"id":"b","workspace_root":"/repo","created_at_ms":1000,"updated_at_ms":5000,"title":"beta"},
            {"id":"c","workspace_root":"/other","created_at_ms":1000,"updated_at_ms":9000,"title":"gamma"}
            "#,
        );

        let (entries, total) =
            FxActor::scan_sessions(dir.path(), Path::new("/repo"), None).unwrap();
        assert_eq!(total, 2);
        assert_eq!(entries[0].id, "b");
        assert_eq!(entries[1].id, "a");

        let (entries, total) =
            FxActor::scan_sessions(dir.path(), Path::new("/repo"), Some(1)).unwrap();
        assert_eq!(total, 2);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "b");
    }

    #[test]
    fn missing_index_is_empty_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let (entries, total) =
            FxActor::scan_sessions(dir.path(), Path::new("/repo"), None).unwrap();
        assert!(entries.is_empty());
        assert_eq!(total, 0);
    }

    #[test]
    fn resume_command_keeps_permission_bypass() {
        assert_eq!(
            FxActor.resume_launch_command(&shell_quote("s 1")).unwrap(),
            "FX_PERMISSION_MODE=yolo fx --resume 's 1'"
        );
    }
}
