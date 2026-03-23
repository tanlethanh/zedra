// OSC escape sequence scanner and terminal metadata types.
//
// Lives in zedra-rpc so both the host (metadata tracking) and client
// (TerminalMeta updates from raw PTY output) can share the same types
// without introducing a circular dependency.

// ---------------------------------------------------------------------------
// OSC event types
// ---------------------------------------------------------------------------

/// An event decoded from an OSC escape sequence in the PTY byte stream.
#[derive(Clone, Debug)]
pub enum OscEvent {
    /// OSC 0 / OSC 2 — window / tab title set by the shell.
    Title(String),
    /// OSC 0 / OSC 2 with empty argument — reset to default title.
    ResetTitle,
    /// OSC 7 — current working directory (`file://host/path` or `kitty-shell-cwd://…`).
    Cwd(String),
    /// OSC 133;A or 133;B — shell is at a prompt (idle, waiting for input).
    PromptReady,
    /// OSC 133;C — a command has started executing.
    CommandStart,
    /// OSC 133;D — a command finished; carries its exit code.
    CommandEnd { exit_code: i32 },
}

/// Shell execution state derived from OSC 133 semantic prompt marks.
#[derive(Clone, Default, Debug, PartialEq)]
pub enum ShellState {
    /// No OSC 133 sequence seen yet (e.g. shell integration not installed).
    #[default]
    Unknown,
    /// Shell is at a prompt, waiting for input.
    Idle,
    /// A command is currently executing.
    Running,
}

/// Last-known terminal metadata, updated in-place as OSC sequences arrive.
/// Cheap to clone (all fields are small Option<String> / enum).
#[derive(Clone, Default, Debug)]
pub struct TerminalMeta {
    /// Most recently set tab/window title (OSC 0/2).
    pub title: Option<String>,
    /// Most recently reported working directory (OSC 7).
    pub cwd: Option<String>,
    /// Exit code of the last completed command (OSC 133;D).
    pub last_exit_code: Option<i32>,
    /// Current shell execution state (OSC 133 prompt marks).
    pub shell_state: ShellState,
}

impl TerminalMeta {
    pub fn apply(&mut self, event: &OscEvent) {
        match event {
            OscEvent::Title(t) => self.title = Some(t.clone()),
            OscEvent::ResetTitle => self.title = None,
            OscEvent::Cwd(p) => self.cwd = Some(p.clone()),
            OscEvent::PromptReady => self.shell_state = ShellState::Idle,
            OscEvent::CommandStart => self.shell_state = ShellState::Running,
            OscEvent::CommandEnd { exit_code } => {
                self.last_exit_code = Some(*exit_code);
                self.shell_state = ShellState::Idle;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// OSC byte-stream scanner
// ---------------------------------------------------------------------------

/// State machine that scans raw PTY bytes for OSC sequences across chunk
/// boundaries (split packets).  Handles both BEL (0x07) and ST (ESC `\`)
/// terminators.  Only OSC 0, 2, 7, and 133 are decoded; all others are
/// silently ignored.
#[derive(Default)]
pub struct OscScanner {
    state: ScanState,
}

#[derive(Default)]
enum ScanState {
    #[default]
    Idle,
    /// Saw 0x1B — waiting to see if next byte is `]`.
    SawEsc,
    /// Saw ESC `]` — about to start collecting the OSC body.
    SawBracket,
    /// Inside an OSC sequence.  `buf` accumulates bytes between `]` and the
    /// terminator.  `esc_pending` is true when the last byte was 0x1B (we
    /// don't know yet if the next byte will be `\` to close the sequence).
    CollectingOsc { buf: Vec<u8>, esc_pending: bool },
}

impl OscScanner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a chunk of bytes through the scanner.  Returns any OSC events
    /// that were fully parsed within this chunk (or completed by it if a
    /// previous chunk left the scanner mid-sequence).
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
                    // Immediately terminated — empty OSC, ignore.
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
                            // ESC `\` — String Terminator, OSC complete.
                            b'\\' => {
                                if let Some(ev) = parse_osc(&buf) {
                                    events.push(ev);
                                }
                                ScanState::Idle
                            }
                            // ESC `]` — abandon current sequence, start new OSC.
                            b']' => ScanState::SawBracket,
                            // Not a valid ST — the ESC was part of the body.
                            _ => {
                                buf.push(0x1B);
                                buf.push(b);
                                ScanState::CollectingOsc {
                                    buf,
                                    esc_pending: false,
                                }
                            }
                        }
                    } else {
                        match b {
                            // BEL — OSC complete.
                            0x07 => {
                                if let Some(ev) = parse_osc(&buf) {
                                    events.push(ev);
                                }
                                ScanState::Idle
                            }
                            // Start of potential ST.
                            0x1B => ScanState::CollectingOsc {
                                buf,
                                esc_pending: true,
                            },
                            _ => {
                                buf.push(b);
                                ScanState::CollectingOsc {
                                    buf,
                                    esc_pending: false,
                                }
                            }
                        }
                    }
                }
            };
        }
        events
    }

    /// Reset scanner state (e.g. after a terminal reset sequence).
    pub fn reset(&mut self) {
        self.state = ScanState::Idle;
    }
}

/// Parse a completed OSC body (bytes between `]` and the terminator).
/// The body has the form `<num>;<rest>` where `<rest>` is sequence-specific.
fn parse_osc(buf: &[u8]) -> Option<OscEvent> {
    let sep = buf.iter().position(|&b| b == b';')?;
    let num = &buf[..sep];
    let body = &buf[sep + 1..];

    match num {
        b"0" | b"2" => {
            let title = std::str::from_utf8(body).ok()?.trim().to_owned();
            if title.is_empty() {
                Some(OscEvent::ResetTitle)
            } else {
                Some(OscEvent::Title(title))
            }
        }
        b"7" => {
            let url = std::str::from_utf8(body).ok()?;
            let path = parse_cwd_url(url.trim())?;
            Some(OscEvent::Cwd(path))
        }
        b"133" => parse_osc_133(body),
        _ => None,
    }
}

/// Parse the body of an OSC 133 semantic prompt sequence.
/// Body format: `<mark>[;<params>]` where mark is A / B / C / D.
fn parse_osc_133(body: &[u8]) -> Option<OscEvent> {
    let mark = *body.first()?;
    match mark {
        b'A' | b'B' => Some(OscEvent::PromptReady),
        b'C' => Some(OscEvent::CommandStart),
        b'D' => {
            // Body is `D` or `D;<exit_code>`.
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

/// Extract a filesystem path from an OSC 7 CWD URL.
///
/// Handles:
/// - `file://hostname/path`  → `/path`
/// - `file:///path`          → `/path` (empty hostname / localhost)
/// - `kitty-shell-cwd://hostname/path` → `/path`
pub fn parse_cwd_url(url: &str) -> Option<String> {
    let rest = if let Some(r) = url.strip_prefix("file://") {
        r
    } else if let Some(r) = url.strip_prefix("kitty-shell-cwd://") {
        r
    } else {
        return None;
    };

    // rest is either `/path` (empty/localhost hostname) or `hostname/path`.
    let path = if rest.starts_with('/') {
        rest
    } else {
        rest.find('/').map(|i| &rest[i..])?
    };

    Some(percent_decode(path))
}

/// Minimal percent-decoder for URL path components.
fn percent_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hi = (b[i + 1] as char).to_digit(16);
            let lo = (b[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
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

/// Build a synthetic OSC preamble encoding a cached title and/or CWD.
///
/// Sent by the host as the first `TermOutput` (seq=0) on `TermAttach` so the
/// client can seed its `TerminalMeta` even when those OSC sequences were
/// evicted from the backlog.
pub fn encode_meta_preamble(title: &Option<String>, cwd: &Option<String>) -> Vec<u8> {
    let mut preamble = Vec::new();
    if let Some(t) = title {
        // OSC 2 ; title BEL
        preamble.extend_from_slice(b"\x1b]2;");
        preamble.extend_from_slice(t.as_bytes());
        preamble.push(0x07);
    }
    if let Some(c) = cwd {
        // OSC 7 ; file:// path BEL
        preamble.extend_from_slice(b"\x1b]7;file://");
        preamble.extend_from_slice(c.as_bytes());
        preamble.push(0x07);
    }
    preamble
}
