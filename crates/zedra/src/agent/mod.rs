use std::path::{Path, PathBuf};
use std::{future::Future, pin::Pin};

use anyhow::{Result, bail};
use zedra_rpc::proto::{AgentFile, AgentSessionSummary, AgentSummary};
use zedra_session::SessionHandle;

pub type AdapterFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

// ---------------------------------------------------------------------------
// Asset + display resolution
//
// Agent identity is a host-provided slug string; the app holds no agent enum.
// Icons and display names resolve from the slug, with specialized adapters
// overriding only where branding or behavior differs.
// ---------------------------------------------------------------------------

/// Resolve a slug to a bundled SVG icon path, falling back to the generic
/// terminal icon when the app ships no asset for it. `AssetSource::load`
/// returns nothing for a missing file (GPUI renders blank), so existence must
/// be checked here rather than relying on a runtime fallback.
fn icon_for_slug(slug: &str) -> String {
    let path = format!("icons/{slug}.svg");
    if crate::ZedraAssets::get(&path).is_some() {
        path
    } else {
        "icons/terminal.svg".to_string()
    }
}

/// Strip an `icons/<slug>.svg` bundle path down to its bare `<slug>` — the name
/// the native asset pipeline keys on, identical to the GPUI-rendered slug.
fn icon_slug(path: &str) -> Option<&str> {
    path.strip_prefix("icons/")?.strip_suffix(".svg")
}

/// Human-readable name for a slug the app has no specialized adapter for.
/// Prefer the host's `AgentSummary.display_name` where one is available; this
/// is only a local fallback (terminal titles, pending labels).
fn display_name_for_slug(slug: &str) -> String {
    let mut chars = slug.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => "Agent".to_string(),
    }
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
    pub source: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Status {
    pub source: String,
    pub text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Target {
    pub tid: String,
    pub slug: String,
    pub title: Option<String>,
    pub cwd: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TargetPresentation {
    pub label: String,
    pub image_name: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Pick {
    AddToChat { targets: Vec<Target> },
    Ask { targets: Vec<Target> },
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

pub trait AgentAdapter: Send + Sync {
    /// Stable host-provided identity. The only agent identifier the app keys on.
    fn slug(&self) -> &str;

    fn display_name(&self) -> &str;

    fn icon_path(&self) -> &str {
        "icons/terminal.svg"
    }

    /// Native (iOS/Android) picker asset = the icon slug, so the native bridge
    /// resolves the same `assets/icons/<slug>.svg` GPUI renders. Derived from
    /// `icon_path` so a branding override (Codex -> openai) carries to native;
    /// a path outside `icons/<slug>.svg` yields `None` (label-only).
    fn native_image_name(&self) -> Option<&str> {
        icon_slug(self.icon_path())
    }

    fn should_notify(&self, _event_name: &str) -> bool {
        false
    }

    // Host-driven features are uniform RPC passthroughs keyed by slug; the
    // default methods carry the logic so each adapter inherits it.

    fn info<'a>(
        &'a self,
        handle: &'a SessionHandle,
        refresh: bool,
    ) -> AdapterFuture<'a, Result<AgentSummary>> {
        let slug = self.slug().to_owned();
        Box::pin(async move {
            handle
                .agent_list(refresh)
                .await?
                .into_iter()
                .find(|agent| agent.slug == slug)
                .ok_or_else(|| anyhow::anyhow!("agent {slug} is not available on the host"))
        })
    }

    fn sessions<'a>(
        &'a self,
        handle: &'a SessionHandle,
        refresh: bool,
    ) -> AdapterFuture<'a, Result<Vec<AgentSessionSummary>>> {
        Box::pin(handle.agent_sessions(self.slug().to_owned(), refresh, 0))
    }

    fn files<'a>(&'a self, handle: &'a SessionHandle) -> AdapterFuture<'a, Result<Vec<AgentFile>>> {
        Box::pin(handle.agent_files(self.slug().to_owned()))
    }

    fn resume<'a>(
        &'a self,
        handle: &'a SessionHandle,
        session_id: String,
        cols: u16,
        rows: u16,
    ) -> AdapterFuture<'a, Result<String>> {
        Box::pin(handle.agent_resume_session(self.slug().to_owned(), session_id, cols, rows))
    }

    fn target_presentation(&self, title: &str) -> TargetPresentation {
        match self.native_image_name() {
            Some(image_name) => native_target_presentation(title, image_name),
            None => TargetPresentation {
                label: format!("{} - {}", self.display_name(), title),
                image_name: None,
            },
        }
    }

    // Paste context only; the user remains in control of submitting the prompt.
    fn add_to_chat(
        &mut self,
        input: AddToChat,
        term: &mut dyn TermCtx,
        _app: &mut dyn AppCtx,
    ) -> Result<()> {
        term.paste(&fenced_add_to_chat_context(&input))
    }

    fn ask(&mut self, input: Ask, term: &mut dyn TermCtx, _app: &mut dyn AppCtx) -> Result<()> {
        term.paste(&format!(
            "{}\n\n{}",
            input.prompt,
            fenced_add_to_chat_context(&input.add_to_chat)
        ))
    }
}

fn native_target_presentation(title: &str, image_name: &str) -> TargetPresentation {
    TargetPresentation {
        label: title.to_string(),
        image_name: Some(image_name.to_string()),
    }
}

/// Generic agent with no special in-app behavior. Constructed from a host slug;
/// display name and icon resolve from the slug (or host metadata upstream).
/// Add-to-chat falls back to fenced context paste.
pub struct GenericAdapter {
    slug: String,
    display_name: String,
    icon_path: String,
}

impl GenericAdapter {
    fn new(slug: &str) -> Self {
        Self {
            slug: slug.to_owned(),
            display_name: display_name_for_slug(slug),
            icon_path: icon_for_slug(slug),
        }
    }
}

impl AgentAdapter for GenericAdapter {
    fn slug(&self) -> &str {
        &self.slug
    }

    fn display_name(&self) -> &str {
        &self.display_name
    }

    fn icon_path(&self) -> &str {
        &self.icon_path
    }
}

#[derive(Default)]
pub struct ShellAdapter;

impl AgentAdapter for ShellAdapter {
    fn slug(&self) -> &str {
        "shell"
    }

    fn display_name(&self) -> &str {
        "Shell"
    }

    fn add_to_chat(
        &mut self,
        _input: AddToChat,
        _term: &mut dyn TermCtx,
        _app: &mut dyn AppCtx,
    ) -> Result<()> {
        bail!("Shell does not support Add to Chat")
    }

    fn ask(&mut self, _input: Ask, _term: &mut dyn TermCtx, _app: &mut dyn AppCtx) -> Result<()> {
        bail!("Shell does not support ask")
    }
}

#[derive(Default)]
pub struct ClaudeAdapter;

impl AgentAdapter for ClaudeAdapter {
    fn slug(&self) -> &str {
        "claude"
    }

    fn display_name(&self) -> &str {
        "Claude Code"
    }

    fn icon_path(&self) -> &str {
        "icons/claude.svg"
    }

    fn should_notify(&self, event_name: &str) -> bool {
        matches!(event_name, "Stop" | "PermissionRequest")
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

/// Agent whose only in-app specialization is branding and hook notification
/// events; behavior stays the generic fenced paste.
#[derive(Clone, Copy)]
struct BrandedAdapter {
    slug: &'static str,
    display_name: &'static str,
    icon_path: &'static str,
    notify_events: &'static [&'static str],
}

impl AgentAdapter for BrandedAdapter {
    fn slug(&self) -> &str {
        self.slug
    }

    fn display_name(&self) -> &str {
        self.display_name
    }

    fn icon_path(&self) -> &str {
        self.icon_path
    }

    fn should_notify(&self, event_name: &str) -> bool {
        self.notify_events.contains(&event_name)
    }
}

/// Branding + notify-event table for agents without behavior overrides.
/// Codex prefers the OpenAI brand icon over a codex-named asset.
const BRANDED_ADAPTERS: &[BrandedAdapter] = &[
    BrandedAdapter {
        slug: "codex",
        display_name: "Codex",
        icon_path: "icons/openai.svg",
        notify_events: &["Stop", "PermissionRequest"],
    },
    BrandedAdapter {
        slug: "opencode",
        display_name: "OpenCode",
        icon_path: "icons/opencode.svg",
        notify_events: &["session.idle", "permission.asked"],
    },
    BrandedAdapter {
        slug: "pi",
        display_name: "Pi",
        icon_path: "icons/pi.svg",
        notify_events: &["Stop"],
    },
    BrandedAdapter {
        slug: "hermes",
        display_name: "Hermes Agent",
        icon_path: "icons/hermesagent.svg",
        notify_events: &["post_llm_call", "pre_approval_request"],
    },
];

/// Build the app-side adapter for a host slug. Always returns a working adapter:
/// a specialized one for agents with custom in-app behavior or branding, else a
/// generic adapter seeded from the slug. Adding an agent needs a host actor and
/// assets; an app adapter is only required for custom behavior.
pub fn adapter(slug: &str) -> Box<dyn AgentAdapter> {
    match slug {
        "shell" => Box::<ShellAdapter>::default(),
        "claude" => Box::<ClaudeAdapter>::default(),
        other => match BRANDED_ADAPTERS.iter().find(|entry| entry.slug == other) {
            Some(entry) => Box::new(*entry),
            None => Box::new(GenericAdapter::new(other)),
        },
    }
}

/// Display name for a slug, resolved through its adapter. Host
/// `AgentSummary.display_name` is preferred upstream; this is the local
/// fallback for terminal titles and pending labels.
pub fn name(slug: &str) -> String {
    adapter(slug).display_name().to_owned()
}

/// Icon path for a slug, resolved through its adapter so branding overrides
/// apply (e.g. Codex -> OpenAI icon) with the slug-asset fallback for unknown
/// agents. Memoized: called from render paths, and the generic fallback does an
/// embedded-asset existence lookup per resolution.
pub fn icon(slug: &str) -> String {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Mutex::default);
    if let Some(path) = cache.lock().ok().and_then(|map| map.get(slug).cloned()) {
        return path;
    }
    let path = adapter(slug).icon_path().to_owned();
    if let Ok(mut map) = cache.lock() {
        map.insert(slug.to_owned(), path.clone());
    }
    path
}

pub fn bracketed_paste(text: &str) -> Vec<u8> {
    // Neutralize embedded ESC so content can't smuggle a `\x1b[201~` paste terminator.
    let sanitized = text.replace('\x1b', "␛");
    let mut bytes = Vec::with_capacity(sanitized.len() + 12);
    bytes.extend_from_slice(b"\x1b[200~");
    bytes.extend_from_slice(sanitized.as_bytes());
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

/// Fence longer than the longest backtick run in the payload, so a selection
/// containing ``` cannot close the block early.
fn code_fence(text: &str) -> String {
    let longest_run = text.split(|ch| ch != '`').map(str::len).max().unwrap_or(0);
    "`".repeat((longest_run + 1).max(3))
}

fn fenced_add_to_chat_context(input: &AddToChat) -> String {
    let fence = code_fence(&input.text);
    format!(
        "{}\n\n{fence}text\n{}\n{fence}",
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

    #[test]
    fn adapter_is_infallible_for_unknown_slug() {
        let adapter = adapter("brand-new-agent");
        assert_eq!(adapter.slug(), "brand-new-agent");
        assert_eq!(adapter.display_name(), "Brand-new-agent");
    }

    #[test]
    fn icon_falls_back_to_terminal_for_unbundled_slug() {
        assert_eq!(adapter("claude").icon_path(), "icons/claude.svg");
        // Codex prefers the OpenAI brand icon over a codex-named asset.
        assert_eq!(adapter("codex").icon_path(), "icons/openai.svg");
        // Unbundled slug falls back instead of rendering blank.
        assert_eq!(icon_for_slug("brand-new-agent"), "icons/terminal.svg");
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
    fn claude_add_to_chat_pastes_mention_without_submitting() {
        let mut adapter = adapter("claude");
        let mut term = Term::new();
        let mut app = App;

        adapter
            .add_to_chat(add_to_chat_input(), &mut term, &mut app)
            .unwrap();

        assert_eq!(take_paste_payload(&mut term), "@src/main.rs#L10-L12 ");
    }

    #[test]
    fn claude_add_to_chat_uses_single_line_mention() {
        let mut adapter = adapter("claude");
        let mut term = Term::new();
        let mut app = App;

        adapter
            .add_to_chat(single_line_add_to_chat_input(), &mut term, &mut app)
            .unwrap();

        assert_eq!(take_paste_payload(&mut term), "@src/lib.rs#L7 ");
    }

    #[test]
    fn claude_ask_pastes_prompt_with_mention_without_submitting() {
        let mut adapter = adapter("claude");
        let mut term = Term::new();
        let mut app = App;

        adapter.ask(ask_input(), &mut term, &mut app).unwrap();

        assert_eq!(
            take_paste_payload(&mut term),
            "Please review this range.\n\n@src/main.rs#L10-L12"
        );
    }

    #[test]
    fn generic_and_codex_add_to_chat_paste_fenced_context() {
        for slug in ["codex", "opencode", "brand-new-agent"] {
            let mut adapter = adapter(slug);
            let mut term = Term::new();
            let mut app = App;

            adapter
                .add_to_chat(add_to_chat_input(), &mut term, &mut app)
                .unwrap();

            assert_eq!(
                take_paste_payload(&mut term),
                "src/main.rs:L10-L12\n\n```text\nfn main() {}\n```",
                "{slug}"
            );
        }
    }

    #[test]
    fn fenced_context_uses_single_line_range() {
        let mut adapter = adapter("codex");
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
    fn fenced_context_outruns_backticks_in_selection() {
        // A selection containing ``` must not close the fence early.
        let input = AddToChat {
            file: PathBuf::from("/repo/docs/README.md"),
            rel: PathBuf::from("docs/README.md"),
            start: 1,
            end: 3,
            text: "```sh\nls\n```".to_string(),
        };
        assert_eq!(
            fenced_add_to_chat_context(&input),
            "docs/README.md:L1-L3\n\n````text\n```sh\nls\n```\n````"
        );
    }

    #[test]
    fn native_image_name_is_the_icon_slug() {
        // Native asset name is the bare icon slug, so the native bridge resolves
        // the same `assets/icons/<slug>.svg` that GPUI renders.
        assert_eq!(adapter("claude").native_image_name(), Some("claude"));
        // Branding override on icon_path carries through to native too.
        assert_eq!(adapter("codex").native_image_name(), Some("openai"));
        assert_eq!(
            adapter("codex").target_presentation("codex"),
            TargetPresentation {
                label: "codex".into(),
                image_name: Some("openai".into()),
            }
        );
    }
}
