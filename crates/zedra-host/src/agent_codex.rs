use crate::agent_utils::*;
use crate::sqlite_readonly;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use zedra_rpc::proto::*;

#[derive(Debug, Deserialize)]
pub struct CodexThreadRow {
    pub id: String,
    pub cwd: String,
    pub title: String,
    pub rollout_path: String,
    pub source: String,
    pub model_provider: String,
    #[serde(default)]
    pub cli_version: String,
    pub created_at_ms: Option<i64>,
    pub updated_at_ms: Option<i64>,
    #[serde(default)]
    pub first_user_message: String,
    #[serde(default)]
    pub preview: String,
    pub agent_nickname: Option<String>,
    pub agent_role: Option<String>,
    pub git_branch: Option<String>,
    pub git_sha: Option<String>,
    pub git_origin_url: Option<String>,
    #[serde(default)]
    pub approval_mode: String,
    pub model: Option<String>,
}

pub struct SessionCounts {
    pub total: usize,
    pub resumable: usize,
    pub latest_session_id: Option<String>,
    pub latest_session_title: Option<String>,
    pub last_activity_at: Option<DateTime<Utc>>,
}

pub fn cli_available() -> bool {
    command_on_path("codex") || state_db_path().is_some()
}

pub fn normalize_event(event_name: &str) -> Option<(AgentEventKind, AgentLifecycleStatus)> {
    Some(match event_name {
        "SessionStart" => (
            AgentEventKind::SessionStarted,
            AgentLifecycleStatus::Starting,
        ),
        "PermissionRequest" => (
            AgentEventKind::PermissionRequested,
            AgentLifecycleStatus::WaitingForPermission,
        ),
        "PostToolUse" => (AgentEventKind::ToolCompleted, AgentLifecycleStatus::Running),
        "Stop" => (
            AgentEventKind::TurnCompleted,
            AgentLifecycleStatus::Completed,
        ),
        name if name.contains("Failure") || name.contains("Failed") => {
            (AgentEventKind::TurnFailed, AgentLifecycleStatus::Failed)
        }
        _ => return None,
    })
}

pub fn session_counts(workdir: &Path) -> Result<SessionCounts, String> {
    let threads = threads_for_workdir(workdir)?;
    let latest = threads.first();
    Ok(SessionCounts {
        total: threads.len(),
        resumable: threads.len(),
        latest_session_id: latest.map(|thread| thread.id.clone()),
        latest_session_title: latest.and_then(title_from_thread),
        last_activity_at: latest.and_then(thread_updated_at),
    })
}

pub fn sessions(
    workdir: &Path,
    cli: &AgentCliSummary,
    limit: usize,
) -> Result<(Vec<AgentSessionSummary>, usize), String> {
    let threads = threads_for_workdir(workdir)?;
    let total = threads.len();
    let summaries = threads
        .into_iter()
        .take(limit)
        .map(|thread| session_summary_from_thread(&thread, cli))
        .collect();
    Ok((summaries, total))
}

fn state_db_path() -> Option<PathBuf> {
    let dir = home_path(&[".codex"]);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return None;
    };
    let mut best: Option<(u64, PathBuf)> = None;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("state_") || !name.ends_with(".sqlite") {
            continue;
        }
        let Some(version) = name
            .strip_prefix("state_")
            .and_then(|suffix| suffix.strip_suffix(".sqlite"))
            .and_then(|version| version.parse::<u64>().ok())
        else {
            continue;
        };
        match best {
            Some((current, _)) if current >= version => {}
            _ => best = Some((version, entry.path())),
        }
    }
    best.map(|(_, path)| path)
}

fn fetch_threads_from_db(workdir: &Path) -> Result<Vec<CodexThreadRow>, String> {
    let db_path = state_db_path().ok_or_else(|| "Codex state database not found".to_string())?;
    let cwd_keys = workdir_keys(workdir);
    let cwd_filter = cwd_keys
        .iter()
        .map(|cwd| sql_string_literal(cwd))
        .collect::<Vec<_>>()
        .join(", ");
    let query = format!(
        r#"
        SELECT
            id,
            cwd,
            title,
            rollout_path,
            source,
            model_provider,
            cli_version,
            created_at_ms,
            updated_at_ms,
            first_user_message,
            preview,
            agent_nickname,
            agent_role,
            git_branch,
            git_sha,
            git_origin_url,
            approval_mode,
            model
        FROM threads
        WHERE archived = 0 AND cwd IN ({cwd_filter})
        ORDER BY updated_at_ms DESC
    "#
    );
    let stdout = sqlite_readonly::query_json(&db_path, &query)?;
    if stdout.is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_slice(&stdout).map_err(|error| error.to_string())
}

pub fn threads_for_workdir(workdir: &Path) -> Result<Vec<CodexThreadRow>, String> {
    fetch_threads_from_db(workdir)
}

/// Look up the session title for a Codex thread id within a workdir.
/// Returns `None` if the thread is not found or has no title yet.
pub fn title_for_session(workdir: &Path, session_id: &str) -> Option<String> {
    threads_for_workdir(workdir)
        .ok()?
        .into_iter()
        .find(|t| t.id == session_id)
        .and_then(|t| title_from_thread(&t))
}

fn workdir_keys(workdir: &Path) -> Vec<String> {
    let canonical = normalize_path(workdir).to_string_lossy().into_owned();
    let raw = workdir.to_string_lossy().trim_end_matches('/').to_string();
    if raw == canonical {
        vec![canonical]
    } else {
        vec![canonical, raw]
    }
}

pub fn thread_updated_at(thread: &CodexThreadRow) -> Option<DateTime<Utc>> {
    thread
        .updated_at_ms
        .and_then(DateTime::<Utc>::from_timestamp_millis)
        .or_else(|| {
            thread
                .created_at_ms
                .and_then(DateTime::<Utc>::from_timestamp_millis)
        })
}

pub fn title_from_thread(thread: &CodexThreadRow) -> Option<String> {
    sanitize_title_field(&thread.title)
        .or_else(|| sanitize_title_field(&thread.preview))
        .or_else(|| sanitize_prompt_fallback(&thread.first_user_message))
        .or_else(|| {
            title_from_agent_identity(
                thread.agent_nickname.as_deref(),
                thread.agent_role.as_deref(),
            )
        })
}

fn sanitize_title_field(raw: &str) -> Option<String> {
    let mut line = raw.lines().next()?.trim();
    if line.is_empty() {
        return None;
    }
    if let Some(rest) = line.strip_prefix("continue ") {
        let rest = rest.trim();
        if rest.starts_with('/') || rest.starts_with('~') {
            line = title_from_path(rest).unwrap_or(rest);
        }
    } else if line.starts_with('/') || line.starts_with('~') {
        line = title_from_path(line).unwrap_or(line);
    }
    finalize_title(line)
}

fn sanitize_prompt_fallback(raw: &str) -> Option<String> {
    let mut line = raw.lines().next()?.trim();
    if line.is_empty() {
        return None;
    }
    if let Some(rest) = line.strip_prefix("CWD:") {
        line = rest.trim();
        if let Some((_, after_path)) = line.split_once(". ") {
            line = after_path.trim();
        }
    }
    sanitize_title_field(line)
}

fn finalize_title(line: &str) -> Option<String> {
    if line.is_empty() {
        return None;
    }
    // Collapse whitespace; length is clamped centrally in `session_title`.
    let collapsed = line.split_whitespace().collect::<Vec<_>>().join(" ");
    Some(collapsed)
}

fn title_from_path(path: &str) -> Option<&str> {
    Path::new(path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
}

fn title_from_agent_identity(nickname: Option<&str>, role: Option<&str>) -> Option<String> {
    let nickname = nickname?.trim();
    if nickname.is_empty() {
        return None;
    }
    let title = match role.map(str::trim).filter(|role| !role.is_empty()) {
        Some(role) => format!("{nickname} ({role})"),
        None => nickname.to_string(),
    };
    Some(title)
}

fn session_summary_from_thread(
    thread: &CodexThreadRow,
    _cli: &AgentCliSummary,
) -> AgentSessionSummary {
    let rollout_path = std::path::PathBuf::from(&thread.rollout_path);
    AgentSessionSummary {
        kind: AgentKind::Codex,
        session_id: thread.id.clone(),
        title: session_title(title_from_thread(thread)),
        cwd: Some(thread.cwd.clone()),
        created_at: thread
            .created_at_ms
            .and_then(DateTime::<Utc>::from_timestamp_millis),
        last_activity_at: thread_updated_at(thread),
        resume: resume_summary(AgentKind::Codex, &thread.id),
        git: Some(AgentGitSummary {
            branch: thread.git_branch.clone(),
            worktree: None,
            commit_hash: thread.git_sha.clone(),
            repository_url: thread.git_origin_url.clone(),
            pr_number: None,
            pr_url: None,
            pr_repository: None,
        }),
        usage: None,
        transcript_size_bytes: file_size_bytes(&rollout_path),
    }
}

// ---------- account / plan / usage ----------

pub fn account_fields() -> Vec<AgentInfoField> {
    let mut fields = Vec::new();
    append_auth_plan_fields(&mut fields);
    let config_path = home_path(&[".codex", "config.toml"]);
    if let Ok(contents) = std::fs::read_to_string(&config_path) {
        for line in contents.lines() {
            let line = line.trim();
            if line.starts_with("model ") {
                fields.push(AgentInfoField {
                    label: "Model".to_string(),
                    value: toml_value(line),
                });
            } else if line.starts_with("personality ") {
                fields.push(AgentInfoField {
                    label: "Personality".to_string(),
                    value: toml_value(line),
                });
            } else if line.starts_with("model_reasoning_effort ") {
                fields.push(AgentInfoField {
                    label: "Reasoning effort".to_string(),
                    value: toml_value(line),
                });
            }
        }
    }
    if let Some(counts) = thread_counts() {
        fields.push(AgentInfoField {
            label: "Week threads".to_string(),
            value: counts.week.to_string(),
        });
        fields.push(AgentInfoField {
            label: "Total threads".to_string(),
            value: counts.total.to_string(),
        });
    }
    fields
}

pub fn subscription_plan_fields() -> Option<Vec<AgentInfoField>> {
    let mut fields = Vec::new();
    append_auth_plan_fields(&mut fields);
    (!fields.is_empty()).then_some(fields)
}

fn append_auth_plan_fields(fields: &mut Vec<AgentInfoField>) {
    let auth_path = home_path(&[".codex", "auth.json"]);
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
    let Some(profile) = value.as_ref().and_then(jwt_profile) else {
        return;
    };
    if !profile.name.is_empty() {
        fields.push(AgentInfoField {
            label: "Account".to_string(),
            value: profile.name.clone(),
        });
    }
    if !profile.plan.is_empty() {
        fields.push(AgentInfoField {
            label: "Plan".to_string(),
            value: profile.plan.clone(),
        });
    }
    if !profile.plan_until.is_empty() {
        fields.push(AgentInfoField {
            label: "Plan until".to_string(),
            value: profile.plan_until.clone(),
        });
    }
}

pub struct CodexProfile {
    pub name: String,
    pub plan: String,
    pub plan_until: String,
}

pub fn jwt_profile(auth: &Value) -> Option<CodexProfile> {
    let token = auth
        .get("tokens")
        .and_then(|t| t.get("id_token"))
        .and_then(Value::as_str)?;
    let payload_seg = token.split('.').nth(1)?;
    let bytes = base64_url::decode(payload_seg).ok()?;
    let payload: Value = serde_json::from_slice(&bytes).ok()?;
    let openai = payload.get("https://api.openai.com/auth")?;
    let name = payload
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let plan = openai
        .get("chatgpt_plan_type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let plan_until = openai
        .get("chatgpt_subscription_active_until")
        .and_then(Value::as_str)
        .map(|s| s.get(..10).unwrap_or(s).to_string())
        .unwrap_or_default();
    Some(CodexProfile {
        name,
        plan,
        plan_until,
    })
}

struct ThreadCounts {
    week: u64,
    total: u64,
}

fn thread_counts() -> Option<ThreadCounts> {
    let db_path = home_path(&[".codex", "state_5.sqlite"]);
    if !db_path.is_file() {
        return None;
    }
    let week_start = (Utc::now() - chrono::Duration::days(7))
        .format("%Y-%m-%d")
        .to_string();
    let week_ts = chrono::NaiveDate::parse_from_str(&week_start, "%Y-%m-%d")
        .ok()?
        .and_hms_opt(0, 0, 0)?
        .and_utc()
        .timestamp();
    let query = format!(
        "SELECT \
            (SELECT COUNT(*) FROM threads) AS total, \
            (SELECT COUNT(*) FROM threads WHERE created_at/1000 >= {week_ts}) AS week;"
    );
    let bytes = sqlite_readonly::query_json(&db_path, &query).ok()?;
    let rows: Vec<Value> = serde_json::from_slice(&bytes).ok()?;
    let row = rows.first()?;
    let total = row.get("total").and_then(Value::as_u64).unwrap_or(0);
    let week = row.get("week").and_then(Value::as_u64).unwrap_or(0);
    Some(ThreadCounts { week, total })
}

pub async fn fetch_account_usage() -> Option<AgentUsageSnapshot> {
    let auth_path = home_path(&[".codex", "auth.json"]);
    let contents = std::fs::read_to_string(&auth_path).ok()?;
    let auth: Value = serde_json::from_str(&contents).ok()?;
    let access_token = auth
        .get("tokens")
        .and_then(|t| t.get("access_token"))
        .and_then(Value::as_str)?
        .to_string();
    let account_id = auth
        .get("account_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .ok()?;
    let mut req = client
        .get("https://chatgpt.com/backend-api/wham/usage")
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Accept", "application/json")
        .header("User-Agent", "Zedra");
    if let Some(ref id) = account_id {
        req = req.header("ChatGPT-Account-Id", id.as_str());
    }
    let resp = req.send().await.ok()?;
    if !resp.status().is_success() {
        tracing::debug!("codex usage API returned {}", resp.status());
        return None;
    }
    let body: Value = resp.json().await.ok()?;
    let rl = body.get("rate_limit");
    let primary = rl.and_then(|r| r.get("primary_window"));
    let secondary = rl.and_then(|r| r.get("secondary_window"));
    let five_hour = primary
        .and_then(|w| w.get("used_percent"))
        .and_then(Value::as_f64)
        .map(|v| v as f32);
    let seven_day = secondary
        .and_then(|w| w.get("used_percent"))
        .and_then(Value::as_f64)
        .map(|v| v as f32);
    let five_hour_resets_at = primary.and_then(parse_usage_window_resets_at);
    let seven_day_resets_at = secondary.and_then(parse_usage_window_resets_at);
    Some(AgentUsageSnapshot {
        context_used_percent: None,
        total_cost_usd: None,
        total_duration_ms: None,
        total_api_duration_ms: None,
        lines_added: None,
        lines_removed: None,
        rate_limit_five_hour_used_percent: five_hour,
        rate_limit_seven_day_used_percent: seven_day,
        rate_limit_five_hour_resets_at: five_hour_resets_at,
        rate_limit_seven_day_resets_at: seven_day_resets_at,
    })
}

// silence unused Command import on non-test builds
#[allow(dead_code)]
fn _keep_command(_: Command) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_utils::paths_equal;

    fn cli(version: &str) -> AgentCliSummary {
        AgentCliSummary {
            available: true,
            version: Some(version.to_string()),
            error: None,
        }
    }

    #[test]
    fn jwt_profile_extracts_plan_fields() {
        let header = base64_url::encode(r#"{"alg":"none"}"#);
        let payload = base64_url::encode(
            r#"{
              "name":"Ada",
              "https://api.openai.com/auth":{
                "chatgpt_plan_type":"plus",
                "chatgpt_subscription_active_until":"2026-06-23T03:09:46+00:00"
              }
            }"#,
        );
        let token = format!("{header}.{payload}.sig");
        let auth = serde_json::json!({ "tokens": { "id_token": token } });
        let profile = jwt_profile(&auth).expect("profile");
        assert_eq!(profile.name, "Ada");
        assert_eq!(profile.plan, "plus");
        assert_eq!(profile.plan_until, "2026-06-23");
    }

    fn fixture_thread(id: &str, cwd: &str, title: &str) -> CodexThreadRow {
        CodexThreadRow {
            id: id.into(),
            cwd: cwd.into(),
            title: title.into(),
            rollout_path: "/home/.codex/sessions/rollout.jsonl".into(),
            source: "vscode".into(),
            model_provider: "openai".into(),
            cli_version: String::new(),
            created_at_ms: None,
            updated_at_ms: None,
            first_user_message: String::new(),
            preview: String::new(),
            agent_nickname: None,
            agent_role: None,
            git_branch: None,
            git_sha: None,
            git_origin_url: None,
            approval_mode: String::new(),
            model: None,
        }
    }

    #[test]
    fn thread_db_json_parses_sqlite_shape() {
        let json = br#"[
          {
            "id": "019e251d-03ed-76a1-87f6-eecda6eb88a8",
            "cwd": "/repo",
            "title": "Research live activity ios",
            "rollout_path": "/home/.codex/sessions/2026/05/14/rollout.jsonl",
            "source": "vscode",
            "model_provider": "openai",
            "cli_version": "0.130.0",
            "created_at_ms": 1778746700000,
            "updated_at_ms": 1778746704000,
            "first_user_message": "research live activity",
            "agent_nickname": null,
            "agent_role": null,
            "git_branch": "main",
            "git_sha": "abc",
            "git_origin_url": "https://example.com/repo.git",
            "approval_mode": "on-request",
            "model": "gpt-5.3-codex"
          }
        ]"#;
        let threads: Vec<CodexThreadRow> = serde_json::from_slice(json).expect("parse");
        assert_eq!(threads.len(), 1);
        let summary = session_summary_from_thread(&threads[0], &cli("0.130.0"));
        assert_eq!(summary.session_id, "019e251d-03ed-76a1-87f6-eecda6eb88a8");
        assert_eq!(summary.title.as_deref(), Some("Research live activity ios"));
    }

    #[test]
    fn thread_matches_exact_workdir_only() {
        let workdir = PathBuf::from("/Users/me/projects/zedra-main");
        let matching = fixture_thread("019e", "/Users/me/projects/zedra-main", "Main session");
        let sibling = fixture_thread("019f", "/Users/me/projects/zedra", "Sibling session");
        assert!(paths_equal(&workdir, Path::new(&matching.cwd)));
        assert!(!paths_equal(&workdir, Path::new(&sibling.cwd)));
    }

    #[test]
    fn title_from_thread_prefers_db_title() {
        let mut thread = fixture_thread("019e", "/repo", "Final title");
        thread.first_user_message = "initial prompt".into();
        assert_eq!(title_from_thread(&thread).as_deref(), Some("Final title"));
    }

    #[test]
    fn title_from_thread_prefers_db_title_over_cwd_message() {
        let mut thread = fixture_thread(
            "019e",
            "/Users/me/projects/zedra-main",
            "Research Gemini CLI integration",
        );
        thread.first_user_message =
            "CWD: /Users/me/projects/zedra-main. Research Gemini CLI integration opportunities"
                .into();
        assert_eq!(
            title_from_thread(&thread).as_deref(),
            Some("Research Gemini CLI integration")
        );
    }

    #[test]
    fn title_from_thread_falls_back_to_preview_before_first_user_message() {
        let mut thread = fixture_thread("019e", "/repo", "");
        thread.preview = "Preview title".into();
        thread.first_user_message = "CWD: /repo. Raw prompt body".into();
        assert_eq!(title_from_thread(&thread).as_deref(), Some("Preview title"));
    }

    #[test]
    fn title_from_thread_falls_back_to_first_user_message() {
        let mut thread = fixture_thread("019e", "/repo", "");
        thread.first_user_message =
            "research how to implement live activity ios for Zedra\n".into();
        assert_eq!(
            title_from_thread(&thread).as_deref(),
            Some("research how to implement live activity ios for Zedra")
        );
    }

    #[test]
    fn sanitize_prompt_fallback_strips_subagent_cwd_prefix() {
        assert_eq!(
            sanitize_prompt_fallback(
                "CWD: /repo. Research Gemini CLI integration opportunities for Zedra"
            )
            .as_deref(),
            Some("Research Gemini CLI integration opportunities for Zedra")
        );
    }

    #[test]
    fn sanitize_title_field_keeps_db_title_without_cwd_strip() {
        assert_eq!(
            sanitize_title_field("Research Gemini CLI integration").as_deref(),
            Some("Research Gemini CLI integration")
        );
    }

    #[test]
    fn title_from_thread_sanitizes_continue_path_db_titles() {
        let mut thread = fixture_thread(
            "019e",
            "/Users/me/projects/zedra-main",
            "continue /Users/me/projects/zedra-main/docs/CLAUDE_HOST_INTEGRATION_PLAN.md",
        );
        thread.first_user_message =
            "continue /Users/me/projects/zedra-main/docs/CLAUDE_HOST_INTEGRATION_PLAN.md".into();
        assert_eq!(
            title_from_thread(&thread).as_deref(),
            Some("CLAUDE_HOST_INTEGRATION_PLAN")
        );
    }

    #[test]
    fn title_from_agent_identity_formats_role() {
        assert_eq!(
            title_from_agent_identity(Some("Aquinas"), Some("explorer")).as_deref(),
            Some("Aquinas (explorer)")
        );
    }
}
