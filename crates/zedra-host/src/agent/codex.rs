use super::utils::*;
use crate::sqlite_readonly;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use zedra_rpc::proto::*;

#[derive(Debug, Deserialize)]
pub struct CodexThreadRow {
    pub id: String,
    pub cwd: String,
    pub title: String,
    pub rollout_path: String,
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
}

impl CodexActor {
    pub fn cli_available() -> bool {
        command_on_path("codex") || Self::state_db_path().is_some()
    }

    pub fn session_counts(workdir: &Path) -> Result<super::SessionCounts, String> {
        let threads = Self::threads_for_workdir(workdir)?;
        let latest = threads.first();
        Ok(super::SessionCounts::from_latest(
            threads.len(),
            latest.map(|thread| thread.id.clone()),
            latest.and_then(Self::title_from_thread),
            latest.and_then(Self::thread_updated_at),
        ))
    }

    pub fn sessions(
        workdir: &Path,
        cli: &AgentCliSummary,
        limit: usize,
    ) -> Result<(Vec<AgentSessionSummary>, usize), String> {
        let threads = Self::threads_for_workdir(workdir)?;
        let total = threads.len();
        let summaries = threads
            .into_iter()
            .take(limit)
            .map(|thread| Self::session_summary_from_thread(&thread, cli))
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

    pub fn threads_for_workdir(workdir: &Path) -> Result<Vec<CodexThreadRow>, String> {
        // A CLI-only install with no state DB yet is "zero sessions", not an error.
        let Some(db_path) = Self::state_db_path() else {
            return Ok(Vec::new());
        };
        let cwd_keys = Self::workdir_keys(workdir);
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
            created_at_ms,
            updated_at_ms,
            first_user_message,
            preview,
            agent_nickname,
            agent_role,
            git_branch,
            git_sha,
            git_origin_url
        FROM threads
        WHERE archived = 0 AND cwd IN ({cwd_filter})
        ORDER BY updated_at_ms DESC
    "#
        );
        sqlite_readonly::query_rows(&db_path, &query)
    }

    /// Session title for a Codex thread id within a workdir; `None` if not found or untitled.
    pub fn title_for_session(workdir: &Path, session_id: &str) -> Option<String> {
        Self::threads_for_workdir(workdir)
            .ok()?
            .into_iter()
            .find(|t| t.id == session_id)
            .and_then(|t| Self::title_from_thread(&t))
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
        Self::sanitize_title_field(&thread.title)
            .or_else(|| Self::sanitize_title_field(&thread.preview))
            .or_else(|| Self::sanitize_prompt_fallback(&thread.first_user_message))
            .or_else(|| {
                Self::title_from_agent_identity(
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
                line = Self::title_from_path(rest).unwrap_or(rest);
            }
        } else if line.starts_with('/') || line.starts_with('~') {
            line = Self::title_from_path(line).unwrap_or(line);
        }
        Self::finalize_title(line)
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
        Self::sanitize_title_field(line)
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
            slug: "codex".to_string(),
            session_id: thread.id.clone(),
            title: session_title(Self::title_from_thread(thread)),
            cwd: Some(thread.cwd.clone()),
            created_at: thread
                .created_at_ms
                .and_then(DateTime::<Utc>::from_timestamp_millis),
            last_activity_at: Self::thread_updated_at(thread),
            resume: resume_summary("codex", &thread.id),
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
        Self::append_auth_plan_fields(&mut fields);
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
        if let Some(counts) = Self::thread_counts() {
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
        Self::append_auth_plan_fields(&mut fields);
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
        let Some(profile) = value.as_ref().and_then(Self::jwt_profile) else {
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
}

pub struct CodexProfile {
    pub name: String,
    pub plan: String,
    pub plan_until: String,
}

impl CodexActor {
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
}

struct ThreadCounts {
    week: u64,
    total: u64,
}

impl CodexActor {
    fn thread_counts() -> Option<ThreadCounts> {
        // Use the selected DB so rollups survive a `state_*.sqlite` version bump.
        let db_path = Self::state_db_path()?;
        let week_start = (Utc::now() - chrono::Duration::days(7))
            .format("%Y-%m-%d")
            .to_string();
        let week_ts_ms = chrono::NaiveDate::parse_from_str(&week_start, "%Y-%m-%d")
            .ok()?
            .and_hms_opt(0, 0, 0)?
            .and_utc()
            .timestamp_millis();
        // `created_at` is seconds; use `created_at_ms` like the session scan.
        let query = format!(
            "SELECT \
            (SELECT COUNT(*) FROM threads) AS total, \
            (SELECT COUNT(*) FROM threads WHERE created_at_ms >= {week_ts_ms}) AS week;"
        );
        let rows: Vec<Value> = sqlite_readonly::query_rows(&db_path, &query).ok()?;
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
            rate_limit_five_hour_used_percent: five_hour,
            rate_limit_seven_day_used_percent: seven_day,
            rate_limit_five_hour_resets_at: five_hour_resets_at,
            rate_limit_seven_day_resets_at: seven_day_resets_at,
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::utils::paths_equal;

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
        let profile = CodexActor::jwt_profile(&auth).expect("profile");
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
            created_at_ms: None,
            updated_at_ms: None,
            first_user_message: String::new(),
            preview: String::new(),
            agent_nickname: None,
            agent_role: None,
            git_branch: None,
            git_sha: None,
            git_origin_url: None,
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
            "created_at_ms": 1778746700000,
            "updated_at_ms": 1778746704000,
            "first_user_message": "research live activity",
            "agent_nickname": null,
            "agent_role": null,
            "git_branch": "main",
            "git_sha": "abc",
            "git_origin_url": "https://example.com/repo.git"
          }
        ]"#;
        let threads: Vec<CodexThreadRow> = serde_json::from_slice(json).expect("parse");
        assert_eq!(threads.len(), 1);
        let summary = CodexActor::session_summary_from_thread(&threads[0], &cli("0.130.0"));
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
        assert_eq!(
            CodexActor::title_from_thread(&thread).as_deref(),
            Some("Final title")
        );
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
            CodexActor::title_from_thread(&thread).as_deref(),
            Some("Research Gemini CLI integration")
        );
    }

    #[test]
    fn title_from_thread_falls_back_to_preview_before_first_user_message() {
        let mut thread = fixture_thread("019e", "/repo", "");
        thread.preview = "Preview title".into();
        thread.first_user_message = "CWD: /repo. Raw prompt body".into();
        assert_eq!(
            CodexActor::title_from_thread(&thread).as_deref(),
            Some("Preview title")
        );
    }

    #[test]
    fn title_from_thread_falls_back_to_first_user_message() {
        let mut thread = fixture_thread("019e", "/repo", "");
        thread.first_user_message =
            "research how to implement live activity ios for Zedra\n".into();
        assert_eq!(
            CodexActor::title_from_thread(&thread).as_deref(),
            Some("research how to implement live activity ios for Zedra")
        );
    }

    #[test]
    fn sanitize_prompt_fallback_strips_subagent_cwd_prefix() {
        assert_eq!(
            CodexActor::sanitize_prompt_fallback(
                "CWD: /repo. Research Gemini CLI integration opportunities for Zedra"
            )
            .as_deref(),
            Some("Research Gemini CLI integration opportunities for Zedra")
        );
    }

    #[test]
    fn sanitize_title_field_keeps_db_title_without_cwd_strip() {
        assert_eq!(
            CodexActor::sanitize_title_field("Research Gemini CLI integration").as_deref(),
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
            CodexActor::title_from_thread(&thread).as_deref(),
            Some("CLAUDE_HOST_INTEGRATION_PLAN")
        );
    }

    #[test]
    fn title_from_agent_identity_formats_role() {
        assert_eq!(
            CodexActor::title_from_agent_identity(Some("Aquinas"), Some("explorer")).as_deref(),
            Some("Aquinas (explorer)")
        );
    }
}

use super::hook::HookContext;
use super::{
    home_path, hook_file_mentions_zedra, hooks_enabled, setup_status, ActorFuture, AgentActor,
    ScanCtx, SessionCounts as ActorSessionCounts,
};

pub(super) struct CodexActor;

impl AgentActor for CodexActor {
    fn shows_detail(&self) -> bool {
        true
    }

    fn slug(&self) -> &'static str {
        "codex"
    }

    fn display_name(&self) -> &'static str {
        "Codex"
    }

    fn icon_name(&self) -> &'static str {
        "openai"
    }

    fn programs(&self) -> &'static [&'static str] {
        &["codex"]
    }

    fn detect_aliases(&self) -> &'static [&'static str] {
        &["codex"]
    }

    fn detect_exact(&self) -> &'static [&'static str] {
        &["openai"]
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

    fn account_fields(&self, _workdir: &Path) -> Vec<AgentInfoField> {
        Self::account_fields()
    }

    fn setup_summary(&self, available: bool, workdir: &Path) -> AgentSetupSummary {
        let config =
            std::fs::read_to_string(home_path(&[".codex", "config.toml"])).unwrap_or_default();
        let plugin_installed = Self::codex_plugin_enabled(&config);
        let hooks_installed = hooks_enabled()
            && ((config.contains("zedra") && config.contains("hook"))
                || hook_file_mentions_zedra(&workdir.join(".codex/hooks.json")));
        setup_status(available, false, plugin_installed, hooks_installed, None)
    }

    // `--no-alt-screen` keeps native scrollback; the `zedra codex` prefix
    // catches a session that is already open.
    fn resume_launch_command(&self, quoted: &str) -> Option<String> {
        let prefix = super::wrapper_prefix("codex")?;
        Some(format!(
            "{prefix} resume --no-alt-screen --dangerously-bypass-approvals-and-sandbox {quoted}"
        ))
    }

    fn run_wrapped(&self, args: &[String]) -> Option<Result<(), String>> {
        Some(Self::wrap_cli(args))
    }

    fn default_launch_command(&self) -> Option<String> {
        self.resolved_program()
            .map(|_| "codex --no-alt-screen --dangerously-bypass-approvals-and-sandbox".to_string())
    }

    fn subscription_plan<'a>(&'a self) -> ActorFuture<'a, Option<Vec<AgentInfoField>>> {
        spawn_blocking_opt(Self::subscription_plan_fields)
    }

    fn account_usage<'a>(&'a self) -> ActorFuture<'a, Option<AgentUsageSnapshot>> {
        Box::pin(Self::fetch_account_usage())
    }

    fn supports_hooks(&self) -> bool {
        true
    }

    // Codex stores titles in its thread DB; look up by session id.
    fn hook_notify_body(
        &self,
        ctx: &HookContext,
        agent_session_id: Option<String>,
    ) -> ActorFuture<'static, Option<String>> {
        let workdir = ctx.workdir.clone();
        spawn_blocking_opt(move || {
            agent_session_id
                .as_deref()
                .and_then(|id| Self::title_for_session(&workdir, id))
        })
    }

    fn supports_setup(&self) -> bool {
        true
    }

    fn setup(&self, workdir: &Path, force: bool) -> anyhow::Result<Vec<PathBuf>> {
        let script_path = super::cli::write_hook_script(workdir, force)?;
        let config_path = workdir.join(".codex/hooks.json");
        super::utils::write_json_file_checked(
            &config_path,
            &super::utils::hook_config_from_events(&script_path, "codex", Self::HOOK_EVENTS),
            force,
            "Codex local hook config",
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
            ctx.require_command("codex")?;
            match action {
                super::SetupAction::Install => {
                    ctx.install_plugin_and_hooks(&Self::plugin_setup(&ctx)?)
                }
                super::SetupAction::Remove => {
                    ctx.remove_plugin_and_hooks(&Self::plugin_setup(&ctx)?)
                }
            }
        })
    }

    fn hook_test_payload(&self, event_name: &str, workdir: &Path) -> serde_json::Value {
        serde_json::json!({
            "hook_event_name": event_name,
            "session_id": "zedra-test-session",
            "cwd": workdir,
            "tool_name": "Bash",
        })
    }
}

impl CodexActor {
    pub(crate) fn codex_plugin_enabled(contents: &str) -> bool {
        let mut in_zedra_plugin = false;
        for line in contents.lines().map(str::trim) {
            if line.starts_with('[') {
                in_zedra_plugin = line == r#"[plugins."zedra@zedra"]"#;
            } else if in_zedra_plugin && line == "enabled = true" {
                return true;
            }
        }
        false
    }
}

// ---------------------------------------------------------------------------
// Interactive `zedra setup codex`
// ---------------------------------------------------------------------------

const CODEX_PLUGIN: &str = "zedra@zedra";

impl CodexActor {
    fn plugin_setup(
        ctx: &super::SetupCliCtx,
    ) -> anyhow::Result<super::setup::PluginSetup<'static>> {
        Ok(super::setup::PluginSetup {
            display: "Codex",
            program: "codex",
            install_args: &["plugin", "add", CODEX_PLUGIN],
            uninstall_args: &["plugin", "remove", CODEX_PLUGIN],
            hooks_path: Self::codex_hooks_path(ctx)?,
            events: Self::HOOK_EVENTS,
            agent: "codex",
            start_in: "Codex",
            start_command: "$zedra-start",
            reload_note: "Restart Codex or reload skills to apply the change.",
            reload_command: None,
        })
    }

    fn codex_hooks_path(ctx: &super::SetupCliCtx) -> anyhow::Result<PathBuf> {
        Ok(ctx.home_dir()?.join(".codex").join("hooks.json"))
    }
}

// ---------------------------------------------------------------------------
// Workdir-scoped hook config written by `AgentActor::setup`
// ---------------------------------------------------------------------------

impl CodexActor {
    // Single source for hook registration; the receive_hook state map consumes the same names.
    const HOOK_EVENTS: &'static [(&'static str, Option<&'static str>, u64)] = &[
        ("UserPromptSubmit", None, 2),
        ("PermissionRequest", Some("*"), 30),
        ("PostToolUse", Some("*"), 2),
        ("Stop", None, 2),
    ];
}

enum Choice {
    Kill,
    Fork,
    Cancel,
}

struct Holder {
    pid: u32,
    detail: String,
}

// `zedra codex <args>` forwards to the codex CLI, guarding `resume` against the
// per-thread writer lock (`$CODEX_HOME/thread-writer-locks/<id>.lock`) codex
// holds for a session's whole life: resuming a session open in another terminal
// otherwise fails with "already has an active writer" and exits at once.
impl CodexActor {
    fn wrap_cli(args: &[String]) -> Result<(), String> {
        let Some(session_id) = Self::resume_target(args) else {
            return Self::exec_codex(args.to_vec());
        };
        let Some(holder) = Self::lock_holder(&session_id) else {
            return Self::exec_codex(args.to_vec());
        };
        match Self::prompt(&session_id, &holder) {
            Choice::Kill => {
                Self::release(&holder, &session_id)?;
                Self::exec_codex(args.to_vec())
            }
            Choice::Fork => Self::exec_codex(Self::fork_args(args)),
            Choice::Cancel => {
                println!("Cancelled.");
                std::process::exit(1);
            }
        }
    }

    /// Session id a `resume` targets; `None` when codex will pick one itself.
    fn resume_target(args: &[String]) -> Option<String> {
        if args.first().map(String::as_str) != Some("resume") {
            return None;
        }
        if let Some(id) = args.iter().skip(1).find(|arg| Self::is_session_id(arg)) {
            return Some(id.clone());
        }
        args.iter()
            .any(|arg| arg == "--last")
            .then(Self::latest_session)
            .flatten()
    }

    fn is_session_id(value: &str) -> bool {
        value.len() == 36
            && value.bytes().enumerate().all(|(i, b)| match i {
                8 | 13 | 18 | 23 => b == b'-',
                _ => b.is_ascii_hexdigit(),
            })
    }

    fn latest_session() -> Option<String> {
        let cwd = std::env::current_dir().ok()?;
        Self::threads_for_workdir(&cwd)
            .ok()?
            .into_iter()
            .next()
            .map(|thread| thread.id)
    }

    fn lock_path(session_id: &str) -> PathBuf {
        std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home_path(&[".codex"]))
            .join("thread-writer-locks")
            .join(format!("{session_id}.lock"))
    }

    /// `None` when the session is free, or when its holder cannot be named —
    /// codex then reports the conflict itself, as it did before this wrapper.
    fn lock_holder(session_id: &str) -> Option<Holder> {
        let path = Self::lock_path(session_id);
        if Self::lock_free(&path) {
            return None;
        }
        let pid = Self::lock_pid(&path)?;
        Some(Holder {
            pid,
            detail: Self::describe(pid),
        })
    }

    /// A missing file or a probe error counts as free, so a broken probe never
    /// blocks a resume codex would have allowed.
    fn lock_free(path: &Path) -> bool {
        let Ok(file) = std::fs::File::open(path) else {
            return true;
        };
        // The probe lock is released when `file` drops.
        !matches!(file.try_lock(), Err(std::fs::TryLockError::WouldBlock))
    }

    fn lock_pid(path: &Path) -> Option<u32> {
        let output = std::process::Command::new("lsof")
            .arg("-t")
            .arg(path)
            .output()
            .ok()?;
        String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    }

    /// `pid 123 · ttys020 · codex`; flags are dropped to fit a phone screen.
    fn describe(pid: u32) -> String {
        let Ok(output) = std::process::Command::new("ps")
            .args(["-o", "tty=,command=", "-p", &pid.to_string()])
            .output()
        else {
            return format!("pid {pid}");
        };
        let line = String::from_utf8_lossy(&output.stdout);
        let mut fields = line.split_whitespace();
        match (fields.next(), fields.next()) {
            (Some(tty), Some(program)) => {
                let program = program.rsplit('/').next().unwrap_or(program);
                format!("pid {pid} \u{b7} {tty} \u{b7} {program}")
            }
            _ => format!("pid {pid}"),
        }
    }

    // One fact per line, so nothing wraps on a phone-width terminal.
    fn prompt(session_id: &str, holder: &Holder) -> Choice {
        let short = session_id.split('-').next().unwrap_or(session_id);
        println!("\nSession {short} is open elsewhere");
        println!("{}", holder.detail);
        if !std::io::stdin().is_terminal() {
            println!("\nNot a terminal; cancelled.");
            return Choice::Cancel;
        }
        println!("\n  y  kill it, resume here (default)");
        println!("  f  fork to a new session");
        println!("  n  cancel\n");
        loop {
            print!("[Y/f/n] ");
            let _ = std::io::stdout().flush();
            let mut line = String::new();
            if std::io::stdin().read_line(&mut line).is_err() {
                return Choice::Cancel;
            }
            match line.trim().to_ascii_lowercase().as_str() {
                "" | "y" => return Choice::Kill,
                "f" => return Choice::Fork,
                "n" => return Choice::Cancel,
                _ => {}
            }
        }
    }

    /// `codex fork` takes the same flags as `resume` except this one.
    fn fork_args(args: &[String]) -> Vec<String> {
        args.iter()
            .enumerate()
            .filter(|(_, arg)| arg.as_str() != "--include-non-interactive")
            .map(|(i, arg)| {
                if i == 0 {
                    "fork".to_string()
                } else {
                    arg.clone()
                }
            })
            .collect()
    }

    fn release(holder: &Holder, session_id: &str) -> Result<(), String> {
        let path = Self::lock_path(session_id);
        Self::signal(holder.pid, false)?;
        if Self::wait_free(&path, 3) {
            return Ok(());
        }
        Self::signal(holder.pid, true)?;
        if Self::wait_free(&path, 2) {
            return Ok(());
        }
        Err(format!(
            "codex (pid {}) still holds the session",
            holder.pid
        ))
    }

    fn wait_free(path: &Path, secs: u64) -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
        while std::time::Instant::now() < deadline {
            if Self::lock_free(path) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        Self::lock_free(path)
    }

    #[cfg(unix)]
    fn signal(pid: u32, force: bool) -> Result<(), String> {
        let signal = if force { libc::SIGKILL } else { libc::SIGTERM };
        // Safety: `kill` on a pid read from the lock file; failures are reported.
        if unsafe { libc::kill(pid as libc::pid_t, signal) } == 0 {
            return Ok(());
        }
        let err = std::io::Error::last_os_error();
        match err.raw_os_error() {
            Some(code) if code == libc::ESRCH => Ok(()),
            _ => Err(format!("failed to signal pid {pid}: {err}")),
        }
    }

    #[cfg(not(unix))]
    fn signal(pid: u32, _force: bool) -> Result<(), String> {
        Err(format!("cannot stop pid {pid} on this platform"))
    }

    #[cfg(unix)]
    fn exec_codex(args: Vec<String>) -> Result<(), String> {
        use std::os::unix::process::CommandExt;
        // `exec` replaces this process so the TUI owns the terminal directly.
        let err = std::process::Command::new("codex").args(&args).exec();
        Err(format!("failed to run codex: {err}"))
    }

    #[cfg(not(unix))]
    fn exec_codex(args: Vec<String>) -> Result<(), String> {
        let status = std::process::Command::new("codex")
            .args(&args)
            .status()
            .map_err(|err| format!("failed to run codex: {err}"))?;
        std::process::exit(status.code().unwrap_or(1));
    }
}

#[cfg(test)]
mod wrap_cli_tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    #[test]
    fn resume_target_reads_the_session_id_past_flags() {
        let id = "01a03ed7-5678-7002-8eb1-b0d7fc14b291";
        assert_eq!(
            CodexActor::resume_target(&args(&["resume", "--no-alt-screen", id])),
            Some(id.to_string())
        );
        assert_eq!(CodexActor::resume_target(&args(&["fork", id])), None);
        assert_eq!(CodexActor::resume_target(&args(&["resume"])), None);
        assert_eq!(
            CodexActor::resume_target(&args(&["resume", "-m", "gpt"])),
            None
        );
    }

    #[test]
    fn fork_args_swap_the_verb_and_drop_the_resume_only_flag() {
        let id = "01a03ed7-5678-7002-8eb1-b0d7fc14b291";
        assert_eq!(
            CodexActor::fork_args(&args(&[
                "resume",
                "--no-alt-screen",
                "--include-non-interactive",
                id,
            ])),
            args(&["fork", "--no-alt-screen", id])
        );
    }

    #[test]
    fn session_ids_are_uuid_shaped() {
        assert!(CodexActor::is_session_id(
            "01a03ed7-5678-7002-8eb1-b0d7fc14b291"
        ));
        assert!(!CodexActor::is_session_id("--no-alt-screen"));
        assert!(!CodexActor::is_session_id("01a03ed7"));
    }

    #[test]
    fn a_missing_lock_file_reads_as_free() {
        assert!(CodexActor::lock_free(Path::new(
            "/tmp/zedra-no-such-codex-lock"
        )));
    }
}

#[cfg(test)]
mod hook_config_tests {
    use super::*;

    #[test]
    fn codex_hook_config_includes_prompt_submit() {
        let config = crate::agent::utils::hook_config_from_events(
            Path::new("/tmp/zedra-hook.sh"),
            "codex",
            CodexActor::HOOK_EVENTS,
        );
        let hooks = config["hooks"].as_object().unwrap();

        for &(event, _, _) in CodexActor::HOOK_EVENTS {
            assert!(hooks.contains_key(event), "missing {event}");
        }
        assert!(!hooks.contains_key("SessionStart"));
    }
}
