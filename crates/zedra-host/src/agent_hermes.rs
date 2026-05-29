//! Hermes is a global personal agent: sessions are not scoped to a workspace
//! (flat `~/.hermes/sessions/*.json`, no `cwd`), so every scan ignores the
//! workdir and returns all sessions. Beyond sessions we surface Hermes's
//! config/memory layer (config.yaml, auth.json, SOUL.md, USER.md, MEMORY.md,
//! .env) read-only via [`config_files`].

use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDateTime, Utc};
use serde_json::Value;
use zedra_rpc::proto::*;

use crate::agent_utils::{
    command_on_path, empty_session_live, file_size_bytes, home_path, info_field, mtime_unix_secs,
    read_json_file, resume_summary, session_title, user_message_text,
};

/// Cap per-file content shipped to the client. Hermes `.env` and memory files
/// are small, but the cap bounds a pathological file from bloating the reply.
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
    model: Option<String>,
    model_provider: Option<String>,
    /// Platform the session ran on (cli, acp, telegram, discord, …).
    source: Option<String>,
    message_count: u64,
    tool_count: u64,
    cost_usd: Option<f64>,
    created_at: Option<DateTime<Utc>>,
    last_activity_at: Option<DateTime<Utc>>,
    /// JSON transcript path when scanned from disk; None for db-only sessions.
    transcript_path: Option<PathBuf>,
}

pub fn cli_available() -> bool {
    command_on_path("hermes") || sessions_dir().is_dir()
}

pub fn normalize_event(_event_name: &str) -> Option<(AgentEventKind, AgentLifecycleStatus)> {
    None
}

/// Sessions are global, so `workdir` is intentionally ignored.
pub fn session_counts(_workdir: &Path) -> Result<SessionCounts, String> {
    let total = count_sessions();
    let latest = collect_sessions(Some(1)).into_iter().next();
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
    let total = count_sessions();
    let summaries = collect_sessions(Some(limit))
        .iter()
        .map(|s| session_summary(s, cli))
        .collect();
    Ok((summaries, total))
}

pub fn account_fields() -> Vec<AgentInfoField> {
    let mut fields = Vec::new();
    let (active, providers) = read_auth();
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
    let cfg = read_config();
    if let Some(model) = cfg.default_model {
        fields.push(info_field("Default model", &model));
    }
    if let Some(provider) = cfg.default_provider {
        fields.push(info_field("Default provider", &provider));
    }
    let skills = skill_count();
    if skills > 0 {
        fields.push(info_field("Skills", &skills.to_string()));
    }
    append_rollups(&mut fields);
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
    walk(&hermes_home().join("skills"), 0)
}

/// Account-level rollups from `state.db`: lifetime spend, session count, and the
/// platforms Hermes has run on. Skipped silently when the db is absent.
fn append_rollups(fields: &mut Vec<AgentInfoField>) {
    let db = state_db_path();
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
    config_files_in(&hermes_home())
}

fn config_files_in(home: &Path) -> Vec<AgentFile> {
    // `label` doubles as the relative path under HERMES_HOME.
    const VIEW_FILES: &[&str] = &[
        "SOUL.md",
        "USER.md",
        "MEMORY.md",
        "config.yaml",
        ".env",
        "cron/jobs.json",
    ];
    VIEW_FILES
        .iter()
        .map(|name| read_view_file(name, &home.join(name)))
        .collect()
}

// ---------------------------------------------------------------------------
// Session scan (global)
// ---------------------------------------------------------------------------

fn hermes_home() -> PathBuf {
    std::env::var_os("HERMES_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| home_path(&[".hermes"]))
}

fn sessions_dir() -> PathBuf {
    hermes_home().join("sessions")
}

fn state_db_path() -> PathBuf {
    hermes_home().join("state.db")
}

fn count_sessions() -> usize {
    if let Some(count) = count_sessions_db() {
        return count;
    }
    let Ok(entries) = std::fs::read_dir(sessions_dir()) else {
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
    if let Some(sessions) = collect_sessions_from_db(limit) {
        return sessions;
    }
    collect_sessions_from_json(limit)
}

fn collect_sessions_from_json(limit: Option<usize>) -> Vec<HermesSession> {
    let Ok(entries) = std::fs::read_dir(sessions_dir()) else {
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
    // Newest first; mtime is the cheap proxy, refined by last_updated per file.
    candidates.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.0.cmp(&a.0)));

    let take = limit.unwrap_or(candidates.len());
    candidates
        .into_iter()
        .take(take)
        .filter_map(|(path, mtime)| read_session(&path, mtime))
        .collect()
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
    let model = value
        .get("model")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let message_count = value
        .get("message_count")
        .and_then(Value::as_u64)
        .or_else(|| {
            value
                .get("messages")
                .and_then(Value::as_array)
                .map(|m| m.len() as u64)
        })
        .unwrap_or(0);
    let created_at = parse_timestamp(value.get("session_start"));
    let last_activity_at = parse_timestamp(value.get("last_updated"))
        .or_else(|| mtime_secs.and_then(|s| DateTime::<Utc>::from_timestamp(s as i64, 0)));
    let title = first_user_title(&value);

    Some(HermesSession {
        session_id,
        title,
        model,
        message_count,
        created_at,
        last_activity_at,
        transcript_path: Some(path.to_path_buf()),
        ..Default::default()
    })
}

// ---------------------------------------------------------------------------
// state.db (canonical session store)
// ---------------------------------------------------------------------------

struct DbRow {
    id: String,
    title: Option<String>,
    model: Option<String>,
    billing_provider: Option<String>,
    source: Option<String>,
    message_count: i64,
    tool_count: i64,
    cost: Option<f64>,
    started_at: Option<f64>,
    ended_at: Option<f64>,
}

fn count_sessions_db() -> Option<usize> {
    let db = state_db_path();
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
    let db = state_db_path();
    if !db.is_file() {
        return None;
    }
    let conn = crate::sqlite_readonly::open(&db).ok()?;
    let limit_sql = limit.map(|l| l as i64).unwrap_or(-1); // -1 = unbounded in SQLite
    let rows: Vec<DbRow> = {
        let mut stmt = conn
            .prepare(
                "SELECT id, title, model, billing_provider, source, message_count, \
                 tool_call_count, COALESCE(actual_cost_usd, estimated_cost_usd), \
                 started_at, ended_at \
                 FROM sessions ORDER BY started_at DESC LIMIT ?1",
            )
            .ok()?;
        let mapped = stmt
            .query_map([limit_sql], |row| {
                Ok(DbRow {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    model: row.get(2)?,
                    billing_provider: row.get(3)?,
                    source: row.get(4)?,
                    message_count: row.get::<_, Option<i64>>(5)?.unwrap_or(0),
                    tool_count: row.get::<_, Option<i64>>(6)?.unwrap_or(0),
                    cost: row.get(7)?,
                    started_at: row.get(8)?,
                    ended_at: row.get(9)?,
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
                .or_else(|| first_user_message_db(&conn, &r.id));
            HermesSession {
                session_id: r.id,
                title,
                model: r.model.filter(|s| !s.is_empty()),
                model_provider: r.billing_provider.filter(|s| !s.is_empty()),
                source: r.source.filter(|s| !s.is_empty()),
                message_count: r.message_count.max(0) as u64,
                tool_count: r.tool_count.max(0) as u64,
                cost_usd: r.cost,
                created_at: r.started_at.and_then(epoch_to_dt),
                last_activity_at: r.ended_at.or(r.started_at).and_then(epoch_to_dt),
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

fn session_summary(session: &HermesSession, cli: &AgentCliSummary) -> AgentSessionSummary {
    AgentSessionSummary {
        kind: ManagedAgentKind::Hermes,
        session_id: session.session_id.clone(),
        title: session_title(session.title.clone()),
        cwd: None,
        created_at: session.created_at,
        last_activity_at: session.last_activity_at,
        resume: resume_summary(ManagedAgentKind::Hermes, &session.session_id),
        live: empty_session_live(),
        provider: AgentProviderSessionInfo {
            model: session.model.clone(),
            permission_mode: None,
            cli_version: cli.version.clone(),
            origin: None,
            // The platform the session ran on (cli / acp / telegram / …).
            source: session.source.clone(),
            entrypoint: None,
            native_project_id: None,
            model_provider: session.model_provider.clone(),
        },
        git: None,
        // Per-session spend, when the host tracked a cost.
        usage: session
            .cost_usd
            .filter(|c| *c > 0.0)
            .map(|cost| AgentUsageSnapshot {
                total_cost_usd: Some(cost),
                ..Default::default()
            }),
        counters: AgentSessionCounters {
            record_count: session.message_count,
            message_count: session.message_count,
            turn_count: 0,
            tool_count: session.tool_count,
            tool_failure_count: 0,
            hook_success_count: 0,
            hook_failure_count: 0,
            malformed_record_count: 0,
        },
        flags: AgentSessionFlags {
            is_sidechain: false,
            is_subagent: false,
            is_archived: false,
            historical_only: true,
            live_bound: false,
        },
        data_sources: vec![AgentDataSource::HistoricalScan],
        warnings: Vec::new(),
        transcript_size_bytes: session.transcript_path.as_deref().and_then(file_size_bytes),
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

/// Hermes `config.yaml` nests model defaults under a top-level `model:` block.
/// We only need a couple of scalars, so parse the block by indentation rather
/// than pulling in a YAML dependency (mirrors how the Codex agent reads its
/// `config.toml` line by line).
fn read_config() -> HermesConfig {
    let Ok(text) = std::fs::read_to_string(hermes_home().join("config.yaml")) else {
        return HermesConfig::default();
    };
    parse_config(&text)
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
                cfg.default_model = yaml_scalar(v);
            } else if let Some(v) = trimmed.strip_prefix("provider:") {
                cfg.default_provider = yaml_scalar(v);
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
    let Ok(value) = read_json_file(&hermes_home().join("auth.json")) else {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_model_block_from_config_yaml() {
        let cfg = parse_config(
            "model:\n  default: gpt-5.5\n  provider: openai-codex\n  base_url: https://x\nproviders: {}\n",
        );
        assert_eq!(cfg.default_model.as_deref(), Some("gpt-5.5"));
        assert_eq!(cfg.default_provider.as_deref(), Some("openai-codex"));
    }

    #[test]
    fn parse_config_ignores_keys_outside_model_block() {
        // `provider:` under a sibling top-level block must not leak into model.
        let cfg = parse_config("server:\n  provider: bogus\nmodel:\n  default: m1\n");
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
            first_user_title(&string_form).as_deref(),
            Some("Summarize the docs")
        );

        let array_form = serde_json::json!({
            "messages": [
                {"role": "user", "content": [{"type": "text", "text": "hello world"}]}
            ]
        });
        assert_eq!(
            first_user_title(&array_form).as_deref(),
            Some("hello world")
        );
    }

    #[test]
    fn config_files_marks_missing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("SOUL.md"), "persona").unwrap();
        let files = config_files_in(dir.path());
        let soul = files.iter().find(|f| f.label == "SOUL.md").unwrap();
        assert!(!soul.missing);
        assert_eq!(soul.content, "persona");
        let user = files.iter().find(|f| f.label == "USER.md").unwrap();
        assert!(user.missing);
    }
}
