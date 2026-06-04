use chrono::{DateTime, Utc};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use zedra_rpc::proto::*;

pub fn home_path(parts: &[&str]) -> PathBuf {
    let mut path = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    for part in parts {
        path.push(part);
    }
    path
}

pub fn command_on_path(program: &str) -> bool {
    if program.contains('/') {
        return Path::new(program).is_file();
    }
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join(program).is_file())
}

pub fn first_non_empty_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

pub fn parse_rfc3339(value: Option<&str>) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value?)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

pub fn file_size_bytes(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().map(|meta| meta.len())
}

pub fn mtime_unix_secs(path: &Path) -> Option<u64> {
    std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

/// First non-empty string field on `record` matching any of `names`, in order.
pub fn string_field<'a>(record: &'a Value, names: &[&str]) -> Option<&'a str> {
    names
        .iter()
        .find_map(|name| record.get(*name)?.as_str())
        .filter(|value| !value.is_empty())
}

/// Trimmed text of a chat `message` when its role is `user`, or `None`.
///
/// Handles both content shapes agents emit: a bare string, or an array of
/// content parts where the first `type == "text"` part wins. Markup-only text
/// (leading `<`) and empty text are rejected so it is suitable as a session
/// title source. Callers pass the message object itself (the JSONL record or an
/// element of a `messages` array), not the enclosing envelope.
pub fn user_message_text(message: &Value) -> Option<String> {
    if string_field(message, &["role"]) != Some("user") {
        return None;
    }
    let content = message.get("content")?;
    let text = if let Some(text) = content.as_str() {
        text.to_string()
    } else {
        content.as_array()?.iter().find_map(|part| {
            (string_field(part, &["type"]) == Some("text"))
                .then(|| string_field(part, &["text"]))
                .flatten()
                .map(str::to_string)
        })?
    };
    let text = text.trim();
    (!text.is_empty() && !text.starts_with('<')).then(|| text.to_string())
}

pub fn info_field(label: &str, value: &str) -> AgentInfoField {
    AgentInfoField {
        label: label.to_string(),
        value: value.to_string(),
    }
}

pub fn cwd_matches(workdir: &Path, cwd: Option<&str>) -> bool {
    let Some(cwd) = cwd else {
        return false;
    };
    paths_equal(workdir, Path::new(cwd))
}

pub fn paths_equal(left: &Path, right: &Path) -> bool {
    if normalize_path(left) == normalize_path(right) {
        return true;
    }
    left.to_string_lossy().trim_end_matches('/') == right.to_string_lossy().trim_end_matches('/')
}

pub fn normalize_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

pub fn sql_string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

pub fn kind_slug(kind: ManagedAgentKind) -> &'static str {
    match kind {
        ManagedAgentKind::Claude => "claude",
        ManagedAgentKind::Codex => "codex",
        ManagedAgentKind::OpenCode => "opencode",
        ManagedAgentKind::Pi => "pi",
        ManagedAgentKind::Hermes => "hermes",
    }
}

pub fn program_name(kind: ManagedAgentKind) -> &'static str {
    match kind {
        ManagedAgentKind::Claude => "claude",
        ManagedAgentKind::Codex => "codex",
        ManagedAgentKind::OpenCode => "opencode",
        ManagedAgentKind::Pi => "pi",
        ManagedAgentKind::Hermes => "hermes",
    }
}

pub fn display_name(kind: ManagedAgentKind) -> &'static str {
    match kind {
        ManagedAgentKind::Claude => "Claude",
        ManagedAgentKind::Codex => "Codex",
        ManagedAgentKind::OpenCode => "OpenCode",
        ManagedAgentKind::Pi => "Pi",
        ManagedAgentKind::Hermes => "Hermes",
    }
}

pub fn capabilities(kind: ManagedAgentKind) -> AgentCapabilities {
    AgentCapabilities {
        list_sessions: true,
        resume_session: true,
        live_binding: true,
        confirm_action: true,
        select_action: true,
        lifecycle_events: true,
        usage_snapshot: matches!(kind, ManagedAgentKind::Claude),
    }
}

/// Generous upper bound on a stored session title. This is an anti-abuse clamp
/// on payload size, not a display limit — the client trims titles to the row
/// width at render time. Applies to every managed agent, since they all route
/// titles through [`session_title`].
pub const SESSION_TITLE_MAX_CHARS: usize = 200;

pub fn session_title(stored: Option<String>) -> Option<String> {
    stored
        .map(|title| title.trim().to_string())
        .filter(|title| !title.is_empty())
        .map(|title| crate::utils::truncate_chars(&title, SESSION_TITLE_MAX_CHARS))
        .or_else(|| Some("Unknown".to_string()))
}

pub fn resume_summary(kind: ManagedAgentKind, session_id: &str) -> AgentResumeSummary {
    let available = !session_id.trim().is_empty();
    AgentResumeSummary {
        available,
        unavailable_reason: (!available).then(|| "missing session id".to_string()),
        action_id: available.then(|| format!("{}:{session_id}", kind_slug(kind))),
    }
}

pub fn empty_session_live() -> AgentSessionLiveSummary {
    AgentSessionLiveSummary {
        terminal_id: None,
        status: AgentLifecycleStatus::Unknown,
        pending_action_count: 0,
        current_turn_id: None,
        latest_event: None,
    }
}

pub fn malformed_warning(count: usize) -> Vec<AgentWarning> {
    if count == 0 {
        Vec::new()
    } else {
        vec![AgentWarning {
            code: "malformed_records".to_string(),
            message: format!("{count} malformed records were ignored"),
        }]
    }
}

pub fn shell_quote(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || "_@%+=:,./-".contains(ch))
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

// ---------- git helpers ----------

pub fn share_git_repository(
    left: &Path,
    right: &Path,
    cache: &mut HashMap<PathBuf, Option<PathBuf>>,
) -> bool {
    let Some(left_common) = git_common_dir(left, cache) else {
        return false;
    };
    let Some(right_common) = git_common_dir(right, cache) else {
        return false;
    };
    left_common == right_common
}

pub fn git_common_dir(
    path: &Path,
    cache: &mut HashMap<PathBuf, Option<PathBuf>>,
) -> Option<PathBuf> {
    let key = normalize_path(path);
    if let Some(cached) = cache.get(&key) {
        return cached.clone();
    }
    let resolved = resolve_git_common_dir(path);
    cache.insert(key, resolved.clone());
    resolved
}

fn resolve_git_common_dir(path: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--git-common-dir"])
        .current_dir(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let relative = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if relative.is_empty() {
        return None;
    }
    let git_dir = PathBuf::from(&relative);
    let absolute = if git_dir.is_absolute() {
        git_dir
    } else {
        path.join(git_dir)
    };
    Some(normalize_path(&absolute))
}

pub fn git_branch_at(path: &Path, cache: &mut HashMap<PathBuf, Option<String>>) -> Option<String> {
    let key = normalize_path(path);
    if let Some(cached) = cache.get(&key) {
        return cached.clone();
    }
    let branch = resolve_git_branch(path);
    cache.insert(key, branch.clone());
    branch
}

fn resolve_git_branch(path: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(path)
        .output()
        .ok()?;
    if output.status.success() {
        let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !branch.is_empty() {
            return Some(branch);
        }
    }

    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!branch.is_empty() && branch != "HEAD").then_some(branch)
}

// ---------- JSON / config helpers ----------

pub fn read_json_file(path: &Path) -> Result<Value, String> {
    let contents =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    serde_json::from_str(&contents).map_err(|error| error.to_string())
}

pub fn push_json_string(
    fields: &mut Vec<AgentInfoField>,
    label: &str,
    value: &Value,
    path: &[&str],
) {
    let Some(raw) = json_path(value, path) else {
        return;
    };
    let Some(text) = value_to_string(raw) else {
        return;
    };
    fields.push(AgentInfoField {
        label: label.to_string(),
        value: text,
    });
}

#[allow(dead_code)]
pub fn push_json_u64(fields: &mut Vec<AgentInfoField>, label: &str, value: &Value, path: &[&str]) {
    let Some(raw) = json_path(value, path) else {
        return;
    };
    let Some(number) = raw.as_u64() else {
        return;
    };
    fields.push(AgentInfoField {
        label: label.to_string(),
        value: number.to_string(),
    });
}

#[allow(dead_code)]
pub fn push_json_bool(fields: &mut Vec<AgentInfoField>, label: &str, value: &Value, path: &[&str]) {
    let Some(raw) = json_path(value, path) else {
        return;
    };
    let Some(flag) = raw.as_bool() else {
        return;
    };
    fields.push(AgentInfoField {
        label: label.to_string(),
        value: if flag { "yes" } else { "no" }.to_string(),
    });
}

pub fn json_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    Some(current)
}

pub fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(if *flag { "yes" } else { "no" }.to_string()),
        _ => None,
    }
}

pub fn humanize_plan_token(raw: &str) -> String {
    let token = raw
        .trim()
        .trim_start_matches("claude_")
        .trim_start_matches("default_");
    if token.is_empty() {
        return raw.to_string();
    }
    let mut chars = token.chars();
    let Some(first) = chars.next() else {
        return raw.to_string();
    };
    let mut out = String::new();
    out.push(first.to_ascii_uppercase());
    for ch in chars {
        if ch == '_' {
            continue;
        }
        out.push(ch);
    }
    out
}

pub fn toml_value(line: &str) -> String {
    line.split_once('=')
        .map(|(_, value)| value.trim().trim_matches('"').to_string())
        .unwrap_or_else(|| line.to_string())
}

// ---------- provider usage API helpers ----------

/// Parse `resets_at` / `reset_at` from a provider usage window object.
pub fn parse_usage_window_resets_at(window: &Value) -> Option<i64> {
    let raw = window.get("resets_at").or_else(|| window.get("reset_at"))?;
    if let Some(secs) = raw.as_i64() {
        return Some(normalize_unix_seconds(secs));
    }
    if let Some(f) = raw.as_f64() {
        return Some(normalize_unix_seconds(f as i64));
    }
    let text = raw.as_str()?;
    if let Ok(dt) = DateTime::parse_from_rfc3339(text) {
        return Some(dt.timestamp());
    }
    DateTime::parse_from_str(text, "%+")
        .ok()
        .map(|dt| dt.timestamp())
}

fn normalize_unix_seconds(secs: i64) -> i64 {
    if secs > 1_000_000_000_000 {
        secs / 1000
    } else {
        secs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_usage_window_resets_at_accepts_unix_and_rfc3339() {
        let unix = serde_json::json!({ "resets_at": 1_700_000_000 });
        assert_eq!(parse_usage_window_resets_at(&unix), Some(1_700_000_000));
        let ms = serde_json::json!({ "resets_at": 1_700_000_000_000_i64 });
        assert_eq!(parse_usage_window_resets_at(&ms), Some(1_700_000_000));
        let rfc = serde_json::json!({ "resets_at": "2026-01-02T22:59:00Z" });
        assert_eq!(
            parse_usage_window_resets_at(&rfc),
            Some(
                DateTime::parse_from_rfc3339("2026-01-02T22:59:00Z")
                    .unwrap()
                    .timestamp()
            )
        );
    }
}
