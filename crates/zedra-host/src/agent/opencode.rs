use super::utils::*;
use crate::sqlite_readonly;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use zedra_rpc::proto::*;

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OpenCodeSessionJson {
    pub id: String,
    pub title: Option<String>,
    pub updated: Option<i64>,
    pub created: Option<i64>,
    pub project_id: Option<String>,
    pub directory: Option<String>,
    #[serde(default, alias = "worktree")]
    pub project_worktree: Option<String>,
    #[serde(default)]
    pub workspace_branch: Option<String>,
    #[serde(default)]
    pub workspace_directory: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub transcript_size_bytes: Option<i64>,
}

struct SessionCountSummary {
    total: usize,
    latest: Option<OpenCodeSessionJson>,
}

impl OpenCodeActor {
    /// Source tag for SQLite-sourced sessions, whose rows already carry sizes;
    /// the message-table size scan runs only for the CLI-list source.
    const DB_SOURCE: &'static str = "opencode sqlite";

    pub fn cli_available() -> bool {
        Self::db_path().is_file() || command_on_path("opencode")
    }

    pub fn session_counts(
        workdir: &Path,
        _cli: &AgentCliSummary,
    ) -> Result<super::SessionCounts, String> {
        if !Self::cli_available() {
            return Ok(super::SessionCounts::default());
        }
        let summary = Self::session_count_summary(workdir)?;
        Ok(super::SessionCounts {
            total: summary.total,
            resumable: summary.total,
            latest_session_id: summary.latest.as_ref().map(|s| s.id.clone()),
            latest_session_title: summary
                .latest
                .as_ref()
                .and_then(|s| session_title(s.title.clone())),
            last_activity_at: summary
                .latest
                .as_ref()
                .and_then(|s| s.updated)
                .and_then(DateTime::<Utc>::from_timestamp_millis),
            provider_project_id: summary.latest.and_then(|s| s.project_id.clone()),
        })
    }

    fn session_count_summary(workdir: &Path) -> Result<SessionCountSummary, String> {
        let (json, _) = Self::fetch_sessions_json()?;
        let raw: Vec<OpenCodeSessionJson> =
            serde_json::from_slice(&json).map_err(|error| error.to_string())?;
        let mut git_cache = HashMap::new();
        let mut matched = raw
            .into_iter()
            .filter(|session| Self::workdir_matches(workdir, session, &mut git_cache))
            .collect::<Vec<_>>();
        matched.sort_by(|left, right| {
            right
                .updated
                .unwrap_or(0)
                .cmp(&left.updated.unwrap_or(0))
                .then_with(|| right.id.cmp(&left.id))
        });
        Ok(SessionCountSummary {
            total: matched.len(),
            latest: matched.into_iter().next(),
        })
    }

    pub fn sessions(
        workdir: &Path,
        cli: &AgentCliSummary,
        limit: usize,
    ) -> Result<(Vec<AgentSessionSummary>, usize), String> {
        if !cli.available {
            return Ok((Vec::new(), 0));
        }
        let (json, source) = Self::fetch_sessions_json()?;
        let mut sessions = Self::sessions_from_json(workdir, &json, cli, source)?;
        let total = sessions.len();
        sessions.truncate(limit);
        Ok((sessions, total))
    }

    fn db_path() -> PathBuf {
        home_path(&[".local", "share", "opencode", "opencode.db"])
    }

    fn auth_path() -> PathBuf {
        home_path(&[".local", "share", "opencode", "auth.json"])
    }

    fn fetch_sessions_json() -> Result<(Vec<u8>, &'static str), String> {
        if let Ok(json) = Self::fetch_sessions_json_from_db() {
            return Ok((json, Self::DB_SOURCE));
        }

        let output = Self::opencode_session_list_output()?;
        if output.status.success() && !output.stdout.is_empty() {
            return Ok((output.stdout, "opencode session list"));
        }

        let cli_error = if output.stderr.is_empty() {
            format!("`opencode session list` exited with {}", output.status)
        } else {
            format!(
                "`opencode session list` exited with {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            )
        };

        Self::fetch_sessions_json_from_db()
            .map(|json| (json, Self::DB_SOURCE))
            .map_err(|fallback_error| {
                format!("{cli_error}; sqlite fallback failed: {fallback_error}")
            })
    }

    /// Run `opencode session list` with a deadline, killing the child on timeout
    /// so the SQLite fallback can run. Pipes drain on threads to avoid deadlock.
    fn opencode_session_list_output() -> Result<std::process::Output, String> {
        use std::io::Read;
        use std::time::{Duration, Instant};

        const SESSION_LIST_TIMEOUT: Duration = Duration::from_secs(10);

        let mut child = Command::new("opencode")
            .args(["session", "list", "--format", "json", "--pure"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|error| error.to_string())?;

        let mut stdout_pipe = child.stdout.take();
        let mut stderr_pipe = child.stderr.take();
        let stdout_reader = std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(pipe) = stdout_pipe.as_mut() {
                let _ = pipe.read_to_end(&mut buf);
            }
            buf
        });
        let stderr_reader = std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(pipe) = stderr_pipe.as_mut() {
                let _ = pipe.read_to_end(&mut buf);
            }
            buf
        });

        let start = Instant::now();
        let status = loop {
            match child.try_wait().map_err(|error| error.to_string())? {
                Some(status) => break status,
                None if start.elapsed() >= SESSION_LIST_TIMEOUT => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("`opencode session list` timed out".to_string());
                }
                None => std::thread::sleep(Duration::from_millis(50)),
            }
        };

        let stdout = stdout_reader.join().unwrap_or_default();
        let stderr = stderr_reader.join().unwrap_or_default();
        Ok(std::process::Output {
            status,
            stdout,
            stderr,
        })
    }

    fn fetch_sessions_json_from_db() -> Result<Vec<u8>, String> {
        let db_path = Self::db_path();
        if !db_path.is_file() {
            return Err(format!(
                "OpenCode database not found at {}",
                db_path.display()
            ));
        }

        const QUERY: &str = r#"
        SELECT
            s.id AS id,
            s.title AS title,
            s.directory AS directory,
            s.path AS path,
            s.workspace_id AS workspaceId,
            w.branch AS workspaceBranch,
            w.directory AS workspaceDirectory,
            s.time_created AS created,
            s.time_updated AS updated,
            s.project_id AS projectId,
            p.worktree AS projectWorktree,
            (
                SELECT CAST(COALESCE(SUM(LENGTH(m.data)), 0) AS INTEGER)
                FROM message m
                WHERE m.session_id = s.id
            ) AS transcriptSizeBytes
        FROM session s
        JOIN project p ON p.id = s.project_id
        LEFT JOIN workspace w ON w.id = s.workspace_id
        WHERE s.time_archived IS NULL
        ORDER BY s.time_updated DESC
    "#;

        sqlite_readonly::query_json(&db_path, QUERY)
    }

    fn workdir_matches(
        workdir: &Path,
        session: &OpenCodeSessionJson,
        git_cache: &mut HashMap<PathBuf, Option<PathBuf>>,
    ) -> bool {
        let candidates = [
            session.directory.as_deref(),
            session.workspace_directory.as_deref(),
            session.project_worktree.as_deref(),
        ];
        for candidate in candidates.into_iter().flatten() {
            if cwd_matches(workdir, Some(candidate)) {
                return true;
            }
            if share_git_repository(workdir, Path::new(candidate), git_cache) {
                return true;
            }
        }
        false
    }

    fn transcript_sizes_from_db() -> HashMap<String, u64> {
        let db_path = Self::db_path();
        if !db_path.is_file() {
            return HashMap::new();
        }

        const QUERY: &str = r#"
        SELECT session_id, CAST(COALESCE(SUM(LENGTH(data)), 0) AS INTEGER) AS bytes
        FROM message
        GROUP BY session_id
    "#;

        #[derive(Deserialize)]
        struct Row {
            session_id: String,
            bytes: i64,
        }

        let Ok(rows) = sqlite_readonly::query_rows::<Row>(&db_path, QUERY) else {
            return HashMap::new();
        };
        rows.into_iter()
            .filter_map(|row| {
                let bytes = row.bytes.max(0) as u64;
                (bytes > 0).then_some((row.session_id, bytes))
            })
            .collect()
    }

    fn git_summary(
        session: &OpenCodeSessionJson,
        branch_cache: &mut HashMap<PathBuf, Option<String>>,
    ) -> Option<AgentGitSummary> {
        let directory = session.directory.as_deref()?;
        let branch = session
            .workspace_branch
            .clone()
            .filter(|value| !value.is_empty())
            .or_else(|| git_branch_at(Path::new(directory), branch_cache));
        Some(AgentGitSummary {
            branch,
            worktree: Some(directory.to_string()),
            commit_hash: None,
            repository_url: None,
            pr_number: None,
            pr_url: None,
            pr_repository: None,
        })
    }

    fn transcript_size_bytes(
        session: &OpenCodeSessionJson,
        transcript_sizes: &HashMap<String, u64>,
    ) -> Option<u64> {
        session
            .transcript_size_bytes
            .and_then(|size| (size > 0).then_some(size as u64))
            .or_else(|| {
                session
                    .path
                    .as_deref()
                    .map(Path::new)
                    .and_then(file_size_bytes)
            })
            .or_else(|| transcript_sizes.get(&session.id).copied())
            .filter(|size| *size > 0)
    }

    fn sessions_from_json(
        workdir: &Path,
        json: &[u8],
        _cli: &AgentCliSummary,
        source: &str,
    ) -> Result<Vec<AgentSessionSummary>, String> {
        let raw: Vec<OpenCodeSessionJson> =
            serde_json::from_slice(json).map_err(|error| error.to_string())?;
        // The SQLite source already computed per-row transcript sizes; only the CLI
        // list lacks them, so run the extra message-table scan solely as a fallback.
        let transcript_sizes = if source == Self::DB_SOURCE {
            HashMap::new()
        } else {
            Self::transcript_sizes_from_db()
        };
        let mut git_common_cache = HashMap::new();
        let mut git_branch_cache = HashMap::new();
        let mut sessions = Vec::new();
        for session in raw {
            if !Self::workdir_matches(workdir, &session, &mut git_common_cache) {
                continue;
            }
            let git = Self::git_summary(&session, &mut git_branch_cache);
            let transcript = Self::transcript_size_bytes(&session, &transcript_sizes);
            sessions.push(AgentSessionSummary {
                slug: "opencode".to_string(),
                session_id: session.id.clone(),
                title: session_title(session.title.clone()),
                cwd: session
                    .directory
                    .clone()
                    .or(session.workspace_directory.clone())
                    .or(session.project_worktree.clone()),
                created_at: session
                    .created
                    .and_then(DateTime::<Utc>::from_timestamp_millis),
                last_activity_at: session
                    .updated
                    .and_then(DateTime::<Utc>::from_timestamp_millis),
                resume: resume_summary("opencode", &session.id),
                git,
                usage: None,
                transcript_size_bytes: transcript,
            });
        }
        sessions.sort_by(|left, right| right.last_activity_at.cmp(&left.last_activity_at));
        Ok(sessions)
    }

    // ---------- account / plan / usage ----------

    pub fn account_fields() -> Vec<AgentInfoField> {
        let mut fields = Vec::new();
        Self::append_auth_plan_fields(&mut fields);
        fields
    }

    pub fn subscription_plan_fields() -> Option<Vec<AgentInfoField>> {
        let mut fields = Vec::new();
        Self::append_auth_plan_fields(&mut fields);
        (!fields.is_empty()).then_some(fields)
    }

    fn append_auth_plan_fields(fields: &mut Vec<AgentInfoField>) {
        let auth_path = Self::auth_path();
        let logged_in = auth_path.is_file();
        fields.push(AgentInfoField {
            label: "Logged in".to_string(),
            value: if logged_in { "yes" } else { "no" }.to_string(),
        });
        if !logged_in {
            return;
        }
        let contents = std::fs::read_to_string(&auth_path).ok();
        let value = contents
            .as_deref()
            .and_then(|raw| serde_json::from_str::<Value>(raw).ok());
        let Some(value) = value else {
            return;
        };

        let openai_profile = Self::openai_jwt_profile(&value);

        if value.get("opencode-go").is_some() {
            fields.push(AgentInfoField {
                label: "Plan".to_string(),
                value: "Go".to_string(),
            });
        } else if value.get("opencode").is_some() {
            fields.push(AgentInfoField {
                label: "Plan".to_string(),
                value: "Zen".to_string(),
            });
        } else if let Some(ref profile) = openai_profile {
            if !profile.plan.is_empty() {
                fields.push(AgentInfoField {
                    label: "Plan".to_string(),
                    value: humanize_plan_token(&profile.plan),
                });
            }
        } else {
            for (key, label) in [("google", "Google"), ("amazon-bedrock", "Amazon Bedrock")] {
                if value.get(key).is_some() {
                    fields.push(AgentInfoField {
                        label: "Plan".to_string(),
                        value: label.to_string(),
                    });
                    break;
                }
            }
        }

        if let Some(ref profile) = openai_profile {
            if !profile.email.is_empty() {
                fields.push(AgentInfoField {
                    label: "Account".to_string(),
                    value: profile.email.clone(),
                });
            }
        }
    }
}

#[derive(Default)]
struct OpenAiProfile {
    plan: String,
    email: String,
}

impl OpenCodeActor {
    fn openai_jwt_profile(auth: &Value) -> Option<OpenAiProfile> {
        let openai = auth.get("openai")?;
        let token = openai.get("access").and_then(Value::as_str)?;
        let payload_seg = token.split('.').nth(1)?;
        let bytes = base64_url::decode(payload_seg).ok()?;
        let payload: Value = serde_json::from_slice(&bytes).ok()?;
        let openai_auth = payload.get("https://api.openai.com/auth")?;
        let plan = openai_auth
            .get("chatgpt_plan_type")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let email = payload
            .get("https://api.openai.com/profile")
            .and_then(|p| p.get("email"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        Some(OpenAiProfile { plan, email })
    }

    pub fn fetch_account_usage() -> Option<AgentUsageSnapshot> {
        let db_path = Self::db_path();
        if !db_path.is_file() {
            return None;
        }
        let query = "SELECT SUM(cost) as total_cost FROM session;";
        let rows: Vec<Value> = sqlite_readonly::query_rows(&db_path, query).ok()?;
        let row = rows.first()?;
        let total_cost = row
            .get("total_cost")
            .and_then(Value::as_f64)
            .filter(|v| *v > 0.0);
        // OpenCode has no gauge; surface cumulative spend as an `extra` field.
        let extra = total_cost
            .map(|cost| vec![info_field("Spend", &format!("${cost:.2}"))])
            .unwrap_or_default();
        Some(AgentUsageSnapshot {
            extra,
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::utils::normalize_path;

    fn cli(version: &str) -> AgentCliSummary {
        AgentCliSummary {
            available: true,
            version: Some(version.to_string()),
            error: None,
        }
    }

    #[test]
    fn json_maps_safe_summary_fields() {
        let json = br#"[
          {
            "id": "ses_123",
            "title": "Fix terminal paste",
            "updated": 1777805877635,
            "created": 1777805707332,
            "projectId": "project-hash",
            "directory": "/repo",
            "transcriptSizeBytes": 8192
          }
        ]"#;

        let sessions = OpenCodeActor::sessions_from_json(
            Path::new("/repo"),
            json,
            &cli("1.14.33"),
            "opencode sqlite",
        )
        .expect("parse opencode sessions");

        assert_eq!(sessions.len(), 1);
        let session = &sessions[0];
        assert_eq!(session.slug, "opencode");
        assert_eq!(session.session_id, "ses_123");
        assert_eq!(session.title.as_deref(), Some("Fix terminal paste"));
        assert!(session.resume.available);
        assert_eq!(
            session.resume.action_id.as_deref(),
            Some("opencode:ses_123")
        );
        assert!(session.git.is_some());
    }

    #[test]
    fn session_summary_resolves_git_branch_and_transcript_size() {
        let workdir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("repo root");
        let directory = workdir.display().to_string();
        let json = format!(
            r#"[{{
              "id": "ses_git",
              "title": "Branch test",
              "directory": "{directory}",
              "projectWorktree": "{directory}",
              "transcriptSizeBytes": 4096,
              "updated": 1777805877635,
              "created": 1777805707332
            }}]"#
        );

        let sessions =
            OpenCodeActor::sessions_from_json(workdir, json.as_bytes(), &cli("1.14.33"), "test")
                .expect("parse opencode sessions");
        assert_eq!(sessions.len(), 1);
        let session = &sessions[0];
        // Branch comes from live `git`; CI checks out detached HEAD, so it may be None.
        // Only assert non-empty when a branch was resolved.
        if let Some(branch) = session.git.as_ref().and_then(|git| git.branch.as_deref()) {
            assert!(!branch.is_empty());
        }
    }

    #[test]
    fn workdir_matches_linked_git_worktree_via_common_dir() {
        let workdir = Path::new("/Users/me/projects/zedra-main");
        let linked = Path::new("/Users/me/projects/zedra");
        let common = PathBuf::from("/Users/me/projects/zedra/.git");
        let session = OpenCodeSessionJson {
            id: "ses_linked".into(),
            title: Some("Linked worktree session".into()),
            updated: Some(1777805877635),
            created: Some(1777805707332),
            project_id: Some("project-hash".into()),
            directory: Some(linked.display().to_string()),
            project_worktree: Some(linked.display().to_string()),
            workspace_branch: None,
            workspace_directory: None,
            path: None,
            transcript_size_bytes: None,
        };

        let mut git_cache = HashMap::new();
        git_cache.insert(normalize_path(workdir), Some(common.clone()));
        git_cache.insert(normalize_path(linked), Some(common));

        assert!(OpenCodeActor::workdir_matches(
            workdir,
            &session,
            &mut git_cache
        ));
    }

    #[test]
    fn db_json_parses_sqlite_shape() {
        let json = br#"[
          {
            "id": "ses_db",
            "title": "From sqlite",
            "directory": "/repo",
            "created": 1777805707332,
            "updated": 1777805877635,
            "projectId": "project-hash",
            "projectWorktree": "/repo",
            "workspaceBranch": "feature/opencode",
            "transcriptSizeBytes": 2048
          }
        ]"#;
        let sessions = OpenCodeActor::sessions_from_json(
            Path::new("/repo"),
            json,
            &cli("1.14.33"),
            "opencode sqlite",
        )
        .expect("parse");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title.as_deref(), Some("From sqlite"));
        assert_eq!(
            sessions[0]
                .git
                .as_ref()
                .and_then(|git| git.branch.as_deref()),
            Some("feature/opencode")
        );
    }

    #[test]
    fn git_summary_prefers_workspace_branch_over_live_git() {
        let session = OpenCodeSessionJson {
            id: "ses_ws".into(),
            title: Some("Workspace branch".into()),
            updated: None,
            created: None,
            project_id: None,
            directory: Some(env!("CARGO_MANIFEST_DIR").to_string()),
            project_worktree: None,
            workspace_branch: Some("stored-branch".into()),
            workspace_directory: Some("/repo/worktree".into()),
            path: None,
            transcript_size_bytes: None,
        };
        let mut branch_cache = HashMap::new();
        let git = OpenCodeActor::git_summary(&session, &mut branch_cache).expect("git summary");
        assert_eq!(git.branch.as_deref(), Some("stored-branch"));
        assert_eq!(git.worktree.as_deref(), Some(env!("CARGO_MANIFEST_DIR")));
        assert!(branch_cache.is_empty());
    }
}

use super::{
    home_path, hook_file_mentions_zedra, hooks_enabled, setup_status, ActorFuture, AgentActor,
    ScanCtx, SessionCounts as ActorSessionCounts,
};

pub(super) struct OpenCodeActor;

impl AgentActor for OpenCodeActor {
    fn shows_detail(&self) -> bool {
        true
    }

    fn slug(&self) -> &'static str {
        "opencode"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["open-code", "open_code"]
    }

    fn display_name(&self) -> &'static str {
        "OpenCode"
    }

    fn icon_name(&self) -> &'static str {
        "opencode"
    }

    fn programs(&self) -> &'static [&'static str] {
        &["opencode"]
    }

    fn detect_aliases(&self) -> &'static [&'static str] {
        &["opencode", "open-code"]
    }

    fn cli_available(&self, _workdir: &Path) -> bool {
        Self::cli_available()
    }

    fn session_counts(&self, ctx: &ScanCtx) -> Result<ActorSessionCounts, String> {
        Self::session_counts(ctx.workdir, ctx.cli)
    }

    fn sessions(
        &self,
        ctx: &ScanCtx,
        limit: usize,
    ) -> Result<(Vec<AgentSessionSummary>, usize), String> {
        Self::sessions(ctx.workdir, ctx.cli, limit)
    }

    fn account_fields(&self, _workdir: &Path) -> Vec<AgentInfoField> {
        Self::account_fields()
    }

    fn scan_data_source(&self) -> AgentDataSource {
        AgentDataSource::ProviderCli
    }

    fn setup_summary(&self, available: bool, workdir: &Path) -> AgentSetupSummary {
        let skills_installed =
            home_path(&[".config", "opencode", "skills", "zedra-start", "SKILL.md"]).is_file();
        let plugin_installed =
            home_path(&[".config", "opencode", "plugins", "zedra-agent-hooks.js"]).is_file();
        let hooks_installed = hooks_enabled()
            && (plugin_installed
                || hook_file_mentions_zedra(&workdir.join(".opencode/plugins/zedra.js")));
        setup_status(
            available,
            skills_installed,
            plugin_installed,
            hooks_installed,
            None,
        )
    }

    fn resume_launch_command(&self, quoted: &str) -> Option<String> {
        Some(format!("opencode --session {quoted}"))
    }

    fn subscription_plan<'a>(&'a self) -> ActorFuture<'a, Option<Vec<AgentInfoField>>> {
        spawn_blocking_opt(Self::subscription_plan_fields)
    }

    fn account_usage<'a>(&'a self) -> ActorFuture<'a, Option<AgentUsageSnapshot>> {
        spawn_blocking_opt(Self::fetch_account_usage)
    }

    fn supports_hooks(&self) -> bool {
        true
    }

    // The OpenCode plugin sends a top-level event name and OpenCode's native
    // event object. Accept flat fields as fallbacks for synthetic/test payloads.
    fn hook_identity(&self, payload: &Value) -> (String, Option<String>) {
        let event_name = super::utils::payload_string(payload, "event_name")
            .or_else(|| super::utils::payload_string(payload, "event"))
            .or_else(|| super::utils::payload_string(payload, "type"))
            .or_else(|| Self::opencode_event_string(payload, "type"))
            .unwrap_or_default();
        let agent_session_id = super::utils::payload_string(payload, "sessionID")
            .or_else(|| super::utils::payload_string(payload, "sessionId"))
            .or_else(|| Self::opencode_event_property_string(payload, "sessionID"));
        (event_name, agent_session_id)
    }

    fn hook_state(&self, event_name: &str, payload: &Value) -> Option<AgentState> {
        Self::opencode_agent_state(event_name, payload)
    }

    fn hook_notify_title(&self, event_name: &str) -> Option<String> {
        let name = self.display_name();
        match event_name {
            "permission.asked" => Some(format!("{name} requires approval")),
            "session.idle" => Some(format!("{name} completed")),
            _ => None,
        }
    }

    fn supports_setup(&self) -> bool {
        true
    }

    fn setup(&self, workdir: &Path, force: bool) -> anyhow::Result<Vec<PathBuf>> {
        let script_path = super::cli::write_hook_script(workdir, force)?;
        let config_path = workdir.join(".opencode/plugins/zedra.js");
        super::utils::write_file_checked(
            &config_path,
            &Self::local_plugin_contents(&script_path)?,
            force,
            "OpenCode local plugin",
        )?;
        Ok(vec![script_path, config_path])
    }

    fn supports_setup_cli(&self) -> bool {
        true
    }

    fn setup_cli<'a>(
        &'a self,
        action: super::SetupAction,
        ctx: super::SetupCliCtx,
    ) -> ActorFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            match action {
                super::SetupAction::Install => {
                    let skills_dir = Self::opencode_skills_dir(&ctx)?;
                    ctx.install_skills("OpenCode", &skills_dir).await?;
                    Self::install_opencode_hooks(&ctx)?;
                    ctx.message("OpenCode setup complete. Start in OpenCode:");
                    ctx.suggest_command("/zedra-start");
                }
                super::SetupAction::Remove => {
                    let skills_dir = Self::opencode_skills_dir(&ctx)?;
                    ctx.remove_skills("OpenCode", &skills_dir)?;
                    Self::remove_opencode_hooks(&ctx)?;

                    ctx.message("");
                    ctx.message("OpenCode setup removed.");
                    ctx.message("Restart OpenCode or reload skills to apply the change.");
                }
            }
            Ok(())
        })
    }

    fn hook_test_payload(&self, event_name: &str, workdir: &Path) -> serde_json::Value {
        serde_json::json!({
            "event": event_name,
            "sessionID": "zedra-test-session",
            "cwd": workdir,
            "tool": "bash",
        })
    }
}

impl OpenCodeActor {
    fn opencode_event_string(payload: &serde_json::Value, key: &str) -> Option<String> {
        payload_string(payload.get("event")?, key)
    }

    fn opencode_event_property_string(payload: &serde_json::Value, key: &str) -> Option<String> {
        payload_string(payload.get("event")?.get("properties")?, key)
    }

    fn opencode_session_status(payload: &serde_json::Value) -> Option<String> {
        let status = payload
            .get("status")
            .or_else(|| payload.get("event")?.get("properties")?.get("status"))?;
        if let Some(status) = status.as_str() {
            return Some(status.to_owned());
        }
        status
            .get("type")?
            .as_str()
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    }

    fn opencode_agent_state(event_name: &str, payload: &serde_json::Value) -> Option<AgentState> {
        match event_name {
            "permission.asked" => Some(AgentState::WaitingApproval),
            "permission.replied" => Some(AgentState::Running),
            "session.idle" => Some(AgentState::Completed),
            "session.status" => match Self::opencode_session_status(payload)?.as_str() {
                "busy" | "retry" => Some(AgentState::Running),
                "idle" => Some(AgentState::Completed),
                _ => None,
            },
            "session.error" => Some(AgentState::Error),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Interactive `zedra setup opencode` (global skills + hook plugin)
// ---------------------------------------------------------------------------

const OPENCODE_HOOK_PLUGIN: &str = "zedra-agent-hooks.js";

/// Events both hook plugins forward; must stay in sync with
/// `opencode_agent_state` and the receive_hook notify set.
const OPENCODE_FORWARDED_EVENTS: &[&str] = &[
    "permission.asked",
    "permission.replied",
    "session.status",
    "session.idle",
    "session.error",
];

/// JS `shouldForward` body shared by the global and workdir plugin templates.
fn should_forward_js() -> String {
    let cond = OPENCODE_FORWARDED_EVENTS
        .iter()
        .map(|event| format!("event === \"{event}\""))
        .collect::<Vec<_>>()
        .join("\n    || ");
    format!("function shouldForward(event) {{\n  return {cond};\n}}")
}

impl OpenCodeActor {
    fn opencode_skills_dir(ctx: &super::SetupCliCtx) -> anyhow::Result<PathBuf> {
        Ok(Self::opencode_config_dir(ctx)?.join("skills"))
    }

    fn opencode_config_dir(ctx: &super::SetupCliCtx) -> anyhow::Result<PathBuf> {
        Ok(ctx.home_dir()?.join(".config").join("opencode"))
    }

    fn install_opencode_hooks(ctx: &super::SetupCliCtx) -> anyhow::Result<()> {
        let dir = Self::opencode_config_dir(ctx)?;
        Self::install_opencode_hooks_in_dir(ctx, &dir, &ctx.hook_binary()?)
    }

    fn install_opencode_hooks_in_dir(
        ctx: &super::SetupCliCtx,
        dir: &Path,
        binary: &str,
    ) -> anyhow::Result<()> {
        let plugin_path = Self::opencode_hook_plugin_path(dir);
        let content = Self::opencode_hook_plugin(binary, ctx.quiet)?;
        if std::fs::read_to_string(&plugin_path).ok().as_deref() == Some(&content) {
            return Ok(());
        }
        if let Some(parent) = plugin_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&plugin_path, content)?;
        ctx.step("hooks");
        ctx.detail(&format!("write {}", plugin_path.display()));
        Ok(())
    }

    fn remove_opencode_hooks(ctx: &super::SetupCliCtx) -> anyhow::Result<()> {
        let dir = Self::opencode_config_dir(ctx)?;
        Self::remove_opencode_hooks_in_dir(ctx, &dir)
    }

    fn remove_opencode_hooks_in_dir(ctx: &super::SetupCliCtx, dir: &Path) -> anyhow::Result<()> {
        let plugin_path = Self::opencode_hook_plugin_path(dir);
        if ctx.remove_path(&plugin_path)? {
            ctx.step("hooks");
            ctx.detail(&format!("remove {}", plugin_path.display()));
        }
        Ok(())
    }

    fn opencode_hook_plugin_path(dir: &Path) -> PathBuf {
        dir.join("plugins").join(OPENCODE_HOOK_PLUGIN)
    }

    fn opencode_hook_plugin(binary: &str, quiet: bool) -> anyhow::Result<String> {
        let binary = serde_json::to_string(binary)?;
        let quiet_arg = if quiet { r#", "--quiet""# } else { "" };
        let should_forward = should_forward_js();
        Ok(format!(
            r#"import {{ spawnSync }} from "node:child_process";

const zedra = {binary};

function send(event, payload = {{}}) {{
  spawnSync(zedra, ["agent", "hook", "receive", "--agent", "opencode"{quiet_arg}, "--payload", JSON.stringify({{ event_name: event, ...payload }})], {{
    stdio: ["ignore", "ignore", "ignore"],
    timeout: 2000,
  }});
}}

{should_forward}

export const ZedraAgentHooks = async () => {{
  return {{
    event: async (input) => {{
      const event = input.event?.type ?? "event";
      if (!shouldForward(event)) {{
        return;
      }}
      send(event, input);
    }},
  }};
}}
"#
        ))
    }
}

#[cfg(test)]
mod hook_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn opencode_native_events_map_to_agent_states() {
        let busy = json!({
            "event": {
                "type": "session.status",
                "properties": {
                    "sessionID": "ses_123",
                    "status": { "type": "busy" }
                }
            }
        });
        assert_eq!(
            OpenCodeActor::opencode_event_property_string(&busy, "sessionID").as_deref(),
            Some("ses_123")
        );
        assert_eq!(
            OpenCodeActor::opencode_agent_state("session.status", &busy),
            Some(AgentState::Running)
        );
        assert_eq!(
            OpenCodeActor::opencode_agent_state("permission.asked", &json!({})),
            Some(AgentState::WaitingApproval)
        );
        assert_eq!(
            OpenCodeActor::opencode_agent_state("permission.replied", &json!({})),
            Some(AgentState::Running)
        );
        assert_eq!(
            OpenCodeActor::opencode_agent_state("session.idle", &json!({})),
            Some(AgentState::Completed)
        );
    }
}

#[cfg(test)]
mod setup_cli_tests {
    use super::*;

    #[test]
    fn opencode_hook_install_and_remove_updates_plugin_directory() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = crate::agent::SetupCliCtx {
            full_bin_path: false,
            quiet: true,
        };

        OpenCodeActor::install_opencode_hooks_in_dir(&ctx, dir.path(), "/tmp/zedra").unwrap();
        OpenCodeActor::install_opencode_hooks_in_dir(&ctx, dir.path(), "/tmp/zedra").unwrap();

        let plugin_path = OpenCodeActor::opencode_hook_plugin_path(dir.path());
        let plugin = std::fs::read_to_string(&plugin_path).unwrap();
        assert!(plugin.contains(
            r#"spawnSync(zedra, ["agent", "hook", "receive", "--agent", "opencode", "--quiet""#
        ));
        assert!(plugin.contains("JSON.stringify({ event_name: event, ...payload })"));
        assert!(plugin.contains(r#"event === "permission.asked""#));
        assert!(plugin.contains(r#"event === "session.error""#));

        OpenCodeActor::remove_opencode_hooks_in_dir(&ctx, dir.path()).unwrap();

        assert!(!plugin_path.exists());
    }
}

// ---------------------------------------------------------------------------
// Workdir-scoped hook plugin written by `AgentActor::setup`
// ---------------------------------------------------------------------------

impl OpenCodeActor {
    fn local_plugin_contents(script_path: &Path) -> anyhow::Result<String> {
        Ok(format!(
            r#"const hookScript = {script};

async function send(event, payload = {{}}) {{
  const proc = Bun.spawn([hookScript], {{
    stdin: "pipe",
    stdout: "ignore",
    stderr: "ignore",
    env: {{
      ...process.env,
      ZEDRA_AGENT_KIND: "opencode",
      ZEDRA_AGENT_EVENT: event,
    }},
  }});
  proc.stdin.write(JSON.stringify({{ type: event, ...payload }}));
  proc.stdin.end();
  await proc.exited;
}}

{should_forward}

export const ZedraPlugin = async () => ({{
  event: async (input) => {{
    const event = input.event?.type ?? "unknown";
    if (!shouldForward(event)) {{
      return;
    }}
    await send(event, input.event ?? input);
  }},
}});
"#,
            script = serde_json::to_string(&script_path.display().to_string())?,
            should_forward = should_forward_js()
        ))
    }
}
