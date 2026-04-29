/// State machine that scans raw PTY bytes for OSC sequences across chunk
/// boundaries (split packets). Handles both BEL (0x07) and ST (ESC `\`)
/// terminators. Only a known subset of OSC numbers are decoded; unknown
/// sequences are silently dropped.
#[derive(Default)]
pub struct OscScanner {
    state: ScanState,
}

/// Guard against pathological producers that emit huge OSC bodies
/// (e.g. inline image payloads we don't handle). Once the in-progress
/// buffer exceeds this, the scanner drops the sequence and resets.
const MAX_OSC_BODY: usize = 64 * 1024;

#[derive(Default)]
enum ScanState {
    #[default]
    Idle,
    SawEsc,
    SawBracket,
    CollectingOsc {
        buf: Vec<u8>,
        esc_pending: bool,
    },
}

/// Source terminal that emitted a notification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotificationSource {
    /// OSC 9 — iTerm-style simple notification.
    Osc9,
    /// OSC 99 — Kitty rich notification.
    Osc99,
    /// OSC 777;notify — rxvt/urxvt style.
    Osc777,
}

/// ConEmu/Ghostty OSC 9;4 progress state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgressState {
    /// State 0 — no progress, clear any existing indicator.
    Inactive,
    /// State 1 — normal progress.
    Normal,
    /// State 2 — error state.
    Error,
    /// State 3 — indeterminate ("spinner").
    Indeterminate,
    /// State 4 — warning / paused.
    Warning,
}

/// A progress report emitted by OSC 9;4.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProgressReport {
    pub state: ProgressState,
    /// Percentage 0..=100 when applicable; `None` for Inactive/Indeterminate.
    pub value: Option<u8>,
}

/// An event decoded from an OSC escape sequence in the PTY byte stream.
#[derive(Debug, Clone)]
pub enum OscEvent {
    /// OSC 0 / OSC 2 — window / tab title set by the shell.
    Title(String),
    /// OSC 0 / OSC 2 with empty argument — reset to default title.
    ResetTitle,
    /// OSC 1 — icon name (some shells treat this as a secondary label).
    IconName(String),
    /// OSC 7 / OSC 633;P;Cwd / OSC 1337;CurrentDir — current working directory.
    Cwd(String),
    /// OSC 133;A or 133;B / OSC 633;A or 633;B — shell at a prompt.
    PromptReady,
    /// OSC 133;C / OSC 633;C — a command has started executing.
    CommandStart,
    /// OSC 133;D / OSC 633;D — a command finished; carries its exit code.
    CommandEnd { exit_code: i32 },
    /// OSC 633;E — VS Code shell integration escaped command line.
    CommandLine(String),
    /// OSC 633;P;<key>=<value> — VS Code shell integration property.
    ShellProperty { key: String, value: String },
    /// OSC 9 / OSC 99 / OSC 777;notify — desktop notification.
    Notification {
        title: Option<String>,
        body: String,
        source: NotificationSource,
    },
    /// OSC 9;4 — progress indicator.
    Progress(ProgressReport),
    /// OSC 1337;RemoteHost=... — iTerm2 shell integration remote host.
    RemoteHost(String),
    /// OSC 1337;ShellIntegrationVersion=... — iTerm2 shell integration version.
    ShellIntegrationVersion(String),
    /// OSC 1337;shell=... — shell name paired with ShellIntegrationVersion.
    ShellName(String),
    /// OSC 1337;SetUserVar=<name>=<base64> — user variable (raw, base64-encoded).
    UserVar { key: String, value_b64: String },
}

impl OscScanner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Vec<OscEvent> {
        let mut events = Vec::new();
        for &b in bytes {
            self.state = match std::mem::take(&mut self.state) {
                ScanState::Idle => {
                    if b == 0x1B {
                        ScanState::SawEsc
                    } else {
                        ScanState::Idle
                    }
                }
                ScanState::SawEsc => match b {
                    b']' => ScanState::SawBracket,
                    0x1B => ScanState::SawEsc,
                    _ => ScanState::Idle,
                },
                ScanState::SawBracket => match b {
                    0x07 => ScanState::Idle,
                    0x1B => ScanState::CollectingOsc {
                        buf: Vec::new(),
                        esc_pending: true,
                    },
                    _ => ScanState::CollectingOsc {
                        buf: vec![b],
                        esc_pending: false,
                    },
                },
                ScanState::CollectingOsc {
                    mut buf,
                    esc_pending,
                } => {
                    if esc_pending {
                        match b {
                            b'\\' => {
                                parse_osc_into(&buf, &mut events);
                                ScanState::Idle
                            }
                            b']' => ScanState::SawBracket,
                            _ => {
                                buf.push(0x1B);
                                buf.push(b);
                                if buf.len() > MAX_OSC_BODY {
                                    ScanState::Idle
                                } else {
                                    ScanState::CollectingOsc {
                                        buf,
                                        esc_pending: false,
                                    }
                                }
                            }
                        }
                    } else {
                        match b {
                            0x07 => {
                                parse_osc_into(&buf, &mut events);
                                ScanState::Idle
                            }
                            0x1B => ScanState::CollectingOsc {
                                buf,
                                esc_pending: true,
                            },
                            _ => {
                                buf.push(b);
                                if buf.len() > MAX_OSC_BODY {
                                    ScanState::Idle
                                } else {
                                    ScanState::CollectingOsc {
                                        buf,
                                        esc_pending: false,
                                    }
                                }
                            }
                        }
                    }
                }
            };
        }
        events
    }

    pub fn reset(&mut self) {
        self.state = ScanState::Idle;
    }
}

/// Parse a completed OSC body (bytes between `]` and the terminator).
/// The body has the form `<num>;<rest>` where `<rest>` is sequence-specific.
/// Some sequences (633, 1337) can emit multiple logical events from one
/// body, so this takes an output vec instead of returning `Option`.
fn parse_osc_into(buf: &[u8], out: &mut Vec<OscEvent>) {
    let Some(sep) = buf.iter().position(|&b| b == b';') else {
        return;
    };
    let num = &buf[..sep];
    let body = &buf[sep + 1..];

    match num {
        b"0" | b"2" => {
            if let Ok(s) = std::str::from_utf8(body) {
                let title = s.trim_end_matches('\0').to_owned();
                if title.is_empty() {
                    out.push(OscEvent::ResetTitle);
                } else {
                    out.push(OscEvent::Title(title));
                }
            }
        }
        b"1" => {
            if let Ok(s) = std::str::from_utf8(body) {
                let name = s.trim_end_matches('\0').to_owned();
                if !name.is_empty() {
                    out.push(OscEvent::IconName(name));
                }
            }
        }
        b"7" => {
            if let Ok(url) = std::str::from_utf8(body) {
                if let Some(path) = parse_cwd_url(url.trim()) {
                    out.push(OscEvent::Cwd(path));
                }
            }
        }
        b"9" => parse_osc_9(body, out),
        b"99" => parse_osc_99(body, out),
        b"133" => {
            if let Some(ev) = parse_osc_133_or_633(body) {
                out.push(ev);
            }
        }
        b"633" => parse_osc_633(body, out),
        b"777" => parse_osc_777(body, out),
        b"1337" => parse_osc_1337(body, out),
        _ => {}
    }
}

/// OSC 9 — either a simple notification (`9;<body>`) or a ConEmu progress
/// report (`9;4;<state>[;<value>]`).
fn parse_osc_9(body: &[u8], out: &mut Vec<OscEvent>) {
    // Progress form: `4;<state>[;<value>]`.
    if let Some(rest) = body.strip_prefix(b"4;") {
        parse_osc_9_4(rest, out);
        return;
    }
    if let Ok(s) = std::str::from_utf8(body) {
        let text = s.to_owned();
        if !text.is_empty() {
            out.push(OscEvent::Notification {
                title: None,
                body: text,
                source: NotificationSource::Osc9,
            });
        }
    }
}

fn parse_osc_9_4(body: &[u8], out: &mut Vec<OscEvent>) {
    let Ok(s) = std::str::from_utf8(body) else {
        return;
    };
    let mut parts = s.split(';');
    let Some(state_s) = parts.next() else {
        return;
    };
    let value = parts
        .next()
        .and_then(|v| v.trim().parse::<u32>().ok())
        .map(|v| v.min(100) as u8);
    let state = match state_s.trim() {
        "0" | "" => ProgressState::Inactive,
        "1" => ProgressState::Normal,
        "2" => ProgressState::Error,
        "3" => ProgressState::Indeterminate,
        "4" => ProgressState::Warning,
        _ => return,
    };
    let report = ProgressReport {
        state,
        value: match state {
            ProgressState::Inactive | ProgressState::Indeterminate => None,
            _ => value,
        },
    };
    out.push(OscEvent::Progress(report));
}

/// OSC 99 — Kitty rich notification. Body form: `<params>:<payload>`.
/// We only surface title (`d=`) and body (the payload), ignoring IDs,
/// actions, and close events for now.
fn parse_osc_99(body: &[u8], out: &mut Vec<OscEvent>) {
    let Ok(s) = std::str::from_utf8(body) else {
        return;
    };
    let (params, payload) = match s.split_once(':') {
        Some((p, r)) => (p, r),
        None => ("", s),
    };
    if payload.is_empty() {
        return;
    }
    let title = params
        .split(':')
        .find_map(|kv| kv.strip_prefix("d="))
        .map(|t| t.to_owned());
    out.push(OscEvent::Notification {
        title,
        body: payload.to_owned(),
        source: NotificationSource::Osc99,
    });
}

/// Parse the body of an OSC 133 / 633 semantic prompt sequence.
/// Body format: `<mark>[;<params>]` where mark is A / B / C / D.
fn parse_osc_133_or_633(body: &[u8]) -> Option<OscEvent> {
    match *body.first()? {
        b'A' | b'B' => Some(OscEvent::PromptReady),
        b'C' => Some(OscEvent::CommandStart),
        b'D' => {
            let exit_code = if body.len() > 1 && body[1] == b';' {
                std::str::from_utf8(&body[2..])
                    .ok()
                    .and_then(|s| s.trim().parse::<i32>().ok())
                    .unwrap_or(0)
            } else {
                0
            };
            Some(OscEvent::CommandEnd { exit_code })
        }
        _ => None,
    }
}

/// OSC 633 — VS Code shell integration. Marks A..D share semantics with
/// 133; E carries the escaped command line; P carries a `key=value`
/// property.
fn parse_osc_633(body: &[u8], out: &mut Vec<OscEvent>) {
    let Some(&mark) = body.first() else {
        return;
    };
    match mark {
        b'A' | b'B' | b'C' | b'D' => {
            if let Some(ev) = parse_osc_133_or_633(body) {
                out.push(ev);
            }
        }
        b'E' => {
            // `E;<escaped-cmd>[;<nonce>]`. VS Code escapes `\` and `;`
            // inside the command with a backslash prefix.
            if body.len() < 2 || body[1] != b';' {
                return;
            }
            let rest = &body[2..];
            let (cmd_raw, _nonce) = split_unescaped_semi(rest);
            if let Ok(cmd_s) = std::str::from_utf8(cmd_raw) {
                let cmd = unescape_osc633(cmd_s);
                if !cmd.is_empty() {
                    out.push(OscEvent::CommandLine(cmd));
                }
            }
        }
        b'P' => {
            // `P;<key>=<value>`.
            if body.len() < 2 || body[1] != b';' {
                return;
            }
            let kv = &body[2..];
            if let Ok(s) = std::str::from_utf8(kv) {
                if let Some((k, v)) = s.split_once('=') {
                    // Cwd key is surfaced as a strongly-typed Cwd event too.
                    if k.eq_ignore_ascii_case("Cwd") {
                        out.push(OscEvent::Cwd(unescape_osc633(v)));
                    }
                    out.push(OscEvent::ShellProperty {
                        key: k.to_owned(),
                        value: unescape_osc633(v),
                    });
                }
            }
        }
        _ => {}
    }
}

/// Split a 633;E payload on the first *unescaped* `;`. Returns
/// `(command_bytes, nonce_bytes)`.
fn split_unescaped_semi(body: &[u8]) -> (&[u8], &[u8]) {
    let mut i = 0;
    while i < body.len() {
        if body[i] == b'\\' && i + 1 < body.len() {
            i += 2;
            continue;
        }
        if body[i] == b';' {
            return (&body[..i], &body[i + 1..]);
        }
        i += 1;
    }
    (body, &[])
}

/// Undo the `\xNN` / `\\` / `\;` style escaping used by OSC 633 payloads.
/// Also decodes `\x20`-style sequences that VS Code emits for control chars.
///
/// Preserves multi-byte UTF-8: non-`\`-prefixed runs are copied as string
/// slices; only bytes produced from `\xNN` escapes go through per-byte
/// decoding (legal, since they always encode ASCII-range control chars).
fn unescape_osc633(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(bytes.len());
    let mut i = 0;
    let mut run_start = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' || i + 1 >= bytes.len() {
            i += 1;
            continue;
        }
        let next = bytes[i + 1];
        let (consumed, replacement): (usize, Option<char>) = match next {
            b'x' if i + 3 < bytes.len() => {
                let hi = (bytes[i + 2] as char).to_digit(16);
                let lo = (bytes[i + 3] as char).to_digit(16);
                match (hi, lo) {
                    (Some(h), Some(l)) => (4, Some(((h * 16 + l) as u8) as char)),
                    _ => (0, None),
                }
            }
            b'\\' => (2, Some('\\')),
            b';' => (2, Some(';')),
            _ => (0, None),
        };
        if consumed == 0 {
            i += 1;
            continue;
        }
        if run_start < i {
            out.push_str(std::str::from_utf8(&bytes[run_start..i]).unwrap_or(""));
        }
        if let Some(ch) = replacement {
            out.push(ch);
        }
        i += consumed;
        run_start = i;
    }
    if run_start < bytes.len() {
        out.push_str(std::str::from_utf8(&bytes[run_start..]).unwrap_or(""));
    }
    out
}

/// OSC 777;<subcmd>[;<args>]. We only handle `notify;<title>;<body>`.
fn parse_osc_777(body: &[u8], out: &mut Vec<OscEvent>) {
    let Ok(s) = std::str::from_utf8(body) else {
        return;
    };
    let mut parts = s.splitn(3, ';');
    let Some(sub) = parts.next() else {
        return;
    };
    if !sub.eq_ignore_ascii_case("notify") {
        return;
    }
    let title = parts.next().map(|t| t.to_owned()).filter(|t| !t.is_empty());
    let body_s = parts.next().unwrap_or("").to_owned();
    if title.is_none() && body_s.is_empty() {
        return;
    }
    out.push(OscEvent::Notification {
        title,
        body: body_s,
        source: NotificationSource::Osc777,
    });
}

/// OSC 1337 — iTerm2 family. Body is typically `<key>=<value>`, optionally
/// with additional `;key=value` pairs. We decode a small allowlist.
fn parse_osc_1337(body: &[u8], out: &mut Vec<OscEvent>) {
    let Ok(s) = std::str::from_utf8(body) else {
        return;
    };
    let (head, rest) = match s.split_once(';') {
        Some((h, r)) => (h, r),
        None => (s, ""),
    };
    dispatch_1337_pair(head, out);
    for pair in rest.split(';').filter(|p| !p.is_empty()) {
        dispatch_1337_pair(pair, out);
    }
}

fn dispatch_1337_pair(pair: &str, out: &mut Vec<OscEvent>) {
    let Some((key, value)) = pair.split_once('=') else {
        return;
    };
    let key_trim = key.trim();
    if key_trim.eq_ignore_ascii_case("CurrentDir") {
        out.push(OscEvent::Cwd(value.to_owned()));
    } else if key_trim.eq_ignore_ascii_case("RemoteHost") {
        out.push(OscEvent::RemoteHost(value.to_owned()));
    } else if key_trim.eq_ignore_ascii_case("ShellIntegrationVersion") {
        out.push(OscEvent::ShellIntegrationVersion(value.to_owned()));
    } else if key_trim.eq_ignore_ascii_case("shell") {
        out.push(OscEvent::ShellName(value.to_owned()));
    } else if key_trim.eq_ignore_ascii_case("SetUserVar") {
        if let Some((var_key, var_val)) = value.split_once('=') {
            out.push(OscEvent::UserVar {
                key: var_key.to_owned(),
                value_b64: var_val.to_owned(),
            });
        }
    }
}

fn parse_cwd_url(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("file://")
        .or_else(|| url.strip_prefix("kitty-shell-cwd://"))?;
    let path = if rest.starts_with('/') {
        rest
    } else {
        &rest[rest.find('/')?..]
    };
    Some(percent_decode(path))
}

fn percent_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (
                (b[i + 1] as char).to_digit(16),
                (b[i + 2] as char).to_digit(16),
            ) {
                out.push((h * 16 + l) as u8 as char);
                i += 3;
                continue;
            }
        }
        out.push(b[i] as char);
        i += 1;
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn feed_all(bytes: &[u8]) -> Vec<OscEvent> {
        let mut s = OscScanner::new();
        s.feed(bytes)
    }

    fn feed_chunks(chunks: &[&[u8]]) -> Vec<OscEvent> {
        let mut s = OscScanner::new();
        let mut all = Vec::new();
        for c in chunks {
            all.extend(s.feed(c));
        }
        all
    }

    #[test]
    fn parses_title_bel() {
        let ev = feed_all(b"\x1b]2;hello\x07");
        assert!(matches!(&ev[..], [OscEvent::Title(t)] if t == "hello"));
    }

    #[test]
    fn parses_title_st() {
        let ev = feed_all(b"\x1b]0;hi\x1b\\");
        assert!(matches!(&ev[..], [OscEvent::Title(t)] if t == "hi"));
    }

    #[test]
    fn parses_icon_name() {
        let ev = feed_all(b"\x1b]1;bash\x07");
        assert!(matches!(&ev[..], [OscEvent::IconName(n)] if n == "bash"));
    }

    #[test]
    fn parses_cwd() {
        let ev = feed_all(b"\x1b]7;file:///home/user\x07");
        assert!(matches!(&ev[..], [OscEvent::Cwd(p)] if p == "/home/user"));
    }

    #[test]
    fn parses_osc_133() {
        let ev = feed_all(b"\x1b]133;A\x07\x1b]133;C\x07\x1b]133;D;17\x07");
        assert_eq!(ev.len(), 3);
        assert!(matches!(ev[0], OscEvent::PromptReady));
        assert!(matches!(ev[1], OscEvent::CommandStart));
        assert!(matches!(ev[2], OscEvent::CommandEnd { exit_code: 17 }));
    }

    #[test]
    fn parses_osc_633_chain() {
        let ev = feed_all(b"\x1b]633;A\x07\x1b]633;E;ls -la\x07\x1b]633;C\x07\x1b]633;D;0\x07");
        assert!(matches!(ev[0], OscEvent::PromptReady));
        assert!(matches!(&ev[1], OscEvent::CommandLine(c) if c == "ls -la"));
        assert!(matches!(ev[2], OscEvent::CommandStart));
        assert!(matches!(ev[3], OscEvent::CommandEnd { exit_code: 0 }));
    }

    #[test]
    fn parses_osc_633_property_cwd() {
        let ev = feed_all(b"\x1b]633;P;Cwd=/tmp\x07");
        assert_eq!(ev.len(), 2);
        assert!(matches!(&ev[0], OscEvent::Cwd(p) if p == "/tmp"));
        assert!(matches!(
            &ev[1],
            OscEvent::ShellProperty { key, value } if key == "Cwd" && value == "/tmp"
        ));
    }

    #[test]
    fn decodes_633_e_escapes() {
        // `\;` becomes `;`, `\\` becomes `\`, `\x20` becomes space.
        let ev = feed_all(b"\x1b]633;E;echo hi\\x20bye\\;ls\x07");
        assert!(matches!(&ev[0], OscEvent::CommandLine(c) if c == "echo hi bye;ls"));
    }

    #[test]
    fn preserves_utf8_in_633_e() {
        // Multi-byte UTF-8 (café, 日本語) must round-trip unharmed.
        let mut bytes = b"\x1b]633;E;echo caf\xc3\xa9 ".to_vec();
        bytes.extend_from_slice("日本語".as_bytes());
        bytes.push(0x07);
        let ev = feed_all(&bytes);
        assert!(matches!(&ev[0], OscEvent::CommandLine(c) if c == "echo café 日本語"));
    }

    #[test]
    fn parses_osc_9_notification() {
        let ev = feed_all(b"\x1b]9;done\x07");
        assert!(matches!(
            &ev[0],
            OscEvent::Notification { title: None, body, source: NotificationSource::Osc9 }
                if body == "done"
        ));
    }

    #[test]
    fn parses_osc_9_numeric_notification_without_progress_prefix() {
        let ev = feed_all(b"\x1b]9;404 build failed\x07");
        assert!(matches!(
            &ev[0],
            OscEvent::Notification { title: None, body, source: NotificationSource::Osc9 }
                if body == "404 build failed"
        ));
    }

    #[test]
    fn parses_osc_9_4_progress_normal() {
        let ev = feed_all(b"\x1b]9;4;1;42\x07");
        assert!(matches!(
            ev[0],
            OscEvent::Progress(ProgressReport {
                state: ProgressState::Normal,
                value: Some(42)
            })
        ));
    }

    #[test]
    fn parses_osc_9_4_progress_clear() {
        let ev = feed_all(b"\x1b]9;4;0\x07");
        assert!(matches!(
            ev[0],
            OscEvent::Progress(ProgressReport {
                state: ProgressState::Inactive,
                value: None
            })
        ));
    }

    #[test]
    fn parses_osc_99_with_title() {
        let ev = feed_all(b"\x1b]99;d=Build:completed\x1b\\");
        assert!(matches!(
            &ev[0],
            OscEvent::Notification {
                title: Some(t),
                body,
                source: NotificationSource::Osc99,
            } if t == "Build" && body == "completed"
        ));
    }

    #[test]
    fn parses_osc_777() {
        let ev = feed_all(b"\x1b]777;notify;Alert;Body text\x07");
        assert!(matches!(
            &ev[0],
            OscEvent::Notification {
                title: Some(t),
                body,
                source: NotificationSource::Osc777,
            } if t == "Alert" && body == "Body text"
        ));
    }

    #[test]
    fn parses_osc_1337_remote_host() {
        let ev = feed_all(b"\x1b]1337;RemoteHost=user@box\x07");
        assert!(matches!(&ev[0], OscEvent::RemoteHost(h) if h == "user@box"));
    }

    #[test]
    fn parses_osc_1337_current_dir() {
        let ev = feed_all(b"\x1b]1337;CurrentDir=/tmp\x07");
        assert!(matches!(&ev[0], OscEvent::Cwd(p) if p == "/tmp"));
    }

    #[test]
    fn parses_osc_1337_shell_integration_version() {
        let ev = feed_all(b"\x1b]1337;ShellIntegrationVersion=5;shell=bash\x07");
        let mut saw_ver = false;
        let mut saw_shell = false;
        for e in &ev {
            match e {
                OscEvent::ShellIntegrationVersion(v) if v == "5" => saw_ver = true,
                OscEvent::ShellName(n) if n == "bash" => saw_shell = true,
                _ => {}
            }
        }
        assert!(saw_ver && saw_shell, "events: {ev:?}");
    }

    #[test]
    fn parses_osc_1337_set_user_var() {
        let ev = feed_all(b"\x1b]1337;SetUserVar=myKey=aGVsbG8=\x07");
        assert!(matches!(
            &ev[0],
            OscEvent::UserVar { key, value_b64 } if key == "myKey" && value_b64 == "aGVsbG8="
        ));
    }

    #[test]
    fn handles_chunked_osc_across_feeds() {
        let ev = feed_chunks(&[b"\x1b]2;par", b"tial\x07rest"]);
        assert!(matches!(&ev[..], [OscEvent::Title(t)] if t == "partial"));
    }

    #[test]
    fn drops_oversized_body() {
        let mut bytes = Vec::with_capacity(MAX_OSC_BODY + 64);
        bytes.extend_from_slice(b"\x1b]2;");
        bytes.extend(std::iter::repeat(b'A').take(MAX_OSC_BODY + 32));
        bytes.push(0x07);
        let ev = feed_all(&bytes);
        assert!(ev.is_empty(), "oversized OSC should be dropped");
    }
}
