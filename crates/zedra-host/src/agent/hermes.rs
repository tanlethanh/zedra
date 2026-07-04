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
    resume_summary, session_title, user_message_text,
};

/// Cap per-file content shipped to the client. Hermes memory/config files are
/// small, but the cap bounds a pathological file from bloating the reply.
const FILE_VIEW_MAX_BYTES: usize = 256 * 1024;

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
    pub fn session_counts(_workdir: &Path) -> Result<super::SessionCounts, String> {
        let total = Self::count_sessions();
        let latest = Self::collect_sessions(Some(1)).into_iter().next();
        Ok(super::SessionCounts::from_latest(
            total,
            latest.as_ref().map(|s| s.session_id.clone()),
            latest.as_ref().and_then(|s| s.title.clone()),
            latest.and_then(|s| s.last_activity_at),
        ))
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

    /// Config/memory files for the detail view; absent files come back with
    /// `missing = true` so the UI shows "not created yet".
    pub fn config_files() -> Vec<AgentFile> {
        Self::config_files_in(&Self::hermes_home())
    }

    fn config_files_in(home: &Path) -> Vec<AgentFile> {
        // `label` doubles as the relative path under HERMES_HOME.
        // `.env` excluded: it holds secrets `read_view_file` would ship to the client.
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

    fn config_path() -> PathBuf {
        Self::hermes_home().join("config.yaml")
    }

    fn hook_script_path() -> PathBuf {
        Self::hermes_home()
            .join("agent-hooks")
            .join("zedra-agent-hooks.sh")
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

    /// `state.db` is canonical (covers gateway/ACP sessions with no JSON file);
    /// fall back to the JSON scan only when the db is absent or unreadable.
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
        // mtime is a cheap proxy; sort by actual last activity before limiting
        // so an older-touched file with newer activity isn't dropped.
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
                    extra: vec![info_field("Cost", &format!("${cost:.2}"))],
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
    /// Parse the `model:` block by indentation instead of a YAML dependency —
    /// only a couple of scalars are needed (mirrors the Codex config reader).
    fn read_config() -> HermesConfig {
        let Ok(text) = std::fs::read_to_string(Self::config_path()) else {
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
    fn config_files(&self) -> Vec<AgentFile> {
        Self::config_files()
    }

    fn setup_summary(&self, available: bool, _workdir: &Path) -> AgentSetupSummary {
        // Mirror what `setup()` installs: the hook script + a config.yaml referencing it.
        let script_installed = Self::hook_script_path().is_file();
        let config_mentions_hook = std::fs::read_to_string(Self::config_path())
            .map(|config| config.contains("zedra-agent-hooks"))
            .unwrap_or(false);
        let hooks_installed = super::hooks_enabled() && script_installed && config_mentions_hook;
        setup_status(available, false, false, hooks_installed, None)
    }

    fn resume_launch_command(&self, quoted: &str) -> Option<String> {
        Some(format!("hermes --resume {quoted}"))
    }

    // No remote plan/usage endpoint: `subscription_plan`/`account_usage` keep
    // the trait's None defaults.

    fn supports_hooks(&self) -> bool {
        true
    }

    // Hermes shell hooks pipe `hook_event_name` and place event-specific
    // fields under `extra`.
    fn hook_identity(&self, payload: &Value) -> (String, Option<String>) {
        let event_name =
            super::utils::payload_string(payload, "hook_event_name").unwrap_or_default();
        let agent_session_id = super::utils::payload_string(payload, "session_id")
            .filter(|value| !value.is_empty())
            .or_else(|| {
                payload
                    .get("extra")
                    .and_then(|extra| super::utils::payload_string(extra, "session_key"))
            });
        (event_name, agent_session_id)
    }

    fn hook_state(&self, event_name: &str, _payload: &Value) -> Option<AgentState> {
        Self::hermes_agent_state(event_name)
    }

    // `on_session_end` follows every turn and would duplicate the successful
    // completion notification already emitted for `post_llm_call`.
    fn hook_notify_title(&self, event_name: &str) -> Option<String> {
        let name = self.display_name();
        match event_name {
            "pre_approval_request" => Some(format!("{name} requires approval")),
            "post_llm_call" => Some(format!("{name} completed")),
            _ => None,
        }
    }

    fn supports_setup(&self) -> bool {
        true
    }

    fn setup(&self, _workdir: &Path, force: bool) -> anyhow::Result<Vec<PathBuf>> {
        use anyhow::Context;
        let cli = std::env::current_exe().context("failed to resolve current zedra binary")?;
        Self::write_hook_config(force, &cli.display().to_string())
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
                    ctx.section("Setting up Hermes");
                    let binary = ctx.hook_binary()?;
                    let paths = Self::write_hook_config(true, &binary)?;
                    for (label, path) in ["hooks", "config"].iter().zip(&paths) {
                        ctx.step(label);
                        ctx.detail(&format!("write {}", path.display()));
                    }
                    ctx.message("Hermes setup complete.");
                }
                super::SetupAction::Remove => {
                    ctx.message("Removing Zedra lifecycle-hook script for Hermes:");
                    let script_path = Self::hook_script_path();
                    if ctx.remove_path(&script_path)? {
                        ctx.step("hooks");
                        ctx.detail(&format!("remove {}", script_path.display()));
                    }
                    let config_path = Self::config_path();
                    if config_path.is_file() {
                        let existing = std::fs::read_to_string(&config_path)?;
                        let cleaned = Self::remove_zedra_hooks(&existing);
                        if cleaned != existing {
                            std::fs::write(&config_path, &cleaned)?;
                            ctx.step("config");
                            ctx.detail(&format!("write {}", config_path.display()));
                        }
                    }
                    ctx.message("");
                    ctx.message("Hermes setup removed.");
                    ctx.message("Restart any running Hermes session to apply the change.");
                }
            }
            Ok(())
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

// ---------------------------------------------------------------------------
// Hook script + config.yaml patching for `AgentActor::setup` and `zedra setup hermes`
// ---------------------------------------------------------------------------

impl HermesActor {
    /// Hermes lifecycle events Zedra hooks into via shell hooks.
    const HOOK_EVENTS: &'static [&'static str] = &[
        "on_session_start",
        "pre_approval_request",
        "post_approval_response",
        "post_llm_call",
        "on_session_end",
    ];

    /// Writes the Hermes hook script and patches `~/.hermes/config.yaml`.
    /// Returns the list of paths written/modified (hook script + config.yaml).
    fn write_hook_config(force: bool, cli_path: &str) -> anyhow::Result<Vec<PathBuf>> {
        use anyhow::Context;
        let script_path = Self::hook_script_path();
        let script = Self::hook_script_contents(cli_path);
        super::utils::write_file_checked(&script_path, &script, force, "Hermes agent hook script")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&script_path)?.permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script_path, perms)?;
        }

        let config_path = Self::config_path();
        let existing = std::fs::read_to_string(&config_path).unwrap_or_default();
        let patched = Self::patch_config_hooks(&existing, &script_path);
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        std::fs::write(&config_path, &patched)
            .with_context(|| format!("failed to write {}", config_path.display()))?;

        Ok(vec![script_path, config_path])
    }

    fn hook_script_contents(cli_path: &str) -> String {
        format!(
            r#"#!/bin/sh
# Zedra hook script for Hermes agent.
# No-op outside a Zedra terminal (ZEDRA_TERMINAL_ID not set by the shell).
[ -z "${{ZEDRA_TERMINAL_ID:-}}" ] && exit 0
CLI="${{ZEDRA_CLI:-}}"
[ -n "$CLI" ] || CLI={cli}
[ -x "$CLI" ] || CLI="zedra"
exec "$CLI" agent hook receive --agent hermes --quiet
"#,
            cli = crate::utils::shell_arg(cli_path),
        )
    }

    /// Idempotent: strip Zedra-owned `HOOK_EVENTS` keys, re-insert fresh entries,
    /// preserve user hooks verbatim; handles `hooks: {}` and block forms.
    fn patch_config_hooks(config: &str, script_path: &Path) -> String {
        let script = script_path.display().to_string();

        let lines: Vec<&str> = config.lines().collect();
        let trailing_newline = config.ends_with('\n');

        // Find the top-level `hooks:` key (must start at column 0).
        let hooks_idx = lines.iter().position(|l| Self::is_hooks_key_line(l));

        let Some(hooks_idx) = hooks_idx else {
            // No hooks: key — append our block to the file.
            let mut out = config.to_string();
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str("hooks:\n");
            out.push_str(&Self::zedra_hooks_entries(&script));
            return out;
        };

        // Is it the inline-empty form `hooks: {}` ?
        let inline_empty =
            lines[hooks_idx].trim() == "hooks: {}" || lines[hooks_idx].trim() == "hooks:{}";

        let hooks_block_end = Self::hooks_block_end(&lines, hooks_idx);

        // Build the new hooks block.
        let mut hooks_block = String::from("hooks:\n");
        if !inline_empty {
            // Preserve non-Zedra event entries from the existing block.
            let existing_content = &lines[hooks_idx + 1..hooks_block_end];
            let preserved = Self::remove_zedra_event_blocks(existing_content, Self::HOOK_EVENTS);
            if !preserved.trim().is_empty() {
                hooks_block.push_str(&preserved);
            }
        }
        hooks_block.push_str(&Self::zedra_hooks_entries(&script));

        Self::rebuild(
            &lines[..hooks_idx],
            &hooks_block,
            &lines[hooks_block_end..],
            trailing_newline,
        )
    }

    /// End index (exclusive) of the `hooks:` block that opens at `hooks_idx`: the
    /// first subsequent top-level (column-0) non-blank, non-comment line, or EOF.
    fn hooks_block_end(lines: &[&str], hooks_idx: usize) -> usize {
        lines[hooks_idx + 1..]
            .iter()
            .position(|l| {
                !l.is_empty() && !l.starts_with(' ') && !l.starts_with('\t') && !l.starts_with('#')
            })
            .map(|i| hooks_idx + 1 + i)
            .unwrap_or(lines.len())
    }

    /// Reassemble the config from the lines before the hooks block, the rebuilt
    /// `hooks:` block, and the lines after it, matching the original trailing newline.
    fn rebuild(pre: &[&str], hooks_block: &str, post: &[&str], trailing_newline: bool) -> String {
        let mut out = String::new();
        for l in pre {
            out.push_str(l);
            out.push('\n');
        }
        out.push_str(hooks_block);
        for l in post {
            out.push_str(l);
            out.push('\n');
        }
        // Normalise trailing newline to match original.
        while out.ends_with("\n\n") {
            out.pop();
        }
        if trailing_newline && !out.ends_with('\n') {
            out.push('\n');
        }
        out
    }

    /// True when `line` is the top-level `hooks:` YAML key (column-0, no indent).
    fn is_hooks_key_line(line: &str) -> bool {
        if line.starts_with(' ') || line.starts_with('\t') {
            return false;
        }
        let t = line.trim();
        t == "hooks: {}"
            || t == "hooks:{}"
            || t == "hooks:"
            || (t.starts_with("hooks:")
                && matches!(t.as_bytes().get(6), Some(b' ') | Some(b'{') | None))
    }

    /// A block runs from a 2-space-indented `identifier:` line to the next such
    /// line or EOF; blank/comment/deeper lines follow their enclosing block.
    fn remove_zedra_event_blocks<'a>(lines: &[&'a str], remove_events: &[&str]) -> String {
        let mut out = String::new();
        let mut skip = false;

        for line in lines {
            if let Some(key) = Self::event_key_at_line(line) {
                skip = remove_events.contains(&key);
            }
            if !skip {
                out.push_str(line);
                out.push('\n');
            }
        }
        out
    }

    /// If `line` is a 2-space-indented YAML mapping key (`  identifier:`),
    /// return the key name; otherwise return `None`.
    fn event_key_at_line<'a>(line: &'a str) -> Option<&'a str> {
        let rest = line.strip_prefix("  ")?;
        if rest.starts_with(' ') {
            return None; // deeper indent — not a top-level event key
        }
        let name = rest.strip_suffix(':')?;
        if name.is_empty() || name.contains(' ') || name.contains('"') || name.contains('\'') {
            return None;
        }
        Some(name)
    }

    /// Strips all Zedra-managed event blocks from a `config.yaml` hooks section,
    /// preserving user-defined hooks (used by `zedra setup hermes --remove`).
    fn remove_zedra_hooks(config: &str) -> String {
        let lines: Vec<&str> = config.lines().collect();
        let trailing_newline = config.ends_with('\n');

        let Some(hooks_idx) = lines.iter().position(|l| Self::is_hooks_key_line(l)) else {
            return config.to_string();
        };

        let hooks_block_end = Self::hooks_block_end(&lines, hooks_idx);

        let existing_content = &lines[hooks_idx + 1..hooks_block_end];
        let preserved = Self::remove_zedra_event_blocks(existing_content, Self::HOOK_EVENTS);

        let mut hooks_block = String::from("hooks:");
        if preserved.trim().is_empty() {
            hooks_block.push_str(" {}\n");
        } else {
            hooks_block.push('\n');
            hooks_block.push_str(&preserved);
        }

        Self::rebuild(
            &lines[..hooks_idx],
            &hooks_block,
            &lines[hooks_block_end..],
            trailing_newline,
        )
    }

    /// YAML text for all Zedra-managed hook entries (2-space indented, ready to
    /// append directly inside the `hooks:` block).
    fn zedra_hooks_entries(script: &str) -> String {
        // Double any backslashes and escape double-quotes for inline YAML double-quoted scalar.
        let script_yaml = script.replace('\\', "\\\\").replace('"', "\\\"");
        let mut out = String::new();
        for event in Self::HOOK_EVENTS {
            out.push_str(&format!("  {}:\n", event));
            out.push_str(&format!("    - command: \"{}\"\n", script_yaml));
            out.push_str("      timeout: 5\n");
        }
        out
    }
}

#[cfg(test)]
mod hook_config_tests {
    use super::*;

    // -------------------------------------------------------------------------
    // patch_hermes_config_hooks
    // -------------------------------------------------------------------------

    #[test]
    fn patch_hermes_hooks_expands_inline_empty_dict() {
        let config = "model:\n  default: gpt-5\nhooks: {}\nhooks_auto_accept: false\n";
        let script = std::path::Path::new("/home/user/.hermes/agent-hooks/zedra-agent-hooks.sh");
        let patched = HermesActor::patch_config_hooks(config, script);
        assert!(patched.contains("hooks:\n"));
        assert!(!patched.contains("hooks: {}"));
        for event in HermesActor::HOOK_EVENTS {
            assert!(patched.contains(event), "missing event {event}");
        }
        assert!(patched.contains("zedra-agent-hooks.sh"));
        // Non-hooks keys must be preserved verbatim.
        assert!(patched.contains("model:\n  default: gpt-5\n"));
        assert!(patched.contains("hooks_auto_accept: false\n"));
    }

    #[test]
    fn hermes_hook_script_preserves_quoted_binary_path() {
        let script = HermesActor::hook_script_contents("/tmp/zedra build/zedra");
        assert!(script.contains("CLI=\"${ZEDRA_CLI:-}\""));
        assert!(script.contains("[ -n \"$CLI\" ] || CLI='/tmp/zedra build/zedra'"));
        assert!(!script.contains("CLI=\"${ZEDRA_CLI:-'"));
    }

    #[test]
    fn patch_hermes_hooks_idempotent_on_rerun() {
        let config = "hooks: {}\n";
        let script = std::path::Path::new("/path/to/zedra-agent-hooks.sh");
        let first = HermesActor::patch_config_hooks(config, script);
        let second = HermesActor::patch_config_hooks(&first, script);
        // Re-running must produce the same output (same set of event keys,
        // no duplicates, script path preserved).
        assert_eq!(first, second);
    }

    #[test]
    fn patch_hermes_hooks_overrides_old_script_path() {
        let old_script = std::path::Path::new("/old/path/zedra-agent-hooks.sh");
        let new_script = std::path::Path::new("/new/path/zedra-agent-hooks.sh");
        let after_first = HermesActor::patch_config_hooks("hooks: {}\n", old_script);
        assert!(after_first.contains("/old/path/"));
        let after_second = HermesActor::patch_config_hooks(&after_first, new_script);
        // Old path gone, new path present.
        assert!(
            !after_second.contains("/old/path/"),
            "old path still present"
        );
        assert!(after_second.contains("/new/path/"));
        for event in HermesActor::HOOK_EVENTS {
            let count = after_second.matches(event).count();
            assert_eq!(count, 1, "event {event} appears {count} times (want 1)");
        }
    }

    #[test]
    fn patch_hermes_hooks_preserves_user_events() {
        let config = "hooks:\n  user_event:\n    - command: user_script.sh\n      timeout: 10\n";
        let script = std::path::Path::new("/path/zedra-agent-hooks.sh");
        let patched = HermesActor::patch_config_hooks(config, script);
        assert!(patched.contains("user_event:"), "user event must survive");
        assert!(
            patched.contains("user_script.sh"),
            "user script must survive"
        );
        for event in HermesActor::HOOK_EVENTS {
            assert!(patched.contains(event), "missing zedra event {event}");
        }
    }

    #[test]
    fn patch_hermes_hooks_no_hooks_key_appends_block() {
        let config = "model:\n  default: gpt-5\n";
        let script = std::path::Path::new("/path/zedra-agent-hooks.sh");
        let patched = HermesActor::patch_config_hooks(config, script);
        assert!(patched.starts_with("model:"));
        assert!(patched.contains("hooks:\n"));
        for event in HermesActor::HOOK_EVENTS {
            assert!(patched.contains(event));
        }
    }
}
