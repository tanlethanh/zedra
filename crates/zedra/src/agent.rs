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
pub struct AgentCaps {
    pub add_to_chat: bool,
    pub ask: bool,
    pub diff: bool,
    pub open: bool,
    pub status: bool,
}

impl AgentCaps {
    pub const NONE: Self = Self {
        add_to_chat: false,
        ask: false,
        diff: false,
        open: false,
        status: false,
    };

    pub const PROMPT: Self = Self {
        add_to_chat: true,
        ask: true,
        diff: false,
        open: false,
        status: false,
    };

    pub const FULL: Self = Self {
        add_to_chat: true,
        ask: true,
        diff: true,
        open: true,
        status: true,
    };
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AddToChat {
    pub file: PathBuf,
    pub rel: PathBuf,
    pub start: u32,
    pub end: u32,
    pub text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Ask {
    pub add_to_chat: AddToChat,
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
pub struct TargetPresentation {
    pub label: String,
    pub image_name: Option<&'static str>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Pick {
    AddToChat { targets: Vec<Target> },
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

pub trait AgentAdapter {
    fn kind(&self) -> Kind;
    fn display_name(&self) -> &'static str;
    fn caps(&self) -> AgentCaps;

    fn target_presentation(&self, title: &str) -> TargetPresentation {
        TargetPresentation {
            label: format!("{} - {}", self.display_name(), title),
            image_name: None,
        }
    }

    // Paste context only; the user remains in control of submitting the prompt.
    fn add_to_chat(
        &mut self,
        _input: AddToChat,
        _term: &mut dyn TermCtx,
        _app: &mut dyn AppCtx,
    ) -> Result<()> {
        bail!("{:?} does not support Add to Chat", self.kind())
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

fn native_target_presentation(title: &str, image_name: &'static str) -> TargetPresentation {
    TargetPresentation {
        label: title.to_string(),
        image_name: Some(image_name),
    }
}

fn paste_fenced_add_to_chat(input: &AddToChat, term: &mut dyn TermCtx) -> Result<()> {
    term.paste(&fenced_add_to_chat_context(input))
}

fn paste_fenced_ask(input: Ask, term: &mut dyn TermCtx) -> Result<()> {
    term.paste(&format!(
        "{}\n\n{}",
        input.prompt,
        fenced_add_to_chat_context(&input.add_to_chat)
    ))
}

#[derive(Default)]
pub struct ShellAdapter;

impl AgentAdapter for ShellAdapter {
    fn kind(&self) -> Kind {
        Kind::Shell
    }

    fn display_name(&self) -> &'static str {
        "Shell"
    }

    fn caps(&self) -> AgentCaps {
        AgentCaps::NONE
    }
}

pub struct GenericPromptAgentAdapter {
    kind: Kind,
}

impl GenericPromptAgentAdapter {
    fn new(kind: Kind) -> Self {
        Self { kind }
    }

    fn display_name_for(kind: Kind) -> &'static str {
        match kind {
            Kind::Shell => "Shell",
            Kind::Amp => "Amp",
            Kind::Claude => "Claude Code",
            Kind::Cline => "Cline",
            Kind::Codex => "Codex",
            Kind::Copilot => "GitHub Copilot",
            Kind::Cursor => "Cursor Agent",
            Kind::Gemini => "Gemini",
            Kind::Goose => "Goose",
            Kind::Hermes => "Hermes Agent",
            Kind::Junie => "Junie",
            Kind::KiloCode => "Kilo Code",
            Kind::OpenClaw => "OpenClaw",
            Kind::OpenCode => "OpenCode",
            Kind::OpenHands => "OpenHands",
            Kind::Pi => "Pi",
            Kind::Qoder => "Qoder",
            Kind::Qwen => "Qwen Code",
            Kind::Trae => "Trae Agent",
            Kind::Zencoder => "Zencoder",
        }
    }

    fn native_image_name(&self) -> Option<&'static str> {
        match self.kind {
            Kind::Shell => None,
            Kind::Amp => Some("AgentAmp"),
            Kind::Claude => Some("AgentClaude"),
            Kind::Cline => Some("AgentCline"),
            Kind::Codex => Some("AgentCodex"),
            Kind::Copilot => Some("AgentCopilot"),
            Kind::Cursor => Some("AgentCursor"),
            Kind::Gemini => Some("AgentGemini"),
            Kind::Goose => Some("AgentGoose"),
            Kind::Hermes => Some("AgentHermes"),
            Kind::Junie => Some("AgentJunie"),
            Kind::KiloCode => Some("AgentKiloCode"),
            Kind::OpenClaw => Some("AgentOpenClaw"),
            Kind::OpenCode => Some("AgentOpenCode"),
            Kind::OpenHands => Some("AgentOpenHands"),
            Kind::Pi => Some("AgentPi"),
            Kind::Qoder => Some("AgentQoder"),
            Kind::Qwen => Some("AgentQwen"),
            Kind::Trae => Some("AgentTrae"),
            Kind::Zencoder => Some("AgentZencoder"),
        }
    }
}

impl AgentAdapter for GenericPromptAgentAdapter {
    fn kind(&self) -> Kind {
        self.kind
    }

    fn display_name(&self) -> &'static str {
        Self::display_name_for(self.kind)
    }

    fn caps(&self) -> AgentCaps {
        AgentCaps::PROMPT
    }

    fn target_presentation(&self, title: &str) -> TargetPresentation {
        if let Some(image_name) = self.native_image_name() {
            native_target_presentation(title, image_name)
        } else {
            TargetPresentation {
                label: title.to_string(),
                image_name: None,
            }
        }
    }

    fn add_to_chat(
        &mut self,
        input: AddToChat,
        term: &mut dyn TermCtx,
        _app: &mut dyn AppCtx,
    ) -> Result<()> {
        paste_fenced_add_to_chat(&input, term)
    }

    fn ask(&mut self, input: Ask, term: &mut dyn TermCtx, _app: &mut dyn AppCtx) -> Result<()> {
        paste_fenced_ask(input, term)
    }
}

#[derive(Default)]
pub struct ClaudeAdapter;

impl AgentAdapter for ClaudeAdapter {
    fn kind(&self) -> Kind {
        Kind::Claude
    }

    fn display_name(&self) -> &'static str {
        "Claude Code"
    }

    fn caps(&self) -> AgentCaps {
        AgentCaps::PROMPT
    }

    fn target_presentation(&self, title: &str) -> TargetPresentation {
        native_target_presentation(title, "AgentClaude")
    }

    fn add_to_chat(
        &mut self,
        input: AddToChat,
        term: &mut dyn TermCtx,
        _app: &mut dyn AppCtx,
    ) -> Result<()> {
        term.paste(&format!("{} ", Self::mention(&input)))
    }

    fn ask(&mut self, input: Ask, term: &mut dyn TermCtx, _app: &mut dyn AppCtx) -> Result<()> {
        term.paste(&format!(
            "{}\n\n{}",
            input.prompt,
            Self::mention(&input.add_to_chat)
        ))
    }
}

impl ClaudeAdapter {
    fn mention(input: &AddToChat) -> String {
        let rel = input.rel.to_string_lossy();
        if input.start == input.end {
            format!("@{rel}#L{}", input.start)
        } else {
            format!("@{rel}#L{}-L{}", input.start, input.end)
        }
    }
}

#[derive(Default)]
pub struct CodexAdapter;

impl AgentAdapter for CodexAdapter {
    fn kind(&self) -> Kind {
        Kind::Codex
    }

    fn display_name(&self) -> &'static str {
        "Codex"
    }

    fn caps(&self) -> AgentCaps {
        AgentCaps::PROMPT
    }

    fn target_presentation(&self, title: &str) -> TargetPresentation {
        native_target_presentation(title, "AgentCodex")
    }

    fn add_to_chat(
        &mut self,
        input: AddToChat,
        term: &mut dyn TermCtx,
        _app: &mut dyn AppCtx,
    ) -> Result<()> {
        paste_fenced_add_to_chat(&input, term)
    }

    fn ask(&mut self, input: Ask, term: &mut dyn TermCtx, _app: &mut dyn AppCtx) -> Result<()> {
        paste_fenced_ask(input, term)
    }
}

#[derive(Default)]
pub struct OpenCodeAdapter;

impl AgentAdapter for OpenCodeAdapter {
    fn kind(&self) -> Kind {
        Kind::OpenCode
    }

    fn display_name(&self) -> &'static str {
        "OpenCode"
    }

    fn caps(&self) -> AgentCaps {
        AgentCaps::PROMPT
    }

    fn target_presentation(&self, title: &str) -> TargetPresentation {
        native_target_presentation(title, "AgentOpenCode")
    }

    fn add_to_chat(
        &mut self,
        input: AddToChat,
        term: &mut dyn TermCtx,
        _app: &mut dyn AppCtx,
    ) -> Result<()> {
        paste_fenced_add_to_chat(&input, term)
    }

    fn ask(&mut self, input: Ask, term: &mut dyn TermCtx, _app: &mut dyn AppCtx) -> Result<()> {
        paste_fenced_ask(input, term)
    }
}

pub fn make_adapter(kind: Kind) -> Box<dyn AgentAdapter> {
    match kind {
        Kind::Shell => Box::<ShellAdapter>::default(),
        Kind::Claude => Box::<ClaudeAdapter>::default(),
        Kind::Codex => Box::<CodexAdapter>::default(),
        Kind::OpenCode => Box::<OpenCodeAdapter>::default(),
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
        | Kind::Zencoder => Box::new(GenericPromptAgentAdapter::new(kind)),
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
    } else if low.contains("hermes-agent")
        || low.contains("hermes agent")
        || has_any(&words, &["hermesagent"])
    {
        Kind::Hermes
    } else if has_any(&words, &["junie"]) {
        Kind::Junie
    } else if has_any(&words, &["kilo", "kilocode"]) {
        Kind::KiloCode
    } else if low.contains("open-claw") || has_any(&words, &["openclaw"]) {
        Kind::OpenClaw
    } else if low.contains("open-hands") || has_any(&words, &["openhands"]) {
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

fn source_range_label(input: &AddToChat) -> String {
    let rel = input.rel.to_string_lossy();
    if input.start == input.end {
        format!("{rel}:L{}", input.start)
    } else {
        format!("{rel}:L{}-L{}", input.start, input.end)
    }
}

fn fenced_add_to_chat_context(input: &AddToChat) -> String {
    format!(
        "{}\n\n```text\n{}\n```",
        source_range_label(input),
        input.text
    )
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

    fn add_to_chat_input() -> AddToChat {
        AddToChat {
            file: PathBuf::from("/repo/src/main.rs"),
            rel: PathBuf::from("src/main.rs"),
            start: 10,
            end: 12,
            text: "fn main() {}".into(),
        }
    }

    fn single_line_add_to_chat_input() -> AddToChat {
        AddToChat {
            file: PathBuf::from("/repo/src/lib.rs"),
            rel: PathBuf::from("src/lib.rs"),
            start: 7,
            end: 7,
            text: "let value = 1;".into(),
        }
    }

    fn ask_input() -> Ask {
        Ask {
            add_to_chat: add_to_chat_input(),
            prompt: "Please review this range.".into(),
        }
    }

    fn take_paste_payload(term: &mut Term) -> String {
        let written = String::from_utf8(term.writes.pop().unwrap()).unwrap();
        assert!(written.starts_with("\x1b[200~"));
        assert!(written.ends_with("\x1b[201~"));
        written
            .trim_start_matches("\x1b[200~")
            .trim_end_matches("\x1b[201~")
            .to_string()
    }

    const AGENT_KINDS: &[Kind] = &[
        Kind::Amp,
        Kind::Claude,
        Kind::Cline,
        Kind::Codex,
        Kind::Copilot,
        Kind::Cursor,
        Kind::Gemini,
        Kind::Goose,
        Kind::Hermes,
        Kind::Junie,
        Kind::KiloCode,
        Kind::OpenClaw,
        Kind::OpenCode,
        Kind::OpenHands,
        Kind::Pi,
        Kind::Qoder,
        Kind::Qwen,
        Kind::Trae,
        Kind::Zencoder,
    ];

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
    fn generic_prompt_adapter_preserves_kind() {
        let adapter = make_adapter(Kind::Gemini);
        assert_eq!(adapter.kind(), Kind::Gemini);
        assert_eq!(adapter.caps(), AgentCaps::PROMPT);
    }

    #[test]
    fn add_to_chat_caps_cover_all_detected_agents() {
        for kind in AGENT_KINDS {
            assert!(
                make_adapter(*kind).caps().add_to_chat,
                "{kind:?} should support Add to Chat"
            );
        }
        assert!(!make_adapter(Kind::Shell).caps().add_to_chat);
    }

    #[test]
    fn shell_adapter_rejects_add_to_chat() {
        let mut adapter = ShellAdapter;
        let mut term = Term::new();
        let mut app = App;

        let error = adapter
            .add_to_chat(add_to_chat_input(), &mut term, &mut app)
            .unwrap_err();

        assert!(error.to_string().contains("does not support Add to Chat"));
        assert!(term.writes.is_empty());
    }

    #[test]
    fn supported_adapters_present_targets_with_native_icons() {
        assert_eq!(
            ClaudeAdapter.target_presentation("claude"),
            TargetPresentation {
                label: "claude".into(),
                image_name: Some("AgentClaude"),
            }
        );
        assert_eq!(
            CodexAdapter.target_presentation("codex"),
            TargetPresentation {
                label: "codex".into(),
                image_name: Some("AgentCodex"),
            }
        );
        assert_eq!(
            OpenCodeAdapter.target_presentation("opencode"),
            TargetPresentation {
                label: "opencode".into(),
                image_name: Some("AgentOpenCode"),
            }
        );

        assert_eq!(
            make_adapter(Kind::Gemini).target_presentation("gemini"),
            TargetPresentation {
                label: "gemini".into(),
                image_name: Some("AgentGemini"),
            }
        );
    }

    #[test]
    fn all_detected_agents_present_native_icons() {
        for kind in AGENT_KINDS {
            let presentation = make_adapter(*kind).target_presentation("terminal title");
            assert_eq!(presentation.label, "terminal title", "{kind:?}");
            assert!(
                presentation.image_name.is_some(),
                "{kind:?} should have a native picker icon"
            );
        }
    }

    #[test]
    fn claude_add_to_chat_pastes_mention_without_submitting() {
        let mut adapter = ClaudeAdapter;
        let mut term = Term::new();
        let mut app = App;

        adapter
            .add_to_chat(add_to_chat_input(), &mut term, &mut app)
            .unwrap();

        assert_eq!(take_paste_payload(&mut term), "@src/main.rs#L10-L12 ");
    }

    #[test]
    fn claude_add_to_chat_uses_single_line_mention() {
        let mut adapter = ClaudeAdapter;
        let mut term = Term::new();
        let mut app = App;

        adapter
            .add_to_chat(single_line_add_to_chat_input(), &mut term, &mut app)
            .unwrap();

        assert_eq!(take_paste_payload(&mut term), "@src/lib.rs#L7 ");
    }

    #[test]
    fn claude_ask_pastes_prompt_with_mention_without_submitting() {
        let mut adapter = ClaudeAdapter;
        let mut term = Term::new();
        let mut app = App;

        adapter.ask(ask_input(), &mut term, &mut app).unwrap();

        assert_eq!(
            take_paste_payload(&mut term),
            "Please review this range.\n\n@src/main.rs#L10-L12"
        );
    }

    #[test]
    fn codex_add_to_chat_pastes_fenced_context_without_submitting() {
        let mut adapter = CodexAdapter;
        let mut term = Term::new();
        let mut app = App;

        adapter
            .add_to_chat(add_to_chat_input(), &mut term, &mut app)
            .unwrap();

        assert_eq!(
            take_paste_payload(&mut term),
            "src/main.rs:L10-L12\n\n```text\nfn main() {}\n```"
        );
    }

    #[test]
    fn codex_ask_pastes_prompt_with_fenced_context_without_submitting() {
        let mut adapter = CodexAdapter;
        let mut term = Term::new();
        let mut app = App;

        adapter.ask(ask_input(), &mut term, &mut app).unwrap();

        assert_eq!(
            take_paste_payload(&mut term),
            "Please review this range.\n\nsrc/main.rs:L10-L12\n\n```text\nfn main() {}\n```"
        );
    }

    #[test]
    fn fenced_context_uses_single_line_range() {
        let mut adapter = CodexAdapter;
        let mut term = Term::new();
        let mut app = App;

        adapter
            .add_to_chat(single_line_add_to_chat_input(), &mut term, &mut app)
            .unwrap();

        assert_eq!(
            take_paste_payload(&mut term),
            "src/lib.rs:L7\n\n```text\nlet value = 1;\n```"
        );
    }

    #[test]
    fn opencode_add_to_chat_pastes_fenced_context_without_submitting() {
        let mut adapter = OpenCodeAdapter;
        let mut term = Term::new();
        let mut app = App;

        adapter
            .add_to_chat(add_to_chat_input(), &mut term, &mut app)
            .unwrap();

        assert_eq!(
            take_paste_payload(&mut term),
            "src/main.rs:L10-L12\n\n```text\nfn main() {}\n```"
        );
    }

    #[test]
    fn all_detected_agents_add_to_chat_without_submitting() {
        for kind in AGENT_KINDS {
            let mut adapter = make_adapter(*kind);
            let mut term = Term::new();
            let mut app = App;

            adapter
                .add_to_chat(add_to_chat_input(), &mut term, &mut app)
                .unwrap();

            let payload = take_paste_payload(&mut term);
            if *kind == Kind::Claude {
                assert_eq!(payload, "@src/main.rs#L10-L12 ", "{kind:?}");
            } else {
                assert_eq!(
                    payload, "src/main.rs:L10-L12\n\n```text\nfn main() {}\n```",
                    "{kind:?}"
                );
            }
        }
    }
}
