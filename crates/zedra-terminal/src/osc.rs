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
    SawEsc,
    SawBracket,
    CollectingOsc {
        buf: Vec<u8>,
        esc_pending: bool,
    },
}

/// An event decoded from an OSC escape sequence.
#[derive(Debug, Clone)]
pub enum OscEvent {
    Title(String),
    ResetTitle,
    Cwd(String),
    PromptReady,
    CommandStart,
    CommandEnd { exit_code: i32 },
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
                                if let Some(ev) = parse_osc(&buf) {
                                    events.push(ev);
                                }
                                ScanState::Idle
                            }
                            b']' => ScanState::SawBracket,
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
                            0x07 => {
                                if let Some(ev) = parse_osc(&buf) {
                                    events.push(ev);
                                }
                                ScanState::Idle
                            }
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

    pub fn reset(&mut self) {
        self.state = ScanState::Idle;
    }
}

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

fn parse_osc_133(body: &[u8]) -> Option<OscEvent> {
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
