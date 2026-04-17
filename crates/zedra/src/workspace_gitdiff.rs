use gpui::*;
use tracing::*;

use zedra_session::SessionHandle;

use crate::editor::git_diff_view::{FileDiff, GitDiffView, parse_unified_diff};
use crate::editor::git_sidebar::GitFileSection;
use crate::placeholder::render_placeholder;

const MAX_DIFF_BYTES: usize = 200 * 1024;

#[derive(Clone, Debug)]
pub enum GitdiffState {
    Loading,
    Loaded { filename: String, diff: FileDiff },
    TooLarge { filename: String },
    Error { error: String },
}

pub struct WorkspaceGitdiff {
    state: GitdiffState,
    session_handle: SessionHandle,
    diff_task: Option<Task<()>>,
}

impl WorkspaceGitdiff {
    pub fn new(session_handle: SessionHandle) -> Self {
        Self {
            state: GitdiffState::Loading,
            session_handle,
            diff_task: None,
        }
    }

    /// Request loading a git diff for a file.
    /// The diff is loaded asynchronously and rendered when ready.
    pub fn open_diff(&mut self, path: String, section: GitFileSection, cx: &mut Context<Self>) {
        let filename = path.rsplit('/').next().unwrap_or(&path).to_string();
        self.state = GitdiffState::Loading;
        cx.notify();

        // Drop any previous task before starting a new one.
        let prev_task = self.diff_task.take();
        drop(prev_task);

        let handle = self.session_handle.clone();
        let read_task = cx.spawn(async move |this, cx| {
            let state = match section {
                GitFileSection::Staged | GitFileSection::Unstaged | GitFileSection::Untracked => {
                    // Untracked file diffs are provided by git in the unstaged set.
                    let staged = matches!(section, GitFileSection::Staged);
                    match handle.git_diff(Some(&path), staged).await {
                        Ok(diff_text) => {
                            if diff_text.len() > MAX_DIFF_BYTES {
                                GitdiffState::TooLarge { filename }
                            } else {
                                let diffs = parse_unified_diff(&diff_text);
                                let diff = diffs
                                    .into_iter()
                                    .find(|d| d.new_path == path)
                                    .unwrap_or(FileDiff {
                                        old_path: path.clone(),
                                        new_path: path.clone(),
                                        hunks: Vec::new(),
                                    });
                                GitdiffState::Loaded { filename, diff }
                            }
                        }
                        Err(e) => {
                            error!("git_diff RPC failed for {}: {}", path, e);
                            GitdiffState::Error {
                                error: e.to_string(),
                            }
                        }
                    }
                }
            };

            if let Err(e) = this.update(cx, |this, cx| {
                this.state = state;
                cx.notify();
            }) {
                error!("failed to update gitdiff state: {}", e);
            }
        });

        self.diff_task = Some(read_task)
    }
}

impl Render for WorkspaceGitdiff {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match self.state.clone() {
            GitdiffState::Loading => render_placeholder("Loading ..."),
            GitdiffState::TooLarge { .. } => render_placeholder("Diff too large (>200 KB)"),
            GitdiffState::Error { error } => render_placeholder(&format!("Error: {}", error)),
            GitdiffState::Loaded { filename: _, diff } => {
                let path = diff.new_path.clone();
                let diff_view = cx.new(|cx| GitDiffView::new(diff, path, cx));
                div().child(diff_view)
            }
        }
    }
}
