//! Hermes is a global personal agent: sessions are not scoped to a workspace
//! (flat `~/.hermes/sessions/*.json`, no `cwd`), so every scan ignores the
//! workdir and returns all sessions. Beyond sessions we surface Hermes's
//! config/memory layer (SOUL.md, USER.md, MEMORY.md, config.yaml,
//! cron/jobs.json) read-only via [`config_files`]. `.env` is deliberately
//! excluded so secrets never reach the client.

use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDateTime, Utc};
use serde_json::Value;
use zedra_rpc::proto::*;

use super::utils::{
    command_on_path, file_size_bytes, home_path, info_field, mtime_unix_secs, read_json_file,
    resume_summary, session_title, spawn_blocking_opt, user_message_text,
};

/// Cap per-file content shipped to the client. Hermes memory/config files are
/// small, but the cap bounds a pathological file from bloating the reply.
const FILE_VIEW_MAX_BYTES: usize = 256 * 1024;

pub struct SessionCounts {
    pub total: usize,
    pub resumable: usize,
    pub latest_session_id: Option<String>,
    pub latest_session_title: Option<String>,
    pub last_activity_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default)]
struct HermesSession {
    session_id: String,
    title: Option<String>,
    cost_usd: Option<f64>,
    created_at: Option<DateTime<Utc>>,
    last_activity_at: Option<DateTime<Utc>>,
    /// JSON transcript path when scanned from disk; None for db-only sessions.
    transcript_path: Option<PathBuf>,
}

impl HermesActor {
    pub fn cli_available() -> bool {
        // `state.db` alone is enough to serve history even without the CLI or a
        // sessions dir, so treat its presence as availability.
        command_on_path("hermes")
            || Self::sessions_dir().is_dir()
            || Self::state_db_path().is_file()
    }

    /// Sessions are global, so `workdir` is intentionally ignored.
    pub fn session_counts(_workdir: &Path) -> Result<SessionCounts, String> {
        let total = Self::count_sessions();
        let latest = Self::collect_sessions(Some(1)).into_iter().next();
        Ok(SessionCounts {
            total,
            resumable: total,
            latest_session_id: latest.as_ref().map(|s| s.session_id.clone()),
            latest_session_title: latest.as_ref().and_then(|s| s.title.clone()),
            last_activity_at: latest.and_then(|s| s.last_activity_at),
        })
    }

    /// Sessions are global, so `workdir` is intentionally ignored.
    pub fn sessions(
        _workdir: &Path,
        cli: &AgentCliSummary,
        limit: usize,
    ) -> Result<(Vec<AgentSessionSummary>, usize), String> {
        let total = Self::count_sessions();
        let summaries = Self::collect_sessions(Some(limit))
            .iter()
            .map(|s| Self::session_summary(s, cli))
            .collect();
        Ok((summaries, total))
    }

    pub fn account_fields() -> Vec<AgentInfoField> {
        let mut fields = Vec::new();
        let (active, providers) = Self::read_auth();
        if providers.is_empty() {
            fields.push(info_field("Logged in", "no"));
        } else {
            for (id, mode) in &providers {
                let value = if Some(id.as_str()) == active.as_deref() {
                    format!("{mode} · active")
                } else {
                    mode.clone()
                };
                fields.push(info_field(id, &value));
            }
        }
        let cfg = Self::read_config();
        if let Some(model) = cfg.default_model {
            fields.push(info_field("Default model", &model));
        }
        if let Some(provider) = cfg.default_provider {
            fields.push(info_field("Default provider", &provider));
        }
        let skills = Self::skill_count();
        if skills > 0 {
            fields.push(info_field("Skills", &skills.to_string()));
        }
        Self::append_rollups(&mut fields);
        fields
    }

    /// Number of installed skills (agentskills.io `SKILL.md` files under
    /// `$HERMES_HOME/skills`, nested by category).
    fn skill_count() -> usize {
        fn walk(dir: &Path, depth: usize) -> usize {
            if depth > 4 {
                return 0;
            }
            let Ok(entries) = std::fs::read_dir(dir) else {
                return 0;
            };
            let mut count = 0;
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    count += walk(&path, depth + 1);
                } else if path.file_name().and_then(|n| n.to_str()) == Some("SKILL.md") {
                    count += 1;
                }
            }
            count
        }
        walk(&Self::hermes_home().join("skills"), 0)
    }

    /// Account-level rollups from `state.db`: lifetime spend, session count, and the
    /// platforms Hermes has run on. Skipped silently when the db is absent.
    fn append_rollups(fields: &mut Vec<AgentInfoField>) {
        let db = Self::state_db_path();
        if !db.is_file() {
            return;
        }
        let Ok(conn) = crate::sqlite_readonly::open(&db) else {
            return;
        };
        if let Ok(spend) = conn.query_row(
        "SELECT COALESCE(SUM(COALESCE(actual_cost_usd, estimated_cost_usd, 0)), 0) FROM sessions",
        [],
        |row| row.get::<_, f64>(0),
    ) {
        if spend > 0.0 {
            fields.push(info_field("Total spend", &format!("${spend:.2}")));
        }
    }
        let platforms = conn
            .prepare("SELECT DISTINCT source FROM sessions WHERE source != ''")
            .and_then(|mut stmt| {
                let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
                Ok(rows.flatten().collect::<Vec<String>>())
            });
        if let Ok(mut platforms) = platforms {
            platforms.sort();
            if !platforms.is_empty() {
                fields.push(info_field("Platforms", &platforms.join(", ")));
            }
        }
    }

    pub fn subscription_plan_fields() -> Option<Vec<AgentInfoField>> {
        // Provider/auth state is local and already populated by `account_fields`;
        // no remote plan to fetch.
        None
    }

    pub fn fetch_account_usage() -> Option<AgentUsageSnapshot> {
        None
    }

    /// Read-only snapshot of Hermes's config/memory files for the agent detail view.
    /// Absent files are returned with `missing = true` so the UI can show them as
    /// "not created yet" rather than omitting them.
    pub fn config_files() -> Vec<AgentFile> {
        Self::config_files_in(&Self::hermes_home())
    }

    fn config_files_in(home: &Path) -> Vec<AgentFile> {
        // `label` doubles as the relative path under HERMES_HOME.
        // `.env` is intentionally excluded: it holds API keys/tokens and
        // `read_view_file` would ship its contents to the client.
        const VIEW_FILES: &[&str] = &[
            "SOUL.md",
            "USER.md",
            "MEMORY.md",
            "config.yaml",
            "cron/jobs.json",
        ];
        VIEW_FILES
            .iter()
            .map(|name| Self::read_view_file(name, &home.join(name)))
            .collect()
    }

    // ---------------------------------------------------------------------------
    // Session scan (global)
    // ---------------------------------------------------------------------------

    pub fn hermes_home() -> PathBuf {
        std::env::var_os("HERMES_HOME")
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| home_path(&[".hermes"]))
    }

    fn sessions_dir() -> PathBuf {
        Self::hermes_home().join("sessions")
    }

    fn state_db_path() -> PathBuf {
        Self::hermes_home().join("state.db")
    }

    fn count_sessions() -> usize {
        if let Some(count) = Self::count_sessions_db() {
            return count;
        }
        let Ok(entries) = std::fs::read_dir(Self::sessions_dir()) else {
            return 0;
        };
        entries
            .flatten()
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
            .count()
    }

    /// `state.db` is the canonical store (richer than the per-session JSON and it
    /// also covers gateway/ACP sessions that never get a JSON file). Fall back to
    /// the JSON scan only when the db is absent or unreadable.
    fn collect_sessions(limit: Option<usize>) -> Vec<HermesSession> {
        if let Some(sessions) = Self::collect_sessions_from_db(limit) {
            return sessions;
        }
        Self::collect_sessions_from_json(limit)
    }

    fn collect_sessions_from_json(limit: Option<usize>) -> Vec<HermesSession> {
        let Ok(entries) = std::fs::read_dir(Self::sessions_dir()) else {
            return Vec::new();
        };
        let mut candidates: Vec<(PathBuf, Option<u64>)> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
            .map(|p| {
                let mtime = mtime_unix_secs(&p);
                (p, mtime)
            })
            .collect();
        // mtime is only a cheap proxy; parse every file and sort by actual
        // last activity before limiting, so an older-touched file with newer
        // in-content activity isn't dropped from the first page.
        candidates.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.0.cmp(&a.0)));
        let mut sessions: Vec<HermesSession> = candidates
            .into_iter()
            .filter_map(|(path, mtime)| Self::read_session(&path, mtime))
            .collect();
        sessions.sort_by(|a, b| b.last_activity_at.cmp(&a.last_activity_at));
        if let Some(limit) = limit {
            sessions.truncate(limit);
        }
        sessions
    }

    fn read_session(path: &Path, mtime_secs: Option<u64>) -> Option<HermesSession> {
        let value = read_json_file(path).ok()?;
        let session_id = value
            .get("session_id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string()
            });
        let created_at = Self::parse_timestamp(value.get("session_start"));
        let last_activity_at = Self::parse_timestamp(value.get("last_updated"))
            .or_else(|| mtime_secs.and_then(|s| DateTime::<Utc>::from_timestamp(s as i64, 0)));
        let title = Self::first_user_title(&value);

        Some(HermesSession {
            session_id,
            title,
            created_at,
            last_activity_at,
            transcript_path: Some(path.to_path_buf()),
            ..Default::default()
        })
    }
}

// ---------------------------------------------------------------------------
// state.db (canonical session store)
// ---------------------------------------------------------------------------

struct DbRow {
    id: String,
    title: Option<String>,
    cost: Option<f64>,
    started_at: Option<f64>,
    ended_at: Option<f64>,
}

impl HermesActor {
    fn count_sessions_db() -> Option<usize> {
        let db = Self::state_db_path();
        if !db.is_file() {
            return None;
        }
        let conn = crate::sqlite_readonly::open(&db).ok()?;
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
            .ok()?;
        Some(count.max(0) as usize)
    }

    fn collect_sessions_from_db(limit: Option<usize>) -> Option<Vec<HermesSession>> {
        let db = Self::state_db_path();
        if !db.is_file() {
            return None;
        }
        let conn = crate::sqlite_readonly::open(&db).ok()?;
        let limit_sql = limit.map(|l| l as i64).unwrap_or(-1); // -1 = unbounded in SQLite
        let rows: Vec<DbRow> = {
            let mut stmt = conn
                .prepare(
                    "SELECT id, title, COALESCE(actual_cost_usd, estimated_cost_usd), \
                 started_at, ended_at \
                 FROM sessions ORDER BY COALESCE(ended_at, started_at) DESC LIMIT ?1",
                )
                .ok()?;
            let mapped = stmt
                .query_map([limit_sql], |row| {
                    Ok(DbRow {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        cost: row.get(2)?,
                        started_at: row.get(3)?,
                        ended_at: row.get(4)?,
                    })
                })
                .ok()?;
            mapped.flatten().collect()
        };

        let sessions = rows
            .into_iter()
            .map(|r| {
                // Prefer the curated title; fall back to the first user message.
                let title = r
                    .title
                    .filter(|t| !t.trim().is_empty())
                    .or_else(|| Self::first_user_message_db(&conn, &r.id));
                HermesSession {
                    session_id: r.id,
                    title,
                    cost_usd: r.cost,
                    created_at: r.started_at.and_then(Self::epoch_to_dt),
                    last_activity_at: r.ended_at.or(r.started_at).and_then(Self::epoch_to_dt),
                    transcript_path: None,
                }
            })
            .collect();
        Some(sessions)
    }

    fn first_user_message_db(conn: &rusqlite::Connection, session_id: &str) -> Option<String> {
        let text: String = conn
            .query_row(
                "SELECT content FROM messages WHERE session_id = ?1 AND role = 'user' \
             AND content IS NOT NULL AND content != '' ORDER BY timestamp LIMIT 1",
                [session_id],
                |row| row.get(0),
            )
            .ok()?;
        let text = text.trim();
        if text.is_empty() || text.starts_with('<') {
            return None;
        }
        Some(text.to_string())
    }

    fn epoch_to_dt(secs: f64) -> Option<DateTime<Utc>> {
        if !secs.is_finite() || secs <= 0.0 {
            return None;
        }
        let nanos = (secs.fract() * 1_000_000_000.0) as u32;
        DateTime::<Utc>::from_timestamp(secs.trunc() as i64, nanos)
    }

    fn session_summary(session: &HermesSession, _cli: &AgentCliSummary) -> AgentSessionSummary {
        AgentSessionSummary {
            slug: "hermes".to_string(),
            session_id: session.session_id.clone(),
            title: session_title(session.title.clone()),
            cwd: None,
            created_at: session.created_at,
            last_activity_at: session.last_activity_at,
            resume: resume_summary("hermes", &session.session_id),
            git: None,
            usage: session
                .cost_usd
                .filter(|c| *c > 0.0)
                .map(|cost| AgentUsageSnapshot {
                    total_cost_usd: Some(cost),
                    ..Default::default()
                }),
            transcript_size_bytes: session.transcript_path.as_deref().and_then(file_size_bytes),
        }
    }
}

// ---------------------------------------------------------------------------
// Config / auth (account info)
// ---------------------------------------------------------------------------

#[derive(Default)]
struct HermesConfig {
    default_model: Option<String>,
    default_provider: Option<String>,
}

impl HermesActor {
    /// Hermes `config.yaml` nests model defaults under a top-level `model:` block.
    /// We only need a couple of scalars, so parse the block by indentation rather
    /// than pulling in a YAML dependency (mirrors how the Codex agent reads its
    /// `config.toml` line by line).
    fn read_config() -> HermesConfig {
        let Ok(text) = std::fs::read_to_string(Self::hermes_home().join("config.yaml")) else {
            return HermesConfig::default();
        };
        Self::parse_config(&text)
    }

    fn parse_config(text: &str) -> HermesConfig {
        let mut cfg = HermesConfig::default();
        let mut in_model = false;
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let indented = line.starts_with(char::is_whitespace);
            if !indented {
                // A new top-level key ends the `model:` block.
                in_model = trimmed == "model:";
                continue;
            }
            if in_model {
                if let Some(v) = trimmed.strip_prefix("default:") {
                    cfg.default_model = Self::yaml_scalar(v);
                } else if let Some(v) = trimmed.strip_prefix("provider:") {
                    cfg.default_provider = Self::yaml_scalar(v);
                }
            }
        }
        cfg
    }

    fn yaml_scalar(raw: &str) -> Option<String> {
        let value = raw.trim().trim_matches('"').trim_matches('\'').trim();
        (!value.is_empty()).then(|| value.to_string())
    }

    /// `(active_provider, [(provider, auth_mode)])` from `auth.json`. Never exposes
    /// tokens — only the provider id and its auth mode.
    fn read_auth() -> (Option<String>, Vec<(String, String)>) {
        let Ok(value) = read_json_file(&Self::hermes_home().join("auth.json")) else {
            return (None, Vec::new());
        };
        let active = value
            .get("active_provider")
            .and_then(Value::as_str)
            .map(str::to_string);
        let mut providers = Vec::new();
        if let Some(map) = value.get("providers").and_then(Value::as_object) {
            for (id, cfg) in map {
                let mode = cfg
                    .get("auth_mode")
                    .and_then(Value::as_str)
                    .unwrap_or("configured")
                    .to_string();
                providers.push((id.clone(), mode));
            }
            providers.sort_by(|a, b| a.0.cmp(&b.0));
        }
        (active, providers)
    }

    fn read_view_file(label: &str, path: &Path) -> AgentFile {
        match std::fs::read(path) {
            Ok(bytes) => {
                let truncated = bytes.len() > FILE_VIEW_MAX_BYTES;
                let end = bytes.len().min(FILE_VIEW_MAX_BYTES);
                AgentFile {
                    label: label.to_string(),
                    path: path.display().to_string(),
                    content: String::from_utf8_lossy(&bytes[..end]).into_owned(),
                    truncated,
                    missing: false,
                }
            }
            Err(_) => AgentFile {
                label: label.to_string(),
                path: path.display().to_string(),
                content: String::new(),
                truncated: false,
                missing: true,
            },
        }
    }

    // ---------------------------------------------------------------------------
    // Helpers
    // ---------------------------------------------------------------------------

    fn first_user_title(value: &Value) -> Option<String> {
        // Length is clamped centrally in `session_title`; the UI trims for display.
        value
            .get("messages")?
            .as_array()?
            .iter()
            .find_map(user_message_text)
    }

    /// Hermes timestamps are ISO-ish without a timezone (e.g.
    /// `2026-05-03T10:24:06.123456`); try RFC 3339 first, then naive UTC.
    fn parse_timestamp(value: Option<&Value>) -> Option<DateTime<Utc>> {
        let raw = value?.as_str()?;
        if let Ok(dt) = DateTime::parse_from_rfc3339(raw) {
            return Some(dt.with_timezone(&Utc));
        }
        for fmt in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S"] {
            if let Ok(naive) = NaiveDateTime::parse_from_str(raw, fmt) {
                return Some(naive.and_utc());
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_model_block_from_config_yaml() {
        let cfg = HermesActor::parse_config(
            "model:\n  default: gpt-5.5\n  provider: openai-codex\n  base_url: https://x\nproviders: {}\n",
        );
        assert_eq!(cfg.default_model.as_deref(), Some("gpt-5.5"));
        assert_eq!(cfg.default_provider.as_deref(), Some("openai-codex"));
    }

    #[test]
    fn parse_config_ignores_keys_outside_model_block() {
        // `provider:` under a sibling top-level block must not leak into model.
        let cfg = HermesActor::parse_config("server:\n  provider: bogus\nmodel:\n  default: m1\n");
        assert_eq!(cfg.default_model.as_deref(), Some("m1"));
        assert_eq!(cfg.default_provider, None);
    }

    #[test]
    fn first_user_title_handles_string_and_array_content() {
        let string_form = serde_json::json!({
            "messages": [
                {"role": "system", "content": "ignore"},
                {"role": "user", "content": "Summarize the docs"}
            ]
        });
        assert_eq!(
            HermesActor::first_user_title(&string_form).as_deref(),
            Some("Summarize the docs")
        );

        let array_form = serde_json::json!({
            "messages": [
                {"role": "user", "content": [{"type": "text", "text": "hello world"}]}
            ]
        });
        assert_eq!(
            HermesActor::first_user_title(&array_form).as_deref(),
            Some("hello world")
        );
    }

    #[test]
    fn config_files_marks_missing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("SOUL.md"), "persona").unwrap();
        let files = HermesActor::config_files_in(dir.path());
        let soul = files.iter().find(|f| f.label == "SOUL.md").unwrap();
        assert!(!soul.missing);
        assert_eq!(soul.content, "persona");
        let user = files.iter().find(|f| f.label == "USER.md").unwrap();
        assert!(user.missing);
    }
}

use super::hook::HookContext;
use super::{setup_status, ActorFuture, AgentActor, ScanCtx, SessionCounts as ActorSessionCounts};

pub(super) struct HermesActor;

impl HermesActor {
    fn hermes_agent_state(event_name: &str) -> Option<AgentState> {
        match event_name {
            "on_session_start" | "post_approval_response" => Some(AgentState::Running),
            "pre_approval_request" => Some(AgentState::WaitingApproval),
            "post_llm_call" | "on_session_end" => Some(AgentState::Completed),
            _ => None,
        }
    }
}

impl AgentActor for HermesActor {
    fn shows_detail(&self) -> bool {
        true
    }

    fn slug(&self) -> &'static str {
        "hermes"
    }
    fn display_name(&self) -> &'static str {
        "Hermes Agent"
    }
    fn icon_name(&self) -> &'static str {
        "hermesagent"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["hermes", "hermes-agent"]
    }

    fn detect_aliases(&self) -> &'static [&'static str] {
        &["hermes-agent", "hermes agent", "hermesagent"]
    }

    fn detect_exact(&self) -> &'static [&'static str] {
        &["hermes"]
    }

    fn is_global(&self) -> bool {
        true
    }

    fn cli_available(&self, _workdir: &Path) -> bool {
        Self::cli_available()
    }

    fn session_counts(&self, ctx: &ScanCtx) -> Result<ActorSessionCounts, String> {
        Ok(Self::session_counts(ctx.workdir)?.into())
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
    fn config_files(&self) -> Vec<AgentFile> {
        Self::config_files()
    }

    fn setup_summary(&self, available: bool, workdir: &Path) -> AgentSetupSummary {
        let _ = workdir;
        // Mirror what `setup()` installs: the global hook script plus a config.yaml
        // that references it. Without this the UI stays "setup incomplete" forever.
        let home = Self::hermes_home();
        let script_installed = home
            .join("agent-hooks")
            .join("zedra-agent-hooks.sh")
            .is_file();
        let config_mentions_hook = std::fs::read_to_string(home.join("config.yaml"))
            .map(|config| config.contains("zedra-agent-hooks"))
            .unwrap_or(false);
        let hooks_installed = super::hooks_enabled() && script_installed && config_mentions_hook;
        setup_status(available, false, false, hooks_installed, None)
    }

    fn resume_launch_command(&self, quoted: &str) -> Option<String> {
        Some(format!("hermes --resume {quoted}"))
    }

    fn subscription_plan<'a>(&'a self) -> ActorFuture<'a, Option<Vec<AgentInfoField>>> {
        spawn_blocking_opt(Self::subscription_plan_fields)
    }

    fn account_usage<'a>(&'a self) -> ActorFuture<'a, Option<AgentUsageSnapshot>> {
        spawn_blocking_opt(Self::fetch_account_usage)
    }

    fn receive_hook<'a>(&'a self, ctx: HookContext) -> ActorFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            // Hermes shell hooks pipe `hook_event_name` and place event-specific
            // fields under `extra`.
            let event_name =
                super::utils::payload_string(&ctx.payload, "hook_event_name").unwrap_or_default();
            let agent_session_id = super::utils::payload_string(&ctx.payload, "session_id")
                .filter(|value| !value.is_empty())
                .or_else(|| {
                    ctx.payload
                        .get("extra")
                        .and_then(|extra| super::utils::payload_string(extra, "session_key"))
                });
            let agent_state = Self::hermes_agent_state(&event_name);
            ctx.apply(
                "hermes",
                &event_name,
                agent_state,
                agent_session_id.as_deref(),
            )
            .await;

            // `on_session_end` follows every turn and would duplicate the successful
            // completion notification already emitted for `post_llm_call`.
            let name = self.display_name();
            let Some(title) = (match event_name.as_str() {
                "pre_approval_request" => Some(format!("{name} requires approval")),
                "post_llm_call" => Some(format!("{name} completed")),
                _ => None,
            }) else {
                return Ok(());
            };
            if ctx.client_in_foreground() {
                return Ok(());
            }
            let Some(delta) = ctx.require_delta() else {
                return Ok(());
            };

            ctx.send_notification(&delta, ctx.notification(name, &event_name, title, None))
                .await
        })
    }

    fn supports_setup(&self) -> bool {
        true
    }

    fn setup(&self, _workdir: &Path, force: bool) -> anyhow::Result<Vec<PathBuf>> {
        super::cli::write_hermes_hook_config(force)
    }

    fn hook_test_payload(&self, event_name: &str, workdir: &Path) -> serde_json::Value {
        serde_json::json!({
            "hook_event_name": event_name,
            "session_id": "zedra-test-session",
            "cwd": workdir,
        })
    }
}

#[cfg(test)]
mod hook_tests {
    use super::*;
    use crate::agent::hook::HookContext;
    use crate::session_registry::SessionRegistry;
    use serde_json::json;
    use std::path::PathBuf;

    #[test]
    fn hermes_lifecycle_events_map_to_agent_states() {
        for (event, state) in [
            ("on_session_start", AgentState::Running),
            ("pre_approval_request", AgentState::WaitingApproval),
            ("post_approval_response", AgentState::Running),
            ("post_llm_call", AgentState::Completed),
            ("on_session_end", AgentState::Completed),
        ] {
            assert_eq!(
                HermesActor::hermes_agent_state(event),
                Some(state),
                "{event}"
            );
        }
        assert_eq!(HermesActor::hermes_agent_state("unknown"), None);
    }

    #[tokio::test]
    async fn hermes_receiver_applies_state_to_terminal() {
        let registry = SessionRegistry::new();
        let session = registry
            .create_named("test", PathBuf::from("/tmp/hermes-hook-test"))
            .await;
        HermesActor
            .receive_hook(HookContext {
                payload: json!({
                    "hook_event_name": "pre_approval_request",
                    "session_id": "hermes-session",
                }),
                terminal_id: "terminal-1".to_string(),
                endpoint_addr: String::new(),
                session: session.clone(),
                delta: None,
                workdir: PathBuf::from("/tmp/hermes-hook-test"),
            })
            .await
            .unwrap();

        assert_eq!(
            session
                .terminal_agent_states
                .lock()
                .await
                .get("terminal-1")
                .copied(),
            Some(AgentState::WaitingApproval)
        );
    }
}
