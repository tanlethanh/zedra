use std::collections::HashMap;

#[derive(Clone, Default, Debug, PartialEq)]
pub enum ShellState {
    #[default]
    Unknown,
    Idle,
    Running,
}

#[derive(Clone, Default, Debug)]
pub struct TerminalMeta {
    pub title: Option<String>,
    pub cwd: Option<String>,
    pub last_exit_code: Option<i32>,
    pub shell_state: ShellState,
    /// Last command line reported by OSC 633;E. Cleared when the shell returns to prompt.
    pub current_command: Option<String>,
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
        self.entry(id).title = title;
    }

    pub fn set_cwd(&mut self, id: &str, cwd: String) {
        self.entry(id).cwd = Some(cwd);
    }

    pub fn set_shell_running(&mut self, id: &str) {
        self.entry(id).shell_state = ShellState::Running;
    }

    pub fn set_shell_idle(&mut self, id: &str, exit_code: Option<i32>) {
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

    fn entry(&mut self, id: &str) -> &mut TerminalMeta {
        self.entries.entry(id.to_string()).or_default()
    }
}
