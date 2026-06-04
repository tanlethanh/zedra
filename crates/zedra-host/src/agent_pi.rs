use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;
use zedra_rpc::proto::*;

use crate::agent_utils::{
    command_on_path, empty_session_live, file_size_bytes, home_path, info_field, mtime_unix_secs,
    parse_rfc3339, read_json_file, resume_summary, session_title, string_field, user_message_text,
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
    // Full-scan: session summaries surface message/malformed counters and the
    // latest activity timestamp, which a head-only scan would underreport.
    let files = collect_session_files(workdir, Some(limit), false).map_err(|e| e.to_string())?;
    let summaries = files
        .iter()
        .map(|file| session_summary(file, cli))
        .collect();
    Ok((summaries, total))
}

pub fn account_fields(workdir: &Path) -> Vec<AgentInfoField> {
    let mut fields = Vec::new();
    let (settings, has_project) = effective_settings(workdir);
    let providers = provider_fields();

    // Per-provider auth replaces a single "Logged in" boolean: Pi is multi-provider.
    if providers.is_empty() {
        fields.push(info_field("Logged in", "no"));
    } else {
        fields.extend(providers);
    }

    if let Some(model) = &settings.default_model {
        fields.push(info_field("Default model", model));
    }
    if let Some(provider) = &settings.default_provider {
        fields.push(info_field("Default provider", provider));
    }
    if let Some(level) = &settings.default_thinking_level {
        fields.push(info_field("Thinking", level));
    }
    if let Some(value) = extensions_value(&settings) {
        fields.push(info_field("Extensions", &value));
    }
    fields.push(info_field(
        "Project config",
        if has_project { "yes" } else { "no" },
    ));
    fields
}

pub fn subscription_plan_fields() -> Option<Vec<AgentInfoField>> {
    // Pi has no remote plan to fetch: provider/auth state is local and already
    // populated synchronously by `account_fields`. The async plan-refresh path
    // would only re-read the same files and merge identical rows back, so opt
    // out instead of duplicating that work.
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
                    created_at = parse_rfc3339(string_field(&record, &["timestamp"]));
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
                    // Length is clamped centrally in `session_title`; the UI trims
                    // for display.
                    title = first_user_text(&record);
                }
            }
            _ => {}
        }

        if let Some(ts) = parse_rfc3339(string_field(&record, &["timestamp"])) {
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

    // A head-only scan stops after the first few records, so its newest scanned
    // timestamp reflects the opening prompt rather than the latest turn. Trust
    // file mtime for activity there; full scans use the real last timestamp.
    let mtime_activity =
        mtime_unix_secs.and_then(|secs| DateTime::<Utc>::from_timestamp(secs as i64, 0));
    let last_activity_at = if head_only {
        mtime_activity.or(last_timestamp)
    } else {
        last_timestamp.or(mtime_activity)
    };

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
        kind: AgentKind::Pi,
        session_id: file.session_id.clone(),
        title: session_title(file.title.clone()),
        cwd: file.cwd.clone(),
        created_at: file.created_at,
        last_activity_at: file.last_activity_at,
        resume: resume_summary(AgentKind::Pi, &file.session_id),
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
// Config / auth (account info)
// ---------------------------------------------------------------------------

fn pi_agent_dir() -> PathBuf {
    home_path(&[".pi", "agent"])
}

/// Subset of Pi's `settings.json` we surface. Pi stores far more, but the agent
/// info panel only needs the model defaults and the extensibility counts.
#[derive(Default, serde::Deserialize)]
struct PiSettings {
    #[serde(rename = "defaultModel")]
    default_model: Option<String>,
    #[serde(rename = "defaultProvider")]
    default_provider: Option<String>,
    #[serde(rename = "defaultThinkingLevel")]
    default_thinking_level: Option<String>,
    /// npm/git package sources (string or `{ source, .. }`).
    packages: Option<Vec<Value>>,
    /// Local extension file/dir paths.
    extensions: Option<Vec<String>>,
}

fn read_pi_settings(path: &Path) -> PiSettings {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

/// Effective config = global (`~/.pi/agent`) overlaid with project
/// (`<workdir>/.pi`); per field the project value replaces the global one
/// (scalars and lists alike), matching Pi's own settings merge. Returns the
/// merged view plus whether the workspace has its own `settings.json`.
fn effective_settings(workdir: &Path) -> (PiSettings, bool) {
    let global = read_pi_settings(&pi_agent_dir().join("settings.json"));
    let project_path = workdir.join(".pi").join("settings.json");
    let has_project = project_path.is_file();
    let project = read_pi_settings(&project_path);
    let merged = PiSettings {
        default_model: project.default_model.or(global.default_model),
        default_provider: project.default_provider.or(global.default_provider),
        default_thinking_level: project
            .default_thinking_level
            .or(global.default_thinking_level),
        packages: project.packages.or(global.packages),
        extensions: project.extensions.or(global.extensions),
    };
    (merged, has_project)
}

fn extensions_value(settings: &PiSettings) -> Option<String> {
    let packages = settings.packages.as_deref().unwrap_or(&[]);
    let locals = settings.extensions.as_deref().map(<[_]>::len).unwrap_or(0);
    let total = packages.len() + locals;
    if total == 0 {
        return None;
    }
    // Show the first package source as a hint; "configured" is honest because we
    // read settings statically and never resolve the package into its resources.
    match packages.iter().find_map(package_source) {
        Some(src) if total == 1 => Some(src),
        Some(src) => Some(format!("{total} configured ({src}, …)")),
        None => Some(format!("{total} configured")),
    }
}

fn package_source(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::to_string)
        .or_else(|| string_field(value, &["source"]).map(str::to_string))
}

#[derive(serde::Deserialize)]
struct PiAuthEntry {
    #[serde(rename = "type")]
    kind: String,
    /// OAuth access-token expiry, Unix milliseconds.
    expires: Option<i64>,
}

/// Authenticated providers from `auth.json` plus custom providers declared in
/// `models.json`, formatted as `{provider → auth state}` info rows. Never
/// exposes tokens or account ids.
fn provider_fields() -> Vec<AgentInfoField> {
    let mut fields = Vec::new();
    if let Ok(Value::Object(map)) = read_json_file(&pi_agent_dir().join("auth.json")) {
        let mut entries: Vec<_> = map.into_iter().collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        for (provider, value) in entries {
            if let Ok(entry) = serde_json::from_value::<PiAuthEntry>(value) {
                fields.push(info_field(&provider, &auth_value(&entry)));
            }
        }
    }
    for (id, models) in custom_providers() {
        fields.push(info_field(&id, &format!("custom · {models} models")));
    }
    fields
}

fn auth_value(entry: &PiAuthEntry) -> String {
    match entry.kind.as_str() {
        "oauth" => match entry.expires {
            Some(expires_ms) => {
                let remaining_ms = expires_ms - Utc::now().timestamp_millis();
                if remaining_ms <= 0 {
                    "OAuth · expired".to_string()
                } else {
                    format!("OAuth · expires in {}", humanize_secs(remaining_ms / 1000))
                }
            }
            None => "OAuth".to_string(),
        },
        "api_key" => "API key".to_string(),
        other => other.to_string(),
    }
}

fn humanize_secs(secs: i64) -> String {
    if secs >= 86_400 {
        format!("{}d", secs / 86_400)
    } else if secs >= 3_600 {
        format!("{}h", secs / 3_600)
    } else {
        format!("{}m", (secs / 60).max(1))
    }
}

/// Custom provider ids (e.g. `ollama`) from `models.json` and their model count.
fn custom_providers() -> Vec<(String, usize)> {
    let Ok(file) = read_json_file(&pi_agent_dir().join("models.json")) else {
        return Vec::new();
    };
    let Some(providers) = file.get("providers").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut out: Vec<(String, usize)> = providers
        .iter()
        .map(|(id, cfg)| {
            let count = cfg
                .get("models")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            (id.clone(), count)
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn first_user_text(record: &Value) -> Option<String> {
    user_message_text(record.get("message").unwrap_or(record))
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
    fn humanize_secs_picks_coarsest_unit() {
        assert_eq!(humanize_secs(9 * 86_400 + 5), "9d");
        assert_eq!(humanize_secs(5 * 3_600), "5h");
        assert_eq!(humanize_secs(120), "2m");
        assert_eq!(humanize_secs(10), "1m"); // never "0m"
    }

    #[test]
    fn auth_value_covers_oauth_and_api_key() {
        let future = Utc::now().timestamp_millis() + 3 * 86_400 * 1000;
        assert!(auth_value(&PiAuthEntry {
            kind: "oauth".into(),
            expires: Some(future),
        })
        .starts_with("OAuth · expires in"));
        assert_eq!(
            auth_value(&PiAuthEntry {
                kind: "oauth".into(),
                expires: Some(0),
            }),
            "OAuth · expired"
        );
        assert_eq!(
            auth_value(&PiAuthEntry {
                kind: "oauth".into(),
                expires: None,
            }),
            "OAuth"
        );
        assert_eq!(
            auth_value(&PiAuthEntry {
                kind: "api_key".into(),
                expires: None,
            }),
            "API key"
        );
    }

    #[test]
    fn package_source_reads_string_and_object_forms() {
        assert_eq!(
            package_source(&serde_json::json!("npm:foo")).as_deref(),
            Some("npm:foo")
        );
        assert_eq!(
            package_source(&serde_json::json!({ "source": "git:bar", "skills": [] })).as_deref(),
            Some("git:bar")
        );
        assert_eq!(package_source(&serde_json::json!({ "skills": [] })), None);
    }

    #[test]
    fn extensions_value_counts_packages_and_locals() {
        let none = PiSettings::default();
        assert_eq!(extensions_value(&none), None);

        let single = PiSettings {
            packages: Some(vec![serde_json::json!("npm:web-search")]),
            ..Default::default()
        };
        assert_eq!(extensions_value(&single).as_deref(), Some("npm:web-search"));

        let many = PiSettings {
            packages: Some(vec![serde_json::json!("npm:a"), serde_json::json!("npm:b")]),
            extensions: Some(vec!["/local/ext".to_string()]),
            ..Default::default()
        };
        assert_eq!(
            extensions_value(&many).as_deref(),
            Some("3 configured (npm:a, …)")
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

    #[test]
    fn full_scan_reaches_latest_activity_and_counts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("2026-05-28_long.jsonl");
        let mut lines = String::from(
            r#"{"type":"session","id":"long","timestamp":"2026-05-28T10:00:00Z","cwd":"/p"}
{"type":"message","id":"u","message":{"role":"user","content":[{"type":"text","text":"start"}]}}
"#,
        );
        // Push well past LIST_HEAD_SCAN_MAX_LINES so head-only would stop early.
        for i in 0..(LIST_HEAD_SCAN_MAX_LINES + 10) {
            lines.push_str(&format!(
                "{{\"type\":\"message\",\"id\":\"m{i}\",\"timestamp\":\"2026-05-28T12:00:{:02}Z\",\"message\":{{\"role\":\"assistant\",\"content\":[]}}}}\n",
                i % 60
            ));
        }
        std::fs::write(&path, lines).unwrap();

        // Head-only: timestamps are unreliable, so fall back to mtime; counters
        // are partial.
        let head = read_session_file(&path, Some(1_700_000_000), true).unwrap();
        assert_eq!(
            head.last_activity_at,
            DateTime::<Utc>::from_timestamp(1_700_000_000, 0)
        );
        assert!(head.message_count < (LIST_HEAD_SCAN_MAX_LINES as u64));

        // Full scan: sees the latest turn timestamp and every message.
        let full = read_session_file(&path, Some(1_700_000_000), false).unwrap();
        assert_eq!(full.message_count, (LIST_HEAD_SCAN_MAX_LINES + 11) as u64);
        assert!(full.last_activity_at.unwrap() > head.last_activity_at.unwrap());
    }
}
