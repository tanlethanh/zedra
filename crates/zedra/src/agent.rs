use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Kind {
    #[default]
    Shell,
    Amp,
    Claude,
    Cline,
    Codex,
    Copilot,
    Cursor,
    Gemini,
    Goose,
    Hermes,
    Junie,
    KiloCode,
    OpenClaw,
    OpenCode,
    OpenHands,
    Pi,
    Qoder,
    Qwen,
    Trae,
    Zencoder,
}

impl Kind {
    pub fn icon(self) -> Option<&'static str> {
        match self {
            Self::Shell => None,
            Self::Amp => Some("icons/amp.svg"),
            Self::Claude => Some("icons/claude.svg"),
            Self::Cline => Some("icons/cline.svg"),
            Self::Codex => Some("icons/openai.svg"),
            Self::Copilot => Some("icons/githubcopilot.svg"),
            Self::Cursor => Some("icons/cursor.svg"),
            Self::Gemini => Some("icons/gemini.svg"),
            Self::Goose => Some("icons/goose.svg"),
            Self::Hermes => Some("icons/hermesagent.svg"),
            Self::Junie => Some("icons/junie.svg"),
            Self::KiloCode => Some("icons/kilocode.svg"),
            Self::OpenClaw => Some("icons/openclaw.svg"),
            Self::OpenCode => Some("icons/opencode.svg"),
            Self::OpenHands => Some("icons/openhands.svg"),
            Self::Pi => Some("icons/pi.svg"),
            Self::Qoder => Some("icons/qoder.svg"),
            Self::Qwen => Some("icons/qwen.svg"),
            Self::Trae => Some("icons/trae.svg"),
            Self::Zencoder => Some("icons/zencoder.svg"),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Caps {
    pub add: bool,
    pub ask: bool,
    pub diff: bool,
    pub open: bool,
    pub status: bool,
}

impl Caps {
    pub const NONE: Self = Self {
        add: false,
        ask: false,
        diff: false,
        open: false,
        status: false,
    };

    pub const PROMPT: Self = Self {
        add: true,
        ask: true,
        diff: false,
        open: false,
        status: false,
    };

    pub const FULL: Self = Self {
        add: true,
        ask: true,
        diff: true,
        open: true,
        status: true,
    };
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Add {
    pub file: PathBuf,
    pub rel: PathBuf,
    pub start: u32,
    pub end: u32,
    pub text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Ask {
    pub add: Add,
    pub prompt: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Loc {
    pub file: PathBuf,
    pub line: Option<u32>,
    pub column: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diff {
    pub title: Option<String>,
    pub file: Option<PathBuf>,
    pub patch: String,
    pub source: Kind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Status {
    pub source: Kind,
    pub text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Target {
    pub tid: String,
    pub kind: Kind,
    pub title: Option<String>,
    pub cwd: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Pick {
    Add { targets: Vec<Target> },
    Ask { targets: Vec<Target> },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Event {
    Command(String),
    Output(String),
    Osc(String),
    Idle,
    Running,
}

pub trait TermCtx {
    fn tid(&self) -> &str;
    fn cwd(&self) -> Option<&Path>;
    fn write(&mut self, bytes: Vec<u8>) -> Result<()>;
    fn selection(&self) -> Option<&str>;

    fn paste(&mut self, text: &str) -> Result<()> {
        self.write(bracketed_paste(text))
    }
}

pub trait AppCtx {
    fn diff(&mut self, diff: Diff) -> Result<()>;
    fn open(&mut self, loc: Loc) -> Result<()>;
    fn pick(&mut self, pick: Pick) -> Result<Option<String>>;
    fn status(&mut self, status: Status) -> Result<()>;
}

pub trait Adapter {
    fn kind(&self) -> Kind;
    fn caps(&self) -> Caps;

    fn add(&mut self, _input: Add, _term: &mut dyn TermCtx, _app: &mut dyn AppCtx) -> Result<()> {
        bail!("{:?} does not support add", self.kind())
    }

    fn ask(&mut self, _input: Ask, _term: &mut dyn TermCtx, _app: &mut dyn AppCtx) -> Result<()> {
        bail!("{:?} does not support ask", self.kind())
    }

    fn event(
        &mut self,
        _event: Event,
        _term: &mut dyn TermCtx,
        _app: &mut dyn AppCtx,
    ) -> Result<()> {
        Ok(())
    }
}

#[derive(Default)]
pub struct Noop;

impl Adapter for Noop {
    fn kind(&self) -> Kind {
        Kind::Shell
    }

    fn caps(&self) -> Caps {
        Caps::NONE
    }
}

pub struct Unsupported {
    kind: Kind,
}

impl Unsupported {
    fn new(kind: Kind) -> Self {
        Self { kind }
    }
}

impl Adapter for Unsupported {
    fn kind(&self) -> Kind {
        self.kind
    }

    fn caps(&self) -> Caps {
        Caps::NONE
    }
}

#[derive(Default)]
pub struct Claude;

impl Adapter for Claude {
    fn kind(&self) -> Kind {
        Kind::Claude
    }

    fn caps(&self) -> Caps {
        Caps::PROMPT
    }

    fn add(&mut self, input: Add, term: &mut dyn TermCtx, _app: &mut dyn AppCtx) -> Result<()> {
        term.paste(&format_claude_add(&input))
    }

    fn ask(&mut self, input: Ask, term: &mut dyn TermCtx, _app: &mut dyn AppCtx) -> Result<()> {
        term.paste(&format_claude_ask(&input))
    }
}

#[derive(Default)]
pub struct Codex;

impl Adapter for Codex {
    fn kind(&self) -> Kind {
        Kind::Codex
    }

    fn caps(&self) -> Caps {
        Caps::PROMPT
    }

    fn add(&mut self, input: Add, term: &mut dyn TermCtx, _app: &mut dyn AppCtx) -> Result<()> {
        term.paste(&format_codex_add(&input))
    }

    fn ask(&mut self, input: Ask, term: &mut dyn TermCtx, _app: &mut dyn AppCtx) -> Result<()> {
        term.paste(&format_codex_ask(&input))
    }
}

#[derive(Default)]
pub struct OpenCode;

impl Adapter for OpenCode {
    fn kind(&self) -> Kind {
        Kind::OpenCode
    }

    fn caps(&self) -> Caps {
        Caps::PROMPT
    }

    fn add(&mut self, input: Add, term: &mut dyn TermCtx, _app: &mut dyn AppCtx) -> Result<()> {
        term.paste(&format_opencode_add(&input))
    }

    fn ask(&mut self, input: Ask, term: &mut dyn TermCtx, _app: &mut dyn AppCtx) -> Result<()> {
        term.paste(&format_opencode_ask(&input))
    }
}

pub fn make(kind: Kind) -> Box<dyn Adapter> {
    match kind {
        Kind::Shell => Box::<Noop>::default(),
        Kind::Claude => Box::<Claude>::default(),
        Kind::Codex => Box::<Codex>::default(),
        Kind::OpenCode => Box::<OpenCode>::default(),
        Kind::Amp
        | Kind::Cline
        | Kind::Copilot
        | Kind::Cursor
        | Kind::Gemini
        | Kind::Goose
        | Kind::Hermes
        | Kind::Junie
        | Kind::KiloCode
        | Kind::OpenClaw
        | Kind::OpenHands
        | Kind::Pi
        | Kind::Qoder
        | Kind::Qwen
        | Kind::Trae
        | Kind::Zencoder => Box::new(Unsupported::new(kind)),
    }
}

pub fn detect(raw: &str) -> Kind {
    let low = raw.to_ascii_lowercase();
    let words = words(&low);

    if has_any(&words, &["claude", "claudecode"]) {
        Kind::Claude
    } else if has_any(&words, &["opencode"]) || low.contains("open-code") {
        Kind::OpenCode
    } else if has_any(&words, &["amp", "ampcode"]) {
        Kind::Amp
    } else if has_any(&words, &["cline"]) {
        Kind::Cline
    } else if low.contains("cursor-agent")
        || low.contains("cursor agent")
        || has_any(&words, &["cursoragent"])
    {
        Kind::Cursor
    } else if has_any(&words, &["goose"]) {
        Kind::Goose
    } else if low.contains("hermes") || has_any(&words, &["hermesagent"]) {
        Kind::Hermes
    } else if has_any(&words, &["junie"]) {
        Kind::Junie
    } else if has_any(&words, &["kilo", "kilocode"]) {
        Kind::KiloCode
    } else if has_any(&words, &["openclaw"]) {
        Kind::OpenClaw
    } else if has_any(&words, &["openhands"]) {
        Kind::OpenHands
    } else if low.trim() == "pi"
        || low.contains("pi-coding-agent")
        || low.contains("@mariozechner/pi")
        || has_any(&words, &["picodingagent"])
    {
        Kind::Pi
    } else if has_any(&words, &["qoder", "qodercli"]) {
        Kind::Qoder
    } else if has_any(&words, &["qwen", "qwencode"]) {
        Kind::Qwen
    } else if has_any(&words, &["trae", "traecli"]) {
        Kind::Trae
    } else if has_any(&words, &["zencoder", "zenflow"])
        || low.contains("zen cli")
        || low.contains("zen-cli")
    {
        Kind::Zencoder
    } else if has_any(&words, &["gemini", "geminicli"]) {
        Kind::Gemini
    } else if has_any(&words, &["copilot", "githubcopilot"]) {
        Kind::Copilot
    } else if has_any(&words, &["codex"]) || low.trim() == "openai" {
        Kind::Codex
    } else {
        Kind::Shell
    }
}

fn words(raw: &str) -> Vec<&str> {
    raw.split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect()
}

fn has_any(words: &[&str], needles: &[&str]) -> bool {
    words.iter().any(|word| needles.contains(word))
}

pub fn bracketed_paste(text: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(text.len() + 12);
    bytes.extend_from_slice(b"\x1b[200~");
    bytes.extend_from_slice(text.as_bytes());
    bytes.extend_from_slice(b"\x1b[201~");
    bytes
}

fn range(input: &Add) -> String {
    let rel = input.rel.to_string_lossy();
    if input.start == input.end {
        format!("{rel}:L{}", input.start)
    } else {
        format!("{rel}:L{}-L{}", input.start, input.end)
    }
}

fn fenced(input: &Add) -> String {
    format!("{}\n\n```text\n{}\n```", range(input), input.text)
}

fn format_claude_add(input: &Add) -> String {
    fenced(input)
}

fn format_claude_ask(input: &Ask) -> String {
    format!("{}\n\n{}", input.prompt, fenced(&input.add))
}

fn format_codex_add(input: &Add) -> String {
    fenced(input)
}

fn format_codex_ask(input: &Ask) -> String {
    format!("{}\n\n{}", input.prompt, fenced(&input.add))
}

fn format_opencode_add(input: &Add) -> String {
    fenced(input)
}

fn format_opencode_ask(input: &Ask) -> String {
    format!("{}\n\n{}", input.prompt, fenced(&input.add))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Term {
        tid: String,
        cwd: Option<PathBuf>,
        selection: Option<String>,
        writes: Vec<Vec<u8>>,
    }

    impl Term {
        fn new() -> Self {
            Self {
                tid: "term-1".into(),
                cwd: Some(PathBuf::from("/repo")),
                selection: None,
                writes: Vec::new(),
            }
        }
    }

    impl TermCtx for Term {
        fn tid(&self) -> &str {
            &self.tid
        }

        fn cwd(&self) -> Option<&Path> {
            self.cwd.as_deref()
        }

        fn write(&mut self, bytes: Vec<u8>) -> Result<()> {
            self.writes.push(bytes);
            Ok(())
        }

        fn selection(&self) -> Option<&str> {
            self.selection.as_deref()
        }
    }

    struct App;

    impl AppCtx for App {
        fn diff(&mut self, _diff: Diff) -> Result<()> {
            Ok(())
        }

        fn open(&mut self, _loc: Loc) -> Result<()> {
            Ok(())
        }

        fn pick(&mut self, _pick: Pick) -> Result<Option<String>> {
            Ok(None)
        }

        fn status(&mut self, _status: Status) -> Result<()> {
            Ok(())
        }
    }

    fn add_input() -> Add {
        Add {
            file: PathBuf::from("/repo/src/main.rs"),
            rel: PathBuf::from("src/main.rs"),
            start: 10,
            end: 12,
            text: "fn main() {}".into(),
        }
    }

    #[test]
    fn detects_supported_agents() {
        assert_eq!(detect("amp"), Kind::Amp);
        assert_eq!(detect("ampcode"), Kind::Amp);
        assert_eq!(detect("claude"), Kind::Claude);
        assert_eq!(detect("claude-code"), Kind::Claude);
        assert_eq!(detect("cline"), Kind::Cline);
        assert_eq!(detect("npx @openai/codex"), Kind::Codex);
        assert_eq!(detect("github-copilot"), Kind::Copilot);
        assert_eq!(detect("gh copilot suggest"), Kind::Copilot);
        assert_eq!(detect("cursor-agent"), Kind::Cursor);
        assert_eq!(detect("cursor agent"), Kind::Cursor);
        assert_eq!(detect("gemini"), Kind::Gemini);
        assert_eq!(detect("gemini-cli"), Kind::Gemini);
        assert_eq!(detect("goose session"), Kind::Goose);
        assert_eq!(detect("hermes-agent"), Kind::Hermes);
        assert_eq!(detect("junie"), Kind::Junie);
        assert_eq!(detect("kilo"), Kind::KiloCode);
        assert_eq!(detect("kilo-code"), Kind::KiloCode);
        assert_eq!(detect("kilocode"), Kind::KiloCode);
        assert_eq!(detect("open-claw"), Kind::OpenClaw);
        assert_eq!(detect("openclaw tui"), Kind::OpenClaw);
        assert_eq!(detect("open-code run"), Kind::OpenCode);
        assert_eq!(detect("opencode run"), Kind::OpenCode);
        assert_eq!(detect("open-hands"), Kind::OpenHands);
        assert_eq!(detect("openhands"), Kind::OpenHands);
        assert_eq!(detect("pi"), Kind::Pi);
        assert_eq!(detect("npx @mariozechner/pi-coding-agent"), Kind::Pi);
        assert_eq!(detect("qoder-cli"), Kind::Qoder);
        assert_eq!(detect("qodercli"), Kind::Qoder);
        assert_eq!(detect("qwen-code"), Kind::Qwen);
        assert_eq!(detect("trae-agent"), Kind::Trae);
        assert_eq!(detect("trae-cli interactive"), Kind::Trae);
        assert_eq!(detect("zen-cli"), Kind::Zencoder);
        assert_eq!(detect("zen cli"), Kind::Zencoder);
        assert_eq!(detect("zsh"), Kind::Shell);
    }

    #[test]
    fn detection_avoids_partial_tokens() {
        assert_eq!(detect("sample"), Kind::Shell);
        assert_eq!(detect("hermes build"), Kind::Shell);
        assert_eq!(detect("openhanded"), Kind::Shell);
        assert_eq!(detect("cursor ."), Kind::Shell);
        assert_eq!(detect("cline auth -p openai"), Kind::Cline);
        assert_eq!(detect("qwen --provider openai"), Kind::Qwen);
        assert_eq!(detect("qwen --provider gemini"), Kind::Qwen);
        assert_eq!(detect("pip install pytest"), Kind::Shell);
    }

    #[test]
    fn unsupported_adapter_preserves_kind() {
        let adapter = make(Kind::Gemini);
        assert_eq!(adapter.kind(), Kind::Gemini);
        assert_eq!(adapter.caps(), Caps::NONE);
    }

    #[test]
    fn add_pastes_without_submitting() {
        let mut adapter = Claude;
        let mut term = Term::new();
        let mut app = App;

        adapter.add(add_input(), &mut term, &mut app).unwrap();

        let written = String::from_utf8(term.writes.pop().unwrap()).unwrap();
        assert!(written.starts_with("\x1b[200~"));
        assert!(written.ends_with("\x1b[201~"));
        assert!(written.contains("src/main.rs:L10-L12"));
        assert!(written.contains("fn main() {}"));
    }
}
