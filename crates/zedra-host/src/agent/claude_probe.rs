//! Claude CLI PTY probe — best-effort usage/plan fallback when no OAuth token file is readable
//! (or OAuth calls fail). Spawns `claude`, drives `/usage` and `/status`, parses plain-text TUI
//! output (layout from codexbar `ClaudeStatusProbe`). Not a stable API: percents are usually
//! reliable, but `rate_limit_*_resets_at` may be missing or wrong (host-local time, glued tokens)
//! — treat missing resets as duration-unknown. One combined probe per cache window: `PTY_LOCK`
//! serializes, `PTY_CACHE` dedupes parallel usage/plan scans.
//! Env: `ZEDRA_DEBUG_CLAUDE_PTY=1`, `ZEDRA_CLAUDE_PTY_CACHE_SECS` (10–300, default 60).

use chrono::{
    DateTime, Datelike, Local, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, TimeZone,
};
use zedra_rpc::proto::{AgentInfoField, AgentUsageSnapshot};

const USAGE_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
const STATUS_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(12);
const READ_TICK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(800);
const USAGE_SETTLE: std::time::Duration = std::time::Duration::from_millis(2000);
const STATUS_SETTLE: std::time::Duration = std::time::Duration::from_millis(250);
const PTY_CACHE_TTL_DEFAULT_SECS: u64 = 60;

#[cfg(unix)]
struct ClaudePtySession {
    master_fd: libc::c_int,
    master: std::fs::File,
    child: std::process::Child,
}

#[cfg(unix)]
impl ClaudePtySession {
    fn spawn(claude_bin: &std::path::Path) -> Option<Self> {
        use std::os::unix::io::FromRawFd;
        use std::os::unix::process::CommandExt;

        let (master_fd, slave_fd) = unsafe {
            let mut master: libc::c_int = -1;
            let mut slave: libc::c_int = -1;
            let mut ws: libc::winsize = std::mem::zeroed();
            ws.ws_row = 50;
            ws.ws_col = 160;
            let rc = libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut ws as *mut libc::winsize,
            );
            if rc != 0 {
                tracing::debug!("claude cli pty: openpty failed");
                return None;
            }
            (master, slave)
        };

        let (slave_stdin, slave_stdout, slave_stderr) = unsafe {
            let out = libc::dup(slave_fd);
            let err = libc::dup(slave_fd);
            if out < 0 || err < 0 {
                if out >= 0 {
                    libc::close(out);
                }
                if err >= 0 {
                    libc::close(err);
                }
                libc::close(slave_fd);
                libc::close(master_fd);
                return None;
            }
            (slave_fd, out, err)
        };

        let mut cmd = std::process::Command::new(claude_bin);
        cmd.args(["--allowed-tools", ""]);
        unsafe {
            cmd.stdin(std::process::Stdio::from_raw_fd(slave_stdin))
                .stdout(std::process::Stdio::from_raw_fd(slave_stdout))
                .stderr(std::process::Stdio::from_raw_fd(slave_stderr))
                .pre_exec(|| {
                    libc::setsid();
                    libc::ioctl(0, libc::TIOCSCTTY as _, 0i32);
                    Ok(())
                });
        }

        let child = match cmd.spawn() {
            Ok(c) => c,
            Err(err) => {
                unsafe { libc::close(master_fd) };
                tracing::debug!("claude cli pty: spawn failed: {err}");
                return None;
            }
        };
        drop(cmd);

        let master = unsafe { std::fs::File::from_raw_fd(master_fd) };
        Some(Self {
            master_fd,
            master,
            child,
        })
    }

    fn drain_startup(&mut self) {
        use std::io::Read;

        let poll_readable = |timeout_ms: i32| -> bool {
            let mut pfd = libc::pollfd {
                fd: self.master_fd,
                events: libc::POLLIN,
                revents: 0,
            };
            unsafe { libc::poll(&mut pfd, 1, timeout_ms) > 0 && pfd.revents & libc::POLLIN != 0 }
        };

        let drain_deadline = std::time::Instant::now() + std::time::Duration::from_millis(2000);
        let mut buf = vec![0u8; 4096];
        while std::time::Instant::now() < drain_deadline {
            let ms = drain_deadline
                .saturating_duration_since(std::time::Instant::now())
                .as_millis()
                .min(200) as i32;
            if poll_readable(ms) {
                match self.master.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
        }
    }

    fn send_command(&mut self, command: &[u8]) -> bool {
        use std::io::Write;
        self.master.write_all(command).is_ok()
    }

    fn read_capture(
        &mut self,
        global_deadline: std::time::Instant,
        tick_interval: std::time::Duration,
        settle: std::time::Duration,
        palette_needles: &[&str],
        mut stop_when: impl FnMut(&str) -> bool,
    ) -> (String, bool) {
        use std::io::{Read, Write};

        let poll_readable = |timeout_ms: i32| -> bool {
            let mut pfd = libc::pollfd {
                fd: self.master_fd,
                events: libc::POLLIN,
                revents: 0,
            };
            unsafe { libc::poll(&mut pfd, 1, timeout_ms) > 0 && pfd.revents & libc::POLLIN != 0 }
        };

        let mut accumulated = String::new();
        let mut last_tick = std::time::Instant::now();
        let mut found_at: Option<std::time::Instant> = None;
        let mut norm = String::new();
        let mut buf = vec![0u8; 4096];
        let mut matched = false;

        loop {
            if std::time::Instant::now() >= global_deadline {
                break;
            }
            if let Some(found) = found_at {
                if found.elapsed() >= settle {
                    break;
                }
            }
            if last_tick.elapsed() >= tick_interval {
                let _ = self.master.write_all(b"\r");
                last_tick = std::time::Instant::now();
            }

            if poll_readable(100) {
                match self.master.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let chunk = strip_ansi(&buf[..n]);
                        accumulated.push_str(&chunk);
                        let new_norm: String = chunk
                            .chars()
                            .filter(|c| !c.is_whitespace())
                            .map(|c| c.to_ascii_lowercase())
                            .collect();
                        norm.push_str(&new_norm);

                        for needle in palette_needles {
                            if norm.contains(needle) {
                                let _ = self.master.write_all(b"\r");
                            }
                        }

                        if found_at.is_none() && stop_when(&norm) {
                            matched = true;
                            found_at = Some(std::time::Instant::now());
                        }
                    }
                }
            }
        }

        (accumulated, matched)
    }

    fn finish(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(unix)]
fn usage_panel_ready(norm: &str) -> bool {
    norm.contains("currentsession")
        && norm.contains('%')
        && norm.chars().any(|c| c.is_ascii_digit())
}

#[cfg(unix)]
fn status_panel_ready(norm: &str) -> bool {
    norm.contains("loginmethod")
        || (norm.contains("account") && norm.contains('@'))
        || norm.contains("organization:")
        || norm.contains("org:")
}

#[cfg(unix)]
const USAGE_PALETTE: &[&str] = &["showplan", "showplanusagelimits"];
#[cfg(unix)]
const STATUS_PALETTE: &[&str] = &["showclaudecode", "showclaudecodestatus"];

/// One Claude CLI PTY per process at a time; `scan_account_usage` and `scan_account_plans` run in parallel.
#[cfg(unix)]
static PTY_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(unix)]
struct PtyCache {
    fetched_at: std::time::Instant,
    usage: Option<AgentUsageSnapshot>,
    plan: Option<Vec<AgentInfoField>>,
}

#[cfg(unix)]
static PTY_CACHE: std::sync::Mutex<Option<PtyCache>> = std::sync::Mutex::new(None);

#[cfg(unix)]
fn with_pty_lock<T>(run: impl FnOnce() -> T) -> T {
    let _guard = PTY_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    run()
}

#[cfg(unix)]
fn pty_cache_ttl() -> std::time::Duration {
    let secs = std::env::var("ZEDRA_CLAUDE_PTY_CACHE_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(PTY_CACHE_TTL_DEFAULT_SECS)
        .clamp(10, 300);
    std::time::Duration::from_secs(secs)
}

#[cfg(unix)]
fn probe_cached(
    claude_bin: &std::path::Path,
) -> (Option<AgentUsageSnapshot>, Option<Vec<AgentInfoField>>) {
    if let Ok(cache) = PTY_CACHE.lock() {
        if let Some(entry) = cache.as_ref() {
            if entry.fetched_at.elapsed() < pty_cache_ttl() {
                let plan = plan_with_usage_fallback(entry.plan.clone(), &entry.usage);
                if entry.plan.is_some() || entry.usage.is_some() {
                    return (entry.usage.clone(), plan);
                }
            }
        }
    }

    let (usage, plan) = combined_blocking(claude_bin);
    let plan = plan_with_usage_fallback(plan, &usage);
    if let Ok(mut cache) = PTY_CACHE.lock() {
        *cache = Some(PtyCache {
            fetched_at: std::time::Instant::now(),
            usage: usage.clone(),
            plan: plan.clone(),
        });
    }
    (usage, plan)
}

#[cfg(unix)]
fn combined_blocking(
    claude_bin: &std::path::Path,
) -> (Option<AgentUsageSnapshot>, Option<Vec<AgentInfoField>>) {
    let Some(mut session) = ClaudePtySession::spawn(claude_bin) else {
        return (None, None);
    };
    session.drain_startup();

    let (usage_text, usage_matched) = if session.send_command(b"/usage\r") {
        let deadline = std::time::Instant::now() + USAGE_PROBE_TIMEOUT;
        session.read_capture(
            deadline,
            READ_TICK_INTERVAL,
            USAGE_SETTLE,
            USAGE_PALETTE,
            usage_panel_ready,
        )
    } else {
        (String::new(), false)
    };

    let usage = if usage_matched {
        parse_usage_output(&usage_text)
    } else {
        tracing::debug!("claude cli usage: panel not found in output");
        None
    };

    let plan = if session.send_command(b"/status\r") {
        let deadline = std::time::Instant::now() + STATUS_PROBE_TIMEOUT;
        let (status_text, status_matched) = session.read_capture(
            deadline,
            READ_TICK_INTERVAL,
            STATUS_SETTLE,
            STATUS_PALETTE,
            status_panel_ready,
        );
        let fields = identity_to_fields(parse_cli_identity(&usage_text, &status_text));
        if fields.is_none() {
            if status_matched {
                tracing::debug!("claude cli plan: /status matched but no identity parsed");
            } else {
                tracing::debug!("claude cli plan: /status panel not found in output");
            }
            if std::env::var_os("ZEDRA_DEBUG_CLAUDE_PTY").is_some() {
                eprintln!(
                    "--- claude /status capture (matched={status_matched}) ---\n{}",
                    status_text.chars().take(6000).collect::<String>()
                );
            }
        }
        fields
    } else {
        None
    };

    session.finish();
    let plan = plan_with_usage_fallback(plan, &usage);
    (usage, plan)
}

fn logged_in_plan_fields() -> Vec<AgentInfoField> {
    vec![AgentInfoField {
        label: "Logged in".to_string(),
        value: "yes".to_string(),
    }]
}

pub(crate) fn plan_with_usage_fallback(
    plan: Option<Vec<AgentInfoField>>,
    usage: &Option<AgentUsageSnapshot>,
) -> Option<Vec<AgentInfoField>> {
    if plan.is_some() {
        return plan;
    }
    usage.as_ref().map(|_| logged_in_plan_fields())
}

struct CliIdentity {
    plan: Option<String>,
    email: Option<String>,
    organization: Option<String>,
}

fn parse_cli_identity(usage_text: &str, status_text: &str) -> CliIdentity {
    CliIdentity {
        plan: extract_claude_cli_login_method(status_text)
            .or_else(|| extract_claude_cli_login_method(usage_text)),
        email: extract_claude_cli_email(status_text)
            .or_else(|| extract_claude_cli_email(usage_text)),
        organization: extract_claude_cli_organization(status_text)
            .or_else(|| extract_claude_cli_organization(usage_text)),
    }
}

fn identity_to_fields(identity: CliIdentity) -> Option<Vec<AgentInfoField>> {
    let has_signal =
        identity.plan.is_some() || identity.email.is_some() || identity.organization.is_some();
    if !has_signal {
        return None;
    }
    let mut fields = vec![AgentInfoField {
        label: "Logged in".to_string(),
        value: "yes".to_string(),
    }];
    if let Some(plan) = identity.plan {
        fields.push(AgentInfoField {
            label: "Plan".to_string(),
            value: plan,
        });
    }
    if let Some(email) = identity.email {
        fields.push(AgentInfoField {
            label: "Account".to_string(),
            value: email,
        });
    }
    if let Some(org) = identity.organization {
        fields.push(AgentInfoField {
            label: "Organization".to_string(),
            value: org,
        });
    }
    Some(fields)
}

fn extract_claude_cli_login_method(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    if let Some(idx) = lower.find("login method:") {
        let rest = text.get(idx + "login method:".len()..)?;
        let line = rest.lines().next()?.trim();
        if let Some(plan) = claude_plan_from_cli_login_method(line) {
            return Some(plan);
        }
    }
    for line in text.lines() {
        let lower = line.to_ascii_lowercase();
        if lower.contains("claude code v") {
            continue;
        }
        if let Some(plan) = claude_plan_from_cli_login_phrase(line) {
            return Some(plan);
        }
    }
    None
}

fn claude_plan_from_cli_login_method(raw: &str) -> Option<String> {
    let cleaned = clean_cli_plan_label(raw);
    claude_plan_from_cli_login_phrase(&cleaned)
}

fn claude_plan_from_cli_login_phrase(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    if lower.contains("enterprise") {
        return Some("Enterprise".to_string());
    }
    if lower.contains("ultra") {
        return Some("Ultra".to_string());
    }
    if lower.contains("max") {
        return Some("Max".to_string());
    }
    if lower.contains("team") {
        return Some("Team".to_string());
    }
    if lower.contains("pro") {
        return Some("Pro".to_string());
    }
    None
}

fn clean_cli_plan_label(raw: &str) -> String {
    let mut out = String::new();
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '[' {
            while let Some(&next) = chars.peek() {
                chars.next();
                if next == ']' {
                    break;
                }
            }
            continue;
        }
        out.push(ch);
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn extract_claude_cli_email(text: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();
        for prefix in ["account:", "email:"] {
            if lower.starts_with(prefix) {
                let value = trimmed[prefix.len()..].trim();
                if value.contains('@') {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

fn extract_claude_cli_organization(text: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();
        for prefix in ["org:", "organization:"] {
            if lower.starts_with(prefix) {
                let value = trimmed[prefix.len()..].trim();
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

fn which_claude() -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join("claude"))
        .find(|p| p.is_file())
}

/// Strip ANSI escapes from a byte slice. Multi-byte UTF-8 is consumed whole by leading-byte
/// length; sequences truncated at the buffer end (split reads) are dropped, not replaced.
fn strip_ansi(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        if input[i] == b'\x1b' {
            i += 1;
            if i >= input.len() {
                break;
            }
            if input[i] == b'[' {
                // CSI sequence: skip until a byte in 0x40–0x7E
                i += 1;
                while i < input.len() && !(0x40..=0x7e).contains(&input[i]) {
                    i += 1;
                }
                i += 1; // skip final byte
            } else if matches!(input[i], b']' | b'P' | b'^' | b'_' | b'X') {
                // OSC / DCS / PM / APC / SOS: scan until BEL or ST (ESC \)
                i += 1;
                while i < input.len() {
                    if input[i] == b'\x07' {
                        i += 1;
                        break;
                    }
                    if input[i] == b'\x1b' && i + 1 < input.len() && input[i + 1] == b'\\' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
            } else {
                // Other two-byte escape sequence: skip one more byte
                i += 1;
            }
        } else if input[i] < 0x20 && input[i] != b'\n' && input[i] != b'\r' && input[i] != b'\t' {
            // Skip other control bytes (e.g. cursor movement OSC)
            i += 1;
        } else {
            // Push the whole UTF-8 character by leading-byte width, not byte-by-byte.
            let char_len = utf8_char_len(input[i]);
            if i + char_len <= input.len() {
                if let Ok(s) = std::str::from_utf8(&input[i..i + char_len]) {
                    out.push_str(s);
                }
                // Skip every byte of the char even if invalid, so continuation bytes
                // aren't treated as independent characters.
                i += char_len;
            } else {
                break; // truncated char at buffer end
            }
        }
    }
    out
}

/// Returns the expected byte length of a UTF-8 character given its leading byte.
/// Stray continuation bytes (`0x80..=0xBF`) are not valid leads, so they count as
/// one invalid byte and never consume the following character.
fn utf8_char_len(b: u8) -> usize {
    if b < 0x80 || (0x80..=0xBF).contains(&b) {
        1
    } else if b < 0xE0 {
        2
    } else if b < 0xF0 {
        3
    } else {
        4
    }
}

// CLI-only: parse `Resets …` lines after `/usage` window labels (fragile; see module docs).
// CLI-only: parse `Resets …` lines after `/usage` window labels (fragile; see PTY note above).
fn claude_session_label(lower: &str) -> bool {
    (lower.contains("current session") || lower.contains("currentsession"))
        && !lower.contains("current week")
        && !lower.contains("currentweek")
}

fn claude_week_all_models_label(lower: &str) -> bool {
    lower.contains("current week (all models)")
        || lower.contains("currentweek(allmodels)")
        || (lower.contains("current week") || lower.contains("currentweek"))
            && !lower.contains("sonnet")
            && !lower.contains("opus")
}

fn claude_label_search_key(text: &str) -> String {
    text.to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect()
}

fn find_claude_reset_after_label(
    lines: &[&str],
    label_idx: usize,
    label_line: &str,
) -> Option<i64> {
    let label_key = claude_label_search_key(label_line);
    for line in lines.iter().skip(label_idx + 1).take(14) {
        if is_other_current_section(line, &label_key) {
            break;
        }
        if let Some(ts) = parse_claude_reset_line(line) {
            return Some(ts);
        }
    }
    None
}

fn is_other_current_section(line: &str, label_key: &str) -> bool {
    let trimmed = line.trim();
    let mut chars = trimmed.chars().filter(|c| c.is_ascii_alphanumeric());
    let current = "current";
    for expected in current.chars() {
        if chars.next().map(|c| c.to_ascii_lowercase()) != Some(expected) {
            return false;
        }
    }
    let rest: String = chars.map(|c| c.to_ascii_lowercase()).collect();
    !rest.is_empty() && !rest.contains(label_key)
}
fn parse_claude_reset_line(line: &str) -> Option<i64> {
    let lower = line.to_ascii_lowercase();
    if !lower.contains("reset") {
        return None;
    }
    let body = strip_claude_resets_prefix(line)?;
    parse_claude_reset_body(body)
}

fn strip_claude_resets_prefix(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    let lower = trimmed.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("resets")
        .or_else(|| lower.strip_prefix("reset"))?;
    let skip = trimmed.len() - rest.len();
    let mut rest = trimmed.get(skip..)?.trim_start();
    rest = rest.trim_start_matches(|c: char| c == ':' || c.is_whitespace());
    (!rest.is_empty()).then_some(rest)
}

fn parse_claude_reset_body(body: &str) -> Option<i64> {
    let (text, _tz) = split_trailing_timezone(body);
    let text = normalize_claude_reset_text(text);
    let now = Local::now();

    if let Some(ts) = parse_claude_reset_month_day_time(&text, now) {
        return Some(ts);
    }

    let datetime_formats = [
        "%b %d, %Y, %I:%M%p",
        "%b %d, %Y, %I%p",
        "%b %d %Y %I:%M%p",
        "%b %d %Y %I%p",
        "%b %d, %I:%M%p",
        "%b %d, %I%p",
        "%b %d %I:%M%p",
        "%b %d %I%p",
        "%b %d at %I:%M%p",
        "%b %d at %I%p",
        "%b %d, %H:%M",
        "%b %d %H:%M",
    ];
    for fmt in datetime_formats {
        if let Ok(ndt) = NaiveDateTime::parse_from_str(&text, fmt) {
            let year = ndt.date().year();
            if let Some(dt) = local_from_month_day_time(year, ndt.date(), ndt.time()) {
                return Some(dt.timestamp());
            }
        }
    }

    let time_formats = ["%I:%M%p", "%I%p", "%H:%M"];
    for fmt in time_formats {
        if let Ok(time) = NaiveTime::parse_from_str(&text, fmt) {
            let today = now.date_naive();
            if let Some(dt) = local_from_month_day_time(now.year(), today, time) {
                if dt < now {
                    if let Some(tomorrow) = today.succ_opt() {
                        if let Some(dt) = local_from_month_day_time(now.year(), tomorrow, time) {
                            return Some(dt.timestamp());
                        }
                    }
                }
                return Some(dt.timestamp());
            }
        }
    }
    None
}

/// Parses `May 30 2pm` after spacing normalization (single-digit hours are common in CLI output).
fn parse_claude_reset_month_day_time(text: &str, now: DateTime<Local>) -> Option<i64> {
    let parts: Vec<&str> = text.split_whitespace().collect();
    if parts.len() != 3 {
        return None;
    }
    let year = now.year();
    let date = format!("{} {} {}", parts[0], parts[1], year);
    let date = NaiveDate::parse_from_str(&date, "%b %d %Y").ok()?;
    let time = parse_claude_reset_ampm_token(parts[2])?;
    local_from_month_day_time(year, date, time).map(|dt| dt.timestamp())
}

fn parse_claude_reset_ampm_token(token: &str) -> Option<NaiveTime> {
    let lower = token.to_ascii_lowercase();
    let (digits, pm) = if let Some(d) = lower.strip_suffix("pm") {
        (d, true)
    } else if let Some(d) = lower.strip_suffix("am") {
        (d, false)
    } else {
        return None;
    };
    let hour12: u32 = digits.parse().ok()?;
    if !(1..=12).contains(&hour12) {
        return None;
    }
    let hour24 = match (hour12, pm) {
        (12, false) => 0,
        (12, true) => 12,
        (h, true) => h + 12,
        (h, false) => h,
    };
    NaiveTime::from_hms_opt(hour24, 0, 0)
}

fn split_trailing_timezone(body: &str) -> (&str, Option<&str>) {
    let trimmed = body.trim();
    let Some(start) = trimmed.rfind('(') else {
        return (trimmed, None);
    };
    let Some(end) = trimmed.rfind(')') else {
        return (trimmed, None);
    };
    if end <= start {
        return (trimmed, None);
    }
    let tz = trimmed[start + 1..end].trim();
    let text = trimmed[..start].trim();
    (text, (!tz.is_empty()).then_some(tz))
}

fn normalize_claude_reset_text(raw: &str) -> String {
    let mut text = raw.trim().to_string();
    if let Some(stripped) = text.strip_prefix("Resets") {
        text = stripped.trim().to_string();
    } else if let Some(stripped) = text.strip_prefix("resets") {
        text = stripped.trim().to_string();
    }
    text = insert_claude_reset_spacing(&text);
    text = text.replace(" at ", " ");
    while text.contains("  ") {
        text = text.replace("  ", " ");
    }
    text
}

/// TTY capture often glues tokens (`May30at2pm`); mirror codexbar spacing normalization.
fn insert_claude_reset_spacing(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut result = String::with_capacity(input.len() + 8);
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_alphabetic() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_alphabetic() {
                i += 1;
            }
            let word_len = i - start;
            if word_len == 3 && i < chars.len() && chars[i].is_ascii_digit() {
                result.extend(&chars[start..i]);
                result.push(' ');
                continue;
            }
            result.extend(&chars[start..i]);
            continue;
        }
        if chars[i].is_ascii_digit() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            if i + 2 <= chars.len()
                && (chars[i] == 'a' || chars[i] == 'A')
                && (chars[i + 1] == 't' || chars[i + 1] == 'T')
                && i + 2 < chars.len()
                && chars[i + 2].is_ascii_digit()
            {
                result.extend(&chars[start..i]);
                result.push(' ');
                result.push_str("at");
                result.push(' ');
                i += 2;
                continue;
            }
            result.extend(&chars[start..i]);
            continue;
        }
        result.push(chars[i]);
        i += 1;
    }
    result
}

fn local_from_month_day_time(
    year: i32,
    date: NaiveDate,
    time: NaiveTime,
) -> Option<DateTime<Local>> {
    let date = date.with_year(year)?;
    let ndt = NaiveDateTime::new(date, time);
    match Local.from_local_datetime(&ndt) {
        LocalResult::Single(dt) => Some(dt),
        LocalResult::Ambiguous(earliest, _) => Some(earliest),
        LocalResult::None => None,
    }
}

/// Parse Claude `/usage` PTY text into `AgentUsageSnapshot`. Labels may be unspaced
/// (`Currentsession`, `Currentweek(allmodels)`) with the percent on the next line; reset lines are
/// best-effort host-local unix timestamps (see module note when `resets_at` is None).
fn parse_usage_output(text: &str) -> Option<AgentUsageSnapshot> {
    let mut session_pct: Option<f32> = None;
    let mut week_pct: Option<f32> = None;
    let mut session_resets_at: Option<i64> = None;
    let mut week_resets_at: Option<i64> = None;
    let mut credits_used: Option<f64> = None;
    let mut credits_limit: Option<f64> = None;

    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let lower = line.to_ascii_lowercase();
        let next = lines.get(i + 1).copied().unwrap_or("");

        if claude_session_label(&lower) {
            let pct = extract_first_percent(line).or_else(|| extract_first_percent(next));
            if pct.is_some() {
                session_pct = pct;
            }
            if session_resets_at.is_none() {
                session_resets_at = find_claude_reset_after_label(&lines, i, line);
            }
        }
        if claude_week_all_models_label(&lower) {
            let pct = extract_first_percent(line).or_else(|| extract_first_percent(next));
            if pct.is_some() {
                week_pct = pct;
            }
            if week_resets_at.is_none() {
                week_resets_at = find_claude_reset_after_label(&lines, i, line);
            }
        }
        // "Usage credits  $0.21 / $50.00 spent" — usually all on one line.
        if lower.contains("usage credits") || lower.contains("usagecredits") {
            let (u, l) = extract_dollar_fraction(line);
            if u.is_some() {
                credits_used = u;
                credits_limit = l;
            }
        }
        i += 1;
    }

    if session_pct.is_none() && week_pct.is_none() && credits_used.is_none() {
        return None;
    }

    let context_used = credits_used.zip(credits_limit).and_then(|(u, l)| {
        if l > 0.0 {
            Some((u / l * 100.0) as f32)
        } else {
            None
        }
    });

    Some(AgentUsageSnapshot {
        rate_limit_five_hour_used_percent: session_pct,
        rate_limit_seven_day_used_percent: week_pct,
        rate_limit_five_hour_resets_at: session_resets_at,
        rate_limit_seven_day_resets_at: week_resets_at,
        context_used_percent: context_used,
        total_cost_usd: credits_used,
        ..Default::default()
    })
}

/// Extract the first `NN%` value from a line of text. Returns the number as f32.
fn extract_first_percent(line: &str) -> Option<f32> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            // Walk backwards over digits (and optional `.`)
            let mut j = i;
            while j > 0 && (bytes[j - 1].is_ascii_digit() || bytes[j - 1] == b'.') {
                j -= 1;
            }
            if j < i {
                if let Ok(s) = std::str::from_utf8(&bytes[j..i]) {
                    if let Ok(v) = s.parse::<f32>() {
                        return Some(v);
                    }
                }
            }
        }
        i += 1;
    }
    None
}

/// Extract `$used/$limit` from a line. Returns `(used, limit)` as `Option<f64>` pair.
fn extract_dollar_fraction(line: &str) -> (Option<f64>, Option<f64>) {
    let mut values: Vec<f64> = Vec::new();
    let mut i = 0;
    let bytes = line.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'$' {
            i += 1;
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            if i > start {
                if let Ok(s) = std::str::from_utf8(&bytes[start..i]) {
                    if let Ok(v) = s.parse::<f64>() {
                        values.push(v);
                    }
                }
            }
        } else {
            i += 1;
        }
    }
    (values.first().copied(), values.get(1).copied())
}

#[cfg(unix)]
pub(crate) async fn fetch_usage() -> Option<AgentUsageSnapshot> {
    let claude_bin = which_claude()?;
    tokio::task::spawn_blocking(move || with_pty_lock(|| probe_cached(&claude_bin).0))
        .await
        .ok()
        .flatten()
}

#[cfg(unix)]
pub(crate) async fn fetch_plan_fields() -> Option<Vec<AgentInfoField>> {
    let claude_bin = which_claude()?;
    tokio::task::spawn_blocking(move || with_pty_lock(|| probe_cached(&claude_bin).1))
        .await
        .ok()
        .flatten()
}

#[cfg(not(unix))]
pub(crate) async fn fetch_usage() -> Option<AgentUsageSnapshot> {
    None
}

#[cfg(not(unix))]
pub(crate) async fn fetch_plan_fields() -> Option<Vec<AgentInfoField>> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const GLUED_PANEL: &str = "Currentsession\n100%used\nResets2:40pm(Asia/Saigon)\nCurrentweek(allmodels)\n27%used\nResetsMay30at2pm(Asia/Saigon)\n";

    fn field_value<'a>(fields: &'a [AgentInfoField], label: &str) -> Option<&'a str> {
        fields
            .iter()
            .find(|field| field.label == label)
            .map(|field| field.value.as_str())
    }

    fn parse_cli_plan_fields(status_text: &str) -> Option<Vec<AgentInfoField>> {
        identity_to_fields(parse_cli_identity("", status_text))
    }

    #[test]
    fn strip_ansi_ascii_passthrough() {
        assert_eq!(strip_ansi(b"hello world\n"), "hello world\n");
    }

    #[test]
    fn strip_ansi_removes_csi_sequences() {
        assert_eq!(strip_ansi(b"\x1b[1mBold\x1b[0m"), "Bold");
        assert_eq!(strip_ansi(b"\x1b[2;5Htext"), "text");
    }

    #[test]
    fn parse_usage_inline_percentages() {
        let text = "Current session 45% used\nCurrent week (all models) 18% used\n";
        let snap = parse_usage_output(text).unwrap();
        assert_eq!(snap.rate_limit_five_hour_used_percent, Some(45.0));
        assert_eq!(snap.rate_limit_seven_day_used_percent, Some(18.0));
    }

    #[test]
    fn parse_usage_resets_tty_glued_tokens() {
        let snap = parse_usage_output(GLUED_PANEL).unwrap();
        assert!(snap.rate_limit_five_hour_resets_at.is_some());
        assert!(snap.rate_limit_seven_day_resets_at.is_some());
    }

    #[test]
    fn parse_usage_fixture_glued_panel() {
        assert!(parse_usage_output(GLUED_PANEL).is_some());
    }

    #[test]
    fn parse_claude_reset_line_glued_week() {
        assert!(parse_claude_reset_line("ResetsMay30at2pm(Asia/Saigon)").is_some());
        assert!(parse_claude_reset_line("Resets2:40pm(Asia/Saigon)").is_some());
    }

    #[test]
    fn parse_cli_plan_from_login_method_line() {
        let status = r#"
        Login method: Claude Max Account
        Account: user@example.com
        Org: ACME
        "#;
        let fields = parse_cli_plan_fields(status).expect("fields");
        assert_eq!(field_value(&fields, "Plan"), Some("Max"));
    }

    #[test]
    fn extract_percent_finds_first_value() {
        assert_eq!(extract_first_percent("  30% used"), Some(30.0));
        assert_eq!(extract_first_percent("100%"), Some(100.0));
    }

    #[test]
    fn cli_plan_fallback_logged_in_when_usage_present() {
        let usage = AgentUsageSnapshot {
            rate_limit_five_hour_used_percent: Some(10.0),
            ..Default::default()
        };
        let fields = plan_with_usage_fallback(None, &Some(usage)).expect("fields");
        assert_eq!(field_value(&fields, "Logged in"), Some("yes"));
    }

    #[test]
    #[cfg(unix)]
    #[ignore = "manual: requires claude on PATH and Keychain auth"]
    fn manual_combined_probe() {
        let Some(bin) = which_claude() else {
            return;
        };
        let (usage, plan) = combined_blocking(&bin);
        eprintln!("usage ok: {}", usage.is_some());
        eprintln!("plan: {plan:?}");
    }
}
