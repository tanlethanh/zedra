use gpui::Action;

#[derive(Clone, PartialEq, Action)]
#[action(namespace = workspace, no_json)]
pub struct GoHome;

#[derive(Clone, PartialEq, Action)]
#[action(namespace = workspace, no_json)]
pub struct OpenFile {
    pub path: String,
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = workspace, no_json)]
pub struct RevealInFileExplorer {
    pub path: String,
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = workspace, no_json)]
pub struct AddSelectionToChat;

#[derive(Clone, PartialEq, Action)]
#[action(namespace = workspace, no_json)]
pub struct OpenGitDiff {
    pub path: String,
    /// 0 = Staged, 1 = Unstaged, 2 = Untracked
    pub section: u8,
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = workspace, no_json)]
pub struct GitStage {
    pub path: String,
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = workspace, no_json)]
pub struct GitUnstage {
    pub path: String,
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = workspace, no_json)]
pub struct GitShowItemActions {
    pub path: String,
    /// 0 = Staged, 1 = Unstaged, 2 = Untracked
    pub section: u8,
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = workspace, no_json)]
pub struct GitCommit {
    pub message: String,
    pub paths: Vec<String>,
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = workspace, no_json)]
pub struct CloseDrawer;

/// Send the device clipboard to the host system clipboard (manual push).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = workspace, no_json)]
pub struct SendClipboard;

#[derive(Clone, PartialEq, Action)]
#[action(namespace = workspace, no_json)]
pub struct ShowConnecting;

#[derive(Clone, PartialEq, Action)]
#[action(namespace = workspace, no_json)]
pub struct ShowWorkspaceConnecting {
    pub entry_index: usize,
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = workspace, no_json)]
pub struct ShowHomeWorkspaceConnecting {
    pub state_index: usize,
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = workspace, no_json)]
pub struct HideConnecting;

#[derive(Clone, PartialEq, Action)]
#[action(namespace = workspace, no_json)]
pub struct RestartConnection;

#[derive(Clone, PartialEq, Action)]
#[action(namespace = workspace, no_json)]
pub struct RequestDisconnect;

#[derive(Clone, PartialEq, Action)]
#[action(namespace = workspace, no_json)]
pub struct CreateNewTerminal;

#[derive(Clone, PartialEq, Action)]
#[action(namespace = workspace, no_json)]
pub struct NavigateBack;

#[derive(Clone, PartialEq, Action)]
#[action(namespace = workspace, no_json)]
pub struct OpenAgentSessions;

#[derive(Clone, PartialEq, Action)]
#[action(namespace = workspace, no_json)]
pub struct OpenAgentManage;

#[derive(Clone, PartialEq, Action)]
#[action(namespace = workspace, no_json)]
pub struct OpenAgentDetail {
    pub slug: String,
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = workspace, no_json)]
pub struct CreateAgent;

#[derive(Clone, PartialEq, Action)]
#[action(namespace = workspace, no_json)]
pub struct SpawnAgentTerminal {
    pub launch_cmd: String,
    pub initial_title: String,
    pub agent_slug: String,
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = workspace, no_json)]
pub struct ResumeAgentSession {
    pub slug: String,
    pub session_id: String,
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = workspace, no_json)]
pub struct OpenTerminal {
    pub id: String,
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = workspace, no_json)]
pub struct CloseTerminal {
    pub id: String,
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = workspace, no_json)]
pub struct ToggleDrawer;

#[derive(Clone, PartialEq, Action)]
#[action(namespace = workspace, no_json)]
pub struct OpenDrawer;

#[derive(Clone, PartialEq, Action)]
#[action(namespace = workspace, no_json)]
pub struct OpenQuickAction;

#[derive(Clone, PartialEq, Action)]
#[action(namespace = workspace, no_json)]
pub struct OpenFileSearch;

#[derive(Clone, PartialEq, Action)]
#[action(namespace = workspace, no_json)]
pub struct UpdateTitle {
    pub title: String,
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = workspace, no_json)]
pub struct UpdateSubtitle {
    pub subtitle: String,
}
