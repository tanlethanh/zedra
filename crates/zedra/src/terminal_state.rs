use std::collections::HashMap;

use zedra_rpc::proto::{AgentState, TerminalSyncEntry};

use crate::agent;

pub use zedra_rpc::proto::TermShellState as ShellState;

#[derive(Clone, Default, Debug)]
pub struct TerminalMeta {
    /// Raw OSC title.
    pub title: Option<String>,
    /// Title with emoji/activity glyphs stripped for compact picker labels.
    pub plain_title: Option<String>,
    pub cwd: Option<String>,
    pub last_exit_code: Option<i32>,
    pub shell_state: ShellState,
    /// Last command from OSC 633;E.
    pub current_command: Option<String>,
    pub agent_icon: Option<String>,
    pub agent_slug: Option<String>,
    pub agent_state: AgentState,
}

/// Live terminal metadata (title, cwd, shell state) keyed by terminal ID.
/// Written from OSC events, read by the terminal/quick-action panels.
pub struct TerminalState {
    entries: HashMap<String, TerminalMeta>,
}

impl TerminalState {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn remove(&mut self, id: &str) {
        self.entries.remove(id);
    }

    pub fn meta(&self, id: &str) -> TerminalMeta {
        self.entries.get(id).cloned().unwrap_or_default()
    }

    pub fn set_title(&mut self, id: &str, title: Option<String>) {
        let plain_title = title.as_deref().and_then(plain_terminal_title);
        let e = self.entry(id);
        e.title = title;
        e.plain_title = plain_title;
    }

    pub fn set_cwd(&mut self, id: &str, cwd: String) {
        self.entry(id).cwd = Some(cwd);
    }

    pub fn set_agent_state(&mut self, id: &str, state: AgentState) {
        self.entry(id).agent_state = state;
    }

    pub fn set_shell_running(&mut self, id: &str) {
        self.entry(id).shell_state = ShellState::Running;
    }

    /// Apply host-resolved agent identity (`None` clears). Sets icon from slug;
    /// defaults title to the display name when the terminal has none.
    pub fn set_agent_slug(&mut self, id: &str, slug: Option<String>) {
        // Adapter applies branding overrides (e.g. Codex -> OpenAI) + slug-asset fallback.
        let resolved = slug.as_deref().map(|slug| {
            let adapter = agent::adapter(slug);
            (
                adapter.icon_path().to_owned(),
                adapter.display_name().to_owned(),
            )
        });
        let icon = resolved.as_ref().map(|(icon, _)| icon.clone());
        let default_title = resolved.map(|(_, name)| name);
        let e = self.entry(id);
        e.agent_slug = slug;
        e.agent_icon = icon;
        if e.title.is_none()
            && let Some(title) = default_title
        {
            e.title = Some(title.clone());
            e.plain_title = plain_terminal_title(&title);
        }
    }

    pub fn set_command_started(&mut self, id: &str) {
        self.entry(id).shell_state = ShellState::Running;
    }

    pub fn set_shell_idle(&mut self, id: &str, exit_code: Option<i32>) {
        self.mark_shell_idle(id, exit_code);
    }

    pub fn set_prompt_ready(&mut self, id: &str) {
        // OSC 133;A = prompt shown. Identity is host-driven, so only shell state updates.
        self.mark_shell_idle(id, None);
    }

    fn mark_shell_idle(&mut self, id: &str, exit_code: Option<i32>) {
        let e = self.entry(id);
        e.shell_state = ShellState::Idle;
        if let Some(code) = exit_code {
            e.last_exit_code = Some(code);
        }
        e.current_command = None;
    }

    pub fn set_current_command(&mut self, id: &str, command: String) {
        self.entry(id).current_command = Some(command);
    }

    /// Seed terminal metadata from the host sync snapshot (source of truth on
    /// reconnect; live OSC events / `TerminalAgentChanged` update from here).
    pub fn seed_host_meta(&mut self, sync: &TerminalSyncEntry) {
        let id = &sync.id;
        self.set_title(id, sync.title.clone());
        if let Some(cwd) = &sync.cwd {
            self.set_cwd(id, cwd.clone());
        }

        let e = self.entry(id);
        e.shell_state = sync.shell_state;
        // current_command only meaningful while running; gate on shell state.
        e.current_command = if sync.shell_state == ShellState::Running {
            sync.agent_command.clone()
        } else {
            None
        };
        if sync.last_exit_code.is_some() {
            e.last_exit_code = sync.last_exit_code;
        }
        e.agent_state = sync.agent_state;

        // Host-resolved identity is authoritative.
        self.set_agent_slug(id, sync.agent_slug.clone());
    }

    fn entry(&mut self, id: &str) -> &mut TerminalMeta {
        self.entries.entry(id.to_string()).or_default()
    }
}

fn plain_terminal_title(title: &str) -> Option<String> {
    let mut plain = String::with_capacity(title.len());
    let mut pending_space = false;

    for ch in title.chars() {
        if is_terminal_title_decoration(ch) {
            pending_space = true;
            continue;
        }

        if ch.is_whitespace() {
            pending_space = true;
            continue;
        }

        if pending_space && !plain.is_empty() {
            plain.push(' ');
        }
        plain.push(ch);
        pending_space = false;
    }

    (!plain.is_empty()).then_some(plain)
}

fn is_terminal_title_decoration(ch: char) -> bool {
    let code = ch as u32;
    matches!(
        code,
        0x00A9
            | 0x00AE
            | 0x203C
            | 0x2049
            | 0x20E3
            | 0x2122
            | 0x2139
            | 0x231A..=0x231B
            | 0x2328
            | 0x23CF
            | 0x23E9..=0x23F3
            | 0x23F8..=0x23FA
            | 0x24C2
            | 0x25AA..=0x25AB
            | 0x25B6
            | 0x25C0
            | 0x25FB..=0x25FE
            | 0x2600..=0x27BF
            | 0x2800..=0x28FF
            | 0x2B1B..=0x2B1C
            | 0x2B50
            | 0x2B55
            | 0x3030
            | 0x303D
            | 0x3297
            | 0x3299
            | 0xFE00..=0xFE0F
            | 0x1F000..=0x1FAFF
            | 0xE0020..=0xE007F
    ) || ch == '\u{200d}'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_agent_slug_resolves_icon_and_defaults_title() {
        let mut state = TerminalState::new();
        let id = "term-1";

        state.set_agent_slug(id, Some("codex".to_owned()));
        let meta = state.meta(id);
        assert_eq!(meta.agent_slug.as_deref(), Some("codex"));
        assert_eq!(meta.agent_icon.as_deref(), Some("icons/openai.svg"));
        assert_eq!(meta.title.as_deref(), Some("Codex"));
    }

    #[test]
    fn set_agent_slug_none_clears_identity() {
        let mut state = TerminalState::new();
        let id = "term-1";

        state.set_agent_slug(id, Some("claude".to_owned()));
        assert!(state.meta(id).agent_icon.is_some());

        state.set_agent_slug(id, None);
        let meta = state.meta(id);
        assert_eq!(meta.agent_slug, None);
        assert_eq!(meta.agent_icon, None);
    }

    #[test]
    fn unknown_agent_slug_falls_back_to_terminal_icon() {
        let mut state = TerminalState::new();
        let id = "term-1";

        state.set_agent_slug(id, Some("brand-new-agent".to_owned()));
        assert_eq!(
            state.meta(id).agent_icon.as_deref(),
            Some("icons/terminal.svg")
        );
    }

    #[test]
    fn live_title_overrides_default_agent_title() {
        let mut state = TerminalState::new();
        let id = "term-1";

        state.set_agent_slug(id, Some("hermes".to_owned()));
        assert_eq!(state.meta(id).title.as_deref(), Some("Hermes Agent"));

        state.set_title(id, Some("Reviewing auth flow".to_owned()));
        assert_eq!(state.meta(id).title.as_deref(), Some("Reviewing auth flow"));
    }

    #[test]
    fn sync_seed_sets_host_resolved_identity() {
        let mut state = TerminalState::new();
        let sync = TerminalSyncEntry {
            id: "term-1".to_owned(),
            title: Some("Reviewing auth flow".to_owned()),
            cwd: Some("/repo".to_owned()),
            agent_command: Some("codex".to_owned()),
            agent_slug: Some("codex".to_owned()),
            shell_state: ShellState::Idle,
            last_exit_code: Some(0),
            ..Default::default()
        };

        state.seed_host_meta(&sync);

        let meta = state.meta("term-1");
        assert_eq!(meta.agent_slug.as_deref(), Some("codex"));
        assert_eq!(meta.agent_icon.as_deref(), Some("icons/openai.svg"));
        assert_eq!(meta.shell_state, ShellState::Idle);
        assert_eq!(meta.title.as_deref(), Some("Reviewing auth flow"));
    }

    #[test]
    fn sync_seed_without_agent_slug_has_no_identity() {
        let mut state = TerminalState::new();
        let sync = TerminalSyncEntry {
            id: "term-1".to_owned(),
            title: Some("zsh".to_owned()),
            shell_state: ShellState::Idle,
            last_exit_code: Some(0),
            ..Default::default()
        };

        state.seed_host_meta(&sync);

        let meta = state.meta("term-1");
        assert_eq!(meta.agent_slug, None);
        assert_eq!(meta.agent_icon, None);
    }

    #[test]
    fn title_updates_store_raw_and_plain_title_without_emoji() {
        let mut state = TerminalState::new();
        let id = "term-1";

        state.set_title(id, Some("🤖  Fix auth flow 🚀".to_owned()));

        let meta = state.meta(id);
        assert_eq!(meta.title.as_deref(), Some("🤖  Fix auth flow 🚀"));
        assert_eq!(meta.plain_title.as_deref(), Some("Fix auth flow"));
    }

    #[test]
    fn plain_title_removes_joined_emoji_and_terminal_spinners() {
        let mut state = TerminalState::new();
        let id = "term-1";

        state.set_title(id, Some("⠋ 👩‍💻  zedra".to_owned()));

        let meta = state.meta(id);
        assert_eq!(meta.title.as_deref(), Some("⠋ 👩‍💻  zedra"));
        assert_eq!(meta.plain_title.as_deref(), Some("zedra"));
    }

    #[test]
    fn plain_title_is_none_when_title_is_only_decoration() {
        let mut state = TerminalState::new();
        let id = "term-1";

        state.set_title(id, Some(" ✨ 🚀 ".to_owned()));

        let meta = state.meta(id);
        assert_eq!(meta.title.as_deref(), Some(" ✨ 🚀 "));
        assert_eq!(meta.plain_title, None);
    }
}
