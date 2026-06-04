use crate::agent_utils::*;
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
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub workspace_branch: Option<String>,
    #[serde(default)]
    pub workspace_directory: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub transcript_size_bytes: Option<i64>,
}

pub struct SessionCounts {
    pub total: usize,
    pub resumable: usize,
    pub latest_session_id: Option<String>,
    pub latest_session_title: Option<String>,
    pub last_activity_at: Option<DateTime<Utc>>,
    pub provider_project_id: Option<String>,
}

struct SessionCountSummary {
    total: usize,
    latest: Option<OpenCodeSessionJson>,
}

pub fn cli_available() -> bool {
    db_path().is_file() || command_on_path("opencode")
}

pub fn normalize_event(event_name: &str) -> Option<(AgentEventKind, AgentLifecycleStatus)> {
    Some(match event_name {
        "session.status" => (
            AgentEventKind::SessionUpdated,
            AgentLifecycleStatus::Running,
        ),
        "session.idle" => (AgentEventKind::SessionUpdated, AgentLifecycleStatus::Idle),
        "session.error" => (AgentEventKind::TurnFailed, AgentLifecycleStatus::Failed),
        "permission.asked" => (
            AgentEventKind::PermissionRequested,
            AgentLifecycleStatus::WaitingForPermission,
        ),
        "permission.replied" => (
            AgentEventKind::PermissionResolved,
            AgentLifecycleStatus::Running,
        ),
        "tool.execute.before" => (AgentEventKind::ToolStarted, AgentLifecycleStatus::Running),
        "tool.execute.after" => (AgentEventKind::ToolCompleted, AgentLifecycleStatus::Running),
        name if name.starts_with("tool.") && name.ends_with(".error") => {
            (AgentEventKind::ToolFailed, AgentLifecycleStatus::Failed)
        }
        _ => return None,
    })
}

pub fn session_counts(workdir: &Path, _cli: &AgentCliSummary) -> Result<SessionCounts, String> {
    if !cli_available() {
        return Ok(SessionCounts {
            total: 0,
            resumable: 0,
            latest_session_id: None,
            latest_session_title: None,
            last_activity_at: None,
            provider_project_id: None,
        });
    }
    let summary = session_count_summary(workdir)?;
    Ok(SessionCounts {
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
    let (json, _) = fetch_sessions_json()?;
    let raw: Vec<OpenCodeSessionJson> =
        serde_json::from_slice(&json).map_err(|error| error.to_string())?;
    let mut git_cache = HashMap::new();
    let mut matched = raw
        .into_iter()
        .filter(|session| workdir_matches(workdir, session, &mut git_cache))
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
    let (json, source) = fetch_sessions_json()?;
    let mut sessions = sessions_from_json(workdir, &json, cli, source)?;
    let total = sessions.len();
    sessions.truncate(limit);
    Ok((sessions, total))
}

pub fn session_cli_summary() -> AgentCliSummary {
    if cli_available() {
        AgentCliSummary {
            available: true,
            version: None,
            error: None,
        }
    } else {
        AgentCliSummary {
            available: false,
            version: None,
            error: Some("OpenCode CLI and database not found".to_string()),
        }
    }
}

fn db_path() -> PathBuf {
    home_path(&[".local", "share", "opencode", "opencode.db"])
}

fn auth_path() -> PathBuf {
    home_path(&[".local", "share", "opencode", "auth.json"])
}

fn fetch_sessions_json() -> Result<(Vec<u8>, &'static str), String> {
    if let Ok(json) = fetch_sessions_json_from_db() {
        return Ok((json, "opencode sqlite"));
    }

    let output = Command::new("opencode")
        .args(["session", "list", "--format", "json", "--pure"])
        .output()
        .map_err(|error| error.to_string())?;
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

    fetch_sessions_json_from_db()
        .map(|json| (json, "opencode sqlite"))
        .map_err(|fallback_error| format!("{cli_error}; sqlite fallback failed: {fallback_error}"))
}

fn fetch_sessions_json_from_db() -> Result<Vec<u8>, String> {
    let db_path = db_path();
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
    let db_path = db_path();
    if !db_path.is_file() {
        return HashMap::new();
    }

    const QUERY: &str = r#"
        SELECT session_id, CAST(COALESCE(SUM(LENGTH(data)), 0) AS INTEGER) AS bytes
        FROM message
        GROUP BY session_id
    "#;

    let stdout = match sqlite_readonly::query_json(&db_path, QUERY) {
        Ok(stdout) => stdout,
        Err(_) => return HashMap::new(),
    };
    if stdout.is_empty() {
        return HashMap::new();
    }

    #[derive(Deserialize)]
    struct Row {
        session_id: String,
        bytes: i64,
    }

    serde_json::from_slice::<Vec<Row>>(&stdout)
        .map(|rows| {
            rows.into_iter()
                .filter_map(|row| {
                    let bytes = row.bytes.max(0) as u64;
                    (bytes > 0).then_some((row.session_id, bytes))
                })
                .collect()
        })
        .unwrap_or_default()
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
    _source: &str,
) -> Result<Vec<AgentSessionSummary>, String> {
    let raw: Vec<OpenCodeSessionJson> =
        serde_json::from_slice(json).map_err(|error| error.to_string())?;
    let transcript_sizes = transcript_sizes_from_db();
    let mut git_common_cache = HashMap::new();
    let mut git_branch_cache = HashMap::new();
    let mut sessions = Vec::new();
    for session in raw {
        if !workdir_matches(workdir, &session, &mut git_common_cache) {
            continue;
        }
        let git = git_summary(&session, &mut git_branch_cache);
        let transcript = transcript_size_bytes(&session, &transcript_sizes);
        sessions.push(AgentSessionSummary {
            kind: AgentKind::OpenCode,
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
            resume: resume_summary(AgentKind::OpenCode, &session.id),
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
    append_auth_plan_fields(&mut fields);
    fields
}

pub fn subscription_plan_fields() -> Option<Vec<AgentInfoField>> {
    let mut fields = Vec::new();
    append_auth_plan_fields(&mut fields);
    (!fields.is_empty()).then_some(fields)
}

fn append_auth_plan_fields(fields: &mut Vec<AgentInfoField>) {
    let auth_path = auth_path();
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

    let openai_profile = openai_jwt_profile(&value);

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

#[derive(Default)]
struct OpenAiProfile {
    plan: String,
    email: String,
}

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
    let db_path = db_path();
    if !db_path.is_file() {
        return None;
    }
    let query = "SELECT SUM(cost) as total_cost, SUM(summary_additions) as lines_added, SUM(summary_deletions) as lines_removed FROM session;";
    let bytes = sqlite_readonly::query_json(&db_path, query).ok()?;
    let rows: Vec<Value> = serde_json::from_slice(&bytes).ok()?;
    let row = rows.first()?;
    let total_cost = row
        .get("total_cost")
        .and_then(Value::as_f64)
        .filter(|v| *v > 0.0);
    let lines_added = row.get("lines_added").and_then(Value::as_i64);
    let lines_removed = row.get("lines_removed").and_then(Value::as_i64);
    Some(AgentUsageSnapshot {
        context_used_percent: None,
        total_cost_usd: total_cost,
        total_duration_ms: None,
        total_api_duration_ms: None,
        lines_added,
        lines_removed,
        rate_limit_five_hour_used_percent: None,
        rate_limit_seven_day_used_percent: None,
        rate_limit_five_hour_resets_at: None,
        rate_limit_seven_day_resets_at: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_utils::normalize_path;

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

        let sessions =
            sessions_from_json(Path::new("/repo"), json, &cli("1.14.33"), "opencode sqlite")
                .expect("parse opencode sessions");

        assert_eq!(sessions.len(), 1);
        let session = &sessions[0];
        assert_eq!(session.kind, AgentKind::OpenCode);
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

        let sessions = sessions_from_json(workdir, json.as_bytes(), &cli("1.14.33"), "test")
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
            workspace_id: None,
            workspace_branch: None,
            workspace_directory: None,
            path: None,
            transcript_size_bytes: None,
        };

        let mut git_cache = HashMap::new();
        git_cache.insert(normalize_path(workdir), Some(common.clone()));
        git_cache.insert(normalize_path(linked), Some(common));

        assert!(workdir_matches(workdir, &session, &mut git_cache));
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
        let sessions =
            sessions_from_json(Path::new("/repo"), json, &cli("1.14.33"), "opencode sqlite")
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
            workspace_id: Some("ws_1".into()),
            workspace_branch: Some("stored-branch".into()),
            workspace_directory: Some("/repo/worktree".into()),
            path: None,
            transcript_size_bytes: None,
        };
        let mut branch_cache = HashMap::new();
        let git = git_summary(&session, &mut branch_cache).expect("git summary");
        assert_eq!(git.branch.as_deref(), Some("stored-branch"));
        assert_eq!(git.worktree.as_deref(), Some(env!("CARGO_MANIFEST_DIR")));
        assert!(branch_cache.is_empty());
    }
}
