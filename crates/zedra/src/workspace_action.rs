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

#[derive(Clone, PartialEq, Action)]
#[action(namespace = workspace, no_json)]
pub struct ShowConnecting;

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
pub struct OpenQuickAction;

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
