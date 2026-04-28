use std::collections::HashMap;

use crate::agent;

#[derive(Clone, Default, Debug, PartialEq)]
pub enum ShellState {
    #[default]
    Unknown,
    Idle,
    Running,
}

#[derive(Clone, Default, Debug)]
pub struct TerminalMeta {
    /// Raw terminal title as reported by OSC title updates.
    pub title: Option<String>,
    /// Title with emoji and transient activity glyphs removed for compact picker labels.
    pub plain_title: Option<String>,
    pub cwd: Option<String>,
    pub last_exit_code: Option<i32>,
    pub shell_state: ShellState,
    /// Last command line reported by OSC 633;E. Cleared when the shell returns to prompt.
    pub current_command: Option<String>,
    /// Agent icon latched for the active command. Cleared when the shell returns to prompt.
    pub agent_icon: Option<&'static str>,
    pub agent_kind: Option<agent::Kind>,
    pub agent_source: AgentIdentitySource,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AgentIdentitySource {
    #[default]
    CommandLine,
    IconName,
}

/// App-level entity that holds live terminal metadata (title, cwd, shell state)
/// keyed by terminal ID. Updated by WorkspaceTerminal as OSC events arrive;
/// read by TerminalPanel and QuickActionPanel for rendering cards.
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

    pub fn set_shell_running(&mut self, id: &str) {
        let e = self.entry(id);
        let kind = if e.agent_source == AgentIdentitySource::IconName {
            e.agent_kind.unwrap_or_default()
        } else {
            e.current_command
                .as_deref()
                .map(agent_kind_from_command)
                .unwrap_or_default()
        };
        let agent_icon = if kind == agent::Kind::Shell {
            None
        } else {
            kind.icon()
        };
        e.shell_state = ShellState::Running;
        if e.agent_source == AgentIdentitySource::CommandLine {
            e.agent_kind = (kind != agent::Kind::Shell).then_some(kind);
            e.agent_icon = agent_icon;
        }
    }

    pub fn set_shell_idle(&mut self, id: &str, exit_code: Option<i32>) {
        self.mark_shell_idle(id, exit_code);
    }

    pub fn set_prompt_ready(&mut self, id: &str) {
        self.mark_shell_idle(id, None);
    }

    fn mark_shell_idle(&mut self, id: &str, exit_code: Option<i32>) {
        let e = self.entry(id);
        e.shell_state = ShellState::Idle;
        if let Some(code) = exit_code {
            e.last_exit_code = Some(code);
        }
        e.current_command = None;
        e.agent_icon = None;
        e.agent_kind = None;
    }

    pub fn set_current_command(&mut self, id: &str, command: String) {
        let kind = agent_kind_from_command(&command);
        let agent_icon = kind.icon();
        let e = self.entry(id);
        let latched = e.shell_state == ShellState::Running;
        let ignored = e.agent_source == AgentIdentitySource::IconName;
        e.current_command = Some(command);
        if latched && !ignored {
            e.agent_kind = (kind != agent::Kind::Shell).then_some(kind);
            e.agent_icon = agent_icon;
        }
    }

    pub fn set_icon_name(&mut self, id: &str, icon_name: String) {
        let kind = agent_kind_from_command(&icon_name);
        let agent_icon = kind.icon();
        let e = self.entry(id);
        e.agent_source = AgentIdentitySource::IconName;

        if let Some(icon) = agent_icon {
            e.shell_state = ShellState::Running;
            e.agent_icon = Some(icon);
            e.agent_kind = Some(kind);
        } else {
            e.shell_state = ShellState::Idle;
            e.agent_icon = None;
            e.agent_kind = None;
        }
    }

    fn entry(&mut self, id: &str) -> &mut TerminalMeta {
        self.entries.entry(id.to_string()).or_default()
    }
}

pub fn agent_icon_from_command(raw: &str) -> Option<&'static str> {
    agent_kind_from_command(raw).icon()
}

pub fn agent_kind_from_command(raw: &str) -> agent::Kind {
    agent::detect(raw)
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
    fn latches_codex_icon_from_command_until_command_end() {
        let mut state = TerminalState::new();
        let id = "term-1";

        state.set_current_command(id, "codex --model gpt-5.4".to_owned());
        state.set_shell_running(id);

        let meta = state.meta(id);
        assert_eq!(meta.shell_state, ShellState::Running);
        assert_eq!(meta.agent_icon, Some("icons/openai.svg"));

        state.set_title(id, Some("Reviewing terminal icon code".to_owned()));
        assert_eq!(
            state.meta(id).agent_icon,
            Some("icons/openai.svg"),
            "running command identity should survive later title changes"
        );

        state.set_shell_idle(id, Some(0));
        let meta = state.meta(id);
        assert_eq!(meta.shell_state, ShellState::Idle);
        assert_eq!(meta.agent_icon, None);
        assert_eq!(meta.current_command, None);
    }

    #[test]
    fn title_does_not_seed_agent_icon_when_command_line_is_missing() {
        let mut state = TerminalState::new();
        let id = "term-1";

        state.set_title(id, Some("codex".to_owned()));
        state.set_shell_running(id);

        assert_eq!(state.meta(id).agent_icon, None);

        state.set_title(id, Some("Implementing a fix".to_owned()));
        assert_eq!(
            state.meta(id).agent_icon,
            None,
            "title is display metadata, not an agent identity source"
        );
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

    #[test]
    fn updates_agent_icon_if_command_line_arrives_after_start() {
        let mut state = TerminalState::new();
        let id = "term-1";

        state.set_shell_running(id);
        assert_eq!(state.meta(id).agent_icon, None);

        state.set_current_command(id, "npx @openai/codex".to_owned());
        assert_eq!(state.meta(id).agent_icon, Some("icons/openai.svg"));
    }

    #[test]
    fn title_does_not_seed_agent_icon_after_command_start() {
        let mut state = TerminalState::new();
        let id = "term-1";

        state.set_shell_running(id);
        assert_eq!(state.meta(id).agent_icon, None);

        state.set_title(id, Some("claude".to_owned()));
        assert_eq!(state.meta(id).agent_icon, None);

        state.set_title(id, Some("Editing terminal_state.rs".to_owned()));
        assert_eq!(
            state.meta(id).agent_icon,
            None,
            "semantic titles must not create an agent identity"
        );
    }

    #[test]
    fn command_line_metadata_sets_agent_icon_even_after_title_changes() {
        let mut state = TerminalState::new();
        let id = "term-1";

        state.set_shell_running(id);
        state.set_title(id, Some("claude".to_owned()));
        assert_eq!(state.meta(id).agent_icon, None);

        state.set_current_command(id, "gemini --yolo".to_owned());
        assert_eq!(state.meta(id).agent_icon, Some("icons/gemini.svg"));
    }

    #[test]
    fn icon_name_latches_agent_when_command_lifecycle_is_missing() {
        let mut state = TerminalState::new();
        let id = "term-1";

        state.set_icon_name(id, "codex".to_owned());
        assert_eq!(state.meta(id).shell_state, ShellState::Running);
        assert_eq!(state.meta(id).agent_icon, Some("icons/openai.svg"));
        assert_eq!(state.meta(id).agent_kind, Some(agent::Kind::Codex));

        state.set_title(id, Some("⠋ zedra".to_owned()));
        assert_eq!(
            state.meta(id).agent_icon,
            Some("icons/openai.svg"),
            "semantic title changes should not clear icon-name fallback identity"
        );

        state.set_icon_name(id, "..rojects/zedra".to_owned());
        let meta = state.meta(id);
        assert_eq!(meta.shell_state, ShellState::Idle);
        assert_eq!(meta.agent_icon, None);
        assert_eq!(meta.agent_kind, None);
        assert_eq!(meta.current_command, None);
    }

    #[test]
    fn icon_name_is_primary_over_command_line_identity() {
        let mut state = TerminalState::new();
        let id = "term-1";

        state.set_current_command(id, "claude --dangerously-skip-permissions".to_owned());
        state.set_shell_running(id);
        assert_eq!(state.meta(id).agent_icon, Some("icons/claude.svg"));

        state.set_icon_name(id, "..rojects/zedra".to_owned());
        assert_eq!(state.meta(id).agent_icon, None);

        state.set_current_command(id, "codex".to_owned());
        state.set_shell_running(id);
        assert_eq!(
            state.meta(id).agent_icon,
            None,
            "once OSC 1 is present, command-line metadata should not override it"
        );

        state.set_icon_name(id, "codex".to_owned());
        assert_eq!(state.meta(id).agent_icon, Some("icons/openai.svg"));
    }

    #[test]
    fn command_start_without_command_line_preserves_icon_name_fallback_source() {
        let mut state = TerminalState::new();
        let id = "term-1";

        state.set_icon_name(id, "claude".to_owned());
        state.set_shell_running(id);
        assert_eq!(state.meta(id).agent_icon, Some("icons/claude.svg"));

        state.set_icon_name(id, "..rojects/zedra".to_owned());
        assert_eq!(state.meta(id).agent_icon, None);
    }
}
