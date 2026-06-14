use std::collections::HashMap;

use zedra_rpc::proto::{AgentState, TerminalSyncEntry};

use crate::agent;

pub use zedra_rpc::proto::TermShellState as ShellState;

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
    /// Icon of the foreground agent. Recomputed when a command starts,
    /// cleared on command end, survives prompt-ready between agent turns.
    pub agent_icon: Option<&'static str>,
    pub agent_kind: Option<agent::Kind>,
    pub agent_source: AgentIdentitySource,
    pub agent_state: AgentState,
}

impl TerminalMeta {
    /// Identity follows the foreground command: agents latch their icon,
    /// any other command resets to the default terminal icon.
    fn update_agent_identity(&mut self, kind: agent::Kind) {
        self.agent_kind = (kind != agent::Kind::Shell).then_some(kind);
        self.agent_icon = kind.icon();
    }
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

    pub fn set_agent_state(&mut self, id: &str, state: AgentState) {
        self.entry(id).agent_state = state;
    }

    pub fn set_shell_running(&mut self, id: &str) {
        let e = self.entry(id);
        e.shell_state = ShellState::Running;
        // Recompute identity from the starting command; keep the current
        // identity when the command is unknown (e.g. bare 633;C).
        if e.agent_source == AgentIdentitySource::CommandLine
            && let Some(command) = e.current_command.as_deref()
        {
            let kind = agent_kind_from_command(command);
            e.update_agent_identity(kind);
        }
    }

    pub fn set_command_started(&mut self, id: &str) {
        let prev_kind = self.entry(id).agent_kind;
        self.set_shell_running(id);
        let e = self.entry(id);
        // Default the title only when an agent is newly latched; an already
        // running agent starting an inner command keeps its live title.
        if let Some(kind) = e.agent_kind
            && prev_kind != Some(kind)
        {
            let title = agent::make_adapter(kind).display_name().to_owned();
            e.plain_title = plain_terminal_title(&title);
            e.title = Some(title);
        }
    }

    pub fn set_shell_idle(&mut self, id: &str, exit_code: Option<i32>) {
        self.mark_shell_idle(id, exit_code, true);
    }

    pub fn set_prompt_ready(&mut self, id: &str) {
        // OSC 133;A signals the shell is showing its prompt. For long-running
        // agents like pi that emit this between turns, do not clear agent
        // identity — the agent is still the foreground process.
        self.mark_shell_idle(id, None, false);
    }

    fn mark_shell_idle(&mut self, id: &str, exit_code: Option<i32>, clear_agent: bool) {
        let e = self.entry(id);
        e.shell_state = ShellState::Idle;
        if let Some(code) = exit_code {
            e.last_exit_code = Some(code);
        }
        e.current_command = None;
        if clear_agent {
            e.agent_icon = None;
            e.agent_kind = None;
        }
    }

    pub fn set_current_command(&mut self, id: &str, command: String) {
        let kind = agent_kind_from_command(&command);
        let e = self.entry(id);
        let latched = e.shell_state == ShellState::Running;
        let ignored = e.agent_source == AgentIdentitySource::IconName;
        e.current_command = Some(command);
        if latched && !ignored {
            e.update_agent_identity(kind);
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

    /// Seed full terminal metadata from the host's sync snapshot. The host is
    /// the source of truth on reconnect; later live OSC events update from
    /// this baseline.
    pub fn seed_host_meta(&mut self, sync: &TerminalSyncEntry) {
        let id = &sync.id;
        self.set_title(id, sync.title.clone());
        if let Some(cwd) = &sync.cwd {
            self.set_cwd(id, cwd.clone());
        }
        if let Some(icon_name) = &sync.icon_name {
            self.set_icon_name(id, icon_name.clone());
        }

        let e = self.entry(id);
        e.shell_state = sync.shell_state;
        // Client current_command is only meaningful while running; the host
        // command survives prompt-ready, so gate on shell state.
        e.current_command = if sync.shell_state == ShellState::Running {
            sync.agent_command.clone()
        } else {
            None
        };
        if sync.last_exit_code.is_some() {
            e.last_exit_code = sync.last_exit_code;
        }
        e.agent_state = sync.agent_state;

        // The host-latched agent command is authoritative for agent identity;
        // it overrides a non-agent OSC 1 icon name (shells often set OSC 1 to
        // the cwd).
        if let Some(command) = &sync.agent_command {
            let kind = agent_kind_from_command(command);
            if kind != agent::Kind::Shell {
                e.agent_source = AgentIdentitySource::CommandLine;
                e.update_agent_identity(kind);
            }
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
    fn agent_identity_survives_prompt_ready_until_next_command() {
        let mut state = TerminalState::new();
        let id = "term-1";

        state.set_current_command(id, "codex".to_owned());
        state.set_shell_running(id);
        assert_eq!(state.meta(id).agent_icon, Some("icons/openai.svg"));

        // Agents emit prompt-ready between turns without exiting.
        state.set_prompt_ready(id);
        assert_eq!(state.meta(id).agent_icon, Some("icons/openai.svg"));

        // A new non-agent command resets identity to the default icon.
        state.set_current_command(id, "vim .".to_owned());
        state.set_command_started(id);
        assert_eq!(state.meta(id).agent_icon, None);
        assert_eq!(state.meta(id).agent_kind, None);
    }

    #[test]
    fn sync_seed_derives_agent_identity_from_latched_command() {
        let mut state = TerminalState::new();
        let sync = TerminalSyncEntry {
            id: "term-1".to_owned(),
            title: Some("Reviewing auth flow".to_owned()),
            cwd: Some("/repo".to_owned()),
            // Shells often set OSC 1 to the cwd; must not block agent identity.
            icon_name: Some("..a-integration".to_owned()),
            agent_command: Some("codex".to_owned()),
            shell_state: ShellState::Idle,
            last_exit_code: Some(0),
            ..Default::default()
        };

        state.seed_host_meta(&sync);

        let meta = state.meta("term-1");
        assert_eq!(meta.agent_kind, Some(agent::Kind::Codex));
        assert_eq!(meta.agent_icon, Some("icons/openai.svg"));
        assert_eq!(meta.shell_state, ShellState::Idle);
        assert_eq!(meta.title.as_deref(), Some("Reviewing auth flow"));
    }

    #[test]
    fn sync_seed_without_agent_command_does_not_invent_identity() {
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
        assert_eq!(meta.agent_kind, None);
        assert_eq!(meta.agent_icon, None);
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
    fn command_start_sets_default_agent_title() {
        let mut state = TerminalState::new();
        let id = "term-1";

        state.set_title(id, Some("Launching Hermes Agent...".to_owned()));
        state.set_current_command(id, "hermes".to_owned());
        state.set_command_started(id);

        let meta = state.meta(id);
        assert_eq!(meta.title.as_deref(), Some("Hermes Agent"));
        assert_eq!(meta.plain_title.as_deref(), Some("Hermes Agent"));
    }

    #[test]
    fn inner_program_title_overrides_default_agent_title() {
        let mut state = TerminalState::new();
        let id = "term-1";

        state.set_current_command(id, "hermes".to_owned());
        state.set_command_started(id);
        state.set_title(id, Some("Reviewing auth flow".to_owned()));

        assert_eq!(state.meta(id).title.as_deref(), Some("Reviewing auth flow"));
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
