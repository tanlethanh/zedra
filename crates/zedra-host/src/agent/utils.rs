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

/// Run a blocking `Option`-returning probe on the blocking pool, mapping a join
/// error to `None`. Shared by the agent actors' async account/usage methods.
pub fn spawn_blocking_opt<T, F>(probe: F) -> super::ActorFuture<'static, Option<T>>
where
    T: Send + 'static,
    F: FnOnce() -> Option<T> + Send + 'static,
{
    Box::pin(async move {
        match tokio::task::spawn_blocking(probe).await {
            Ok(result) => result,
            Err(err) => {
                // A join error is a panic/shutdown, not "no data" — surface it.
                tracing::info!("[debug:agent] blocking probe join failed: {err}");
                None
            }
        }
    })
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

/// `.jsonl` files in `dir` newest-first by (mtime, path) — a cheap recency
/// proxy that avoids opening files to sort. Empty when `dir` is absent.
pub fn sorted_jsonl_candidates(dir: &Path) -> anyhow::Result<Vec<(PathBuf, Option<u64>)>> {
    use anyhow::Context;
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => {
            return Err(err).with_context(|| format!("failed to read {}", dir.display()));
        }
    };
    let mut candidates: Vec<(PathBuf, Option<u64>)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            continue;
        }
        let mtime = mtime_unix_secs(&path);
        candidates.push((path, mtime));
    }
    candidates.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.0.cmp(&a.0)));
    Ok(candidates)
}

/// First non-empty string field on `record` matching any of `names`, in order.
pub fn string_field<'a>(record: &'a Value, names: &[&str]) -> Option<&'a str> {
    names
        .iter()
        .find_map(|name| record.get(*name)?.as_str())
        .filter(|value| !value.is_empty())
}

/// Trimmed text of a `user`-role chat `message`, or `None`. Accepts both content shapes
/// (bare string, or array where the first `type == "text"` part wins) and rejects empty or
/// markup-only (leading `<`) text so it is title-safe. Pass the message object itself (JSONL
/// record or `messages` element), not the envelope.
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

/// Anti-abuse payload clamp on stored titles, not a display limit (the client trims to row width).
/// Applies to every agent — all route titles through [`session_title`].
pub const SESSION_TITLE_MAX_CHARS: usize = 200;

pub fn session_title(stored: Option<String>) -> Option<String> {
    stored
        .map(|title| title.trim().to_string())
        .filter(|title| !title.is_empty())
        .map(|title| crate::utils::truncate_chars(&title, SESSION_TITLE_MAX_CHARS))
        .or_else(|| Some("Unknown".to_string()))
}

pub fn resume_summary(slug: &str, session_id: &str) -> AgentResumeSummary {
    // Trim once so availability and the `slug:id` payload agree.
    let session_id = session_id.trim();
    let available = !session_id.is_empty();
    AgentResumeSummary {
        available,
        unavailable_reason: (!available).then(|| "missing session id".to_string()),
        action_id: available.then(|| format!("{slug}:{session_id}")),
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

/// Lowercase Claude plan tier/phrase -> display label; shared by the
/// credentials and CLI-login probes so their tier lists never drift.
pub fn plan_label_from_token(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    [
        ("enterprise", "Enterprise"),
        ("ultra", "Ultra"),
        ("max", "Max"),
        ("team", "Team"),
        ("pro", "Pro"),
    ]
    .into_iter()
    .find(|(needle, _)| lower.contains(needle))
    .map(|(_, label)| label.to_string())
}

/// Extract a non-empty string value from a JSON object by key.
pub fn payload_string(payload: &Value, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
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

// ---------------------------------------------------------------------------
// Hook-config building and checked file writes (shared by actor hook writers)
// ---------------------------------------------------------------------------

fn hook_groups(command: &str, matcher: Option<&str>) -> serde_json::Value {
    let mut group = serde_json::json!({
        "hooks": [{
            "type": "command",
            "command": command,
            "timeout": 5
        }]
    });
    if let Some(matcher) = matcher {
        group["matcher"] = serde_json::Value::String(matcher.to_string());
    }
    serde_json::Value::Array(vec![group])
}

/// Builds a workdir hook-config JSON (`{"hooks": {...}}`) from an actor's
/// event table, one `hook_groups_for_event` entry per event.
pub fn hook_config_from_events(
    script_path: &Path,
    slug: &str,
    events: &[(&str, Option<&str>, u64)],
) -> serde_json::Value {
    let mut hooks = serde_json::Map::new();
    for &(event, matcher, _timeout) in events {
        hooks.insert(
            event.to_string(),
            hook_groups_for_event(script_path, slug, event, matcher),
        );
    }
    serde_json::json!({ "hooks": hooks })
}

pub fn hook_groups_for_event(
    script_path: &Path,
    slug: &str,
    event_name: &str,
    matcher: Option<&str>,
) -> serde_json::Value {
    hook_groups(&hook_command(script_path, slug, event_name), matcher)
}

fn hook_command(script_path: &Path, slug: &str, event_expr: &str) -> String {
    format!(
        "ZEDRA_AGENT_KIND={} ZEDRA_AGENT_EVENT={} {}",
        slug,
        event_expr,
        crate::utils::shell_arg_path(script_path)
    )
}

pub fn write_json_file_checked(
    path: &Path,
    value: &serde_json::Value,
    force: bool,
    label: &str,
) -> anyhow::Result<()> {
    let mut contents = serde_json::to_string_pretty(value)?;
    contents.push('\n');
    write_file_checked(path, &contents, force, label)
}

pub fn write_file_checked(
    path: &Path,
    contents: &str,
    force: bool,
    label: &str,
) -> anyhow::Result<()> {
    use anyhow::Context;
    if path.exists() && !force {
        anyhow::bail!(
            "{label} already exists at {}. Re-run with --force to overwrite it.",
            path.display()
        );
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    std::fs::write(path, contents)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
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

    #[test]
    fn hook_command_uses_provider_env() {
        let command = hook_command(Path::new("/tmp/zedra hook.sh"), "claude", "Stop");
        assert!(command.contains("ZEDRA_AGENT_KIND=claude"));
        assert!(command.contains("ZEDRA_AGENT_EVENT=Stop"));
        assert!(command.contains("'/tmp/zedra hook.sh'"));
    }
}
