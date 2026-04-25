use gpui::*;
use tracing::*;

use zedra_session::SessionHandle;

use crate::editor::git_diff_view::{FileDiff, GitDiffView, parse_unified_diff};
use crate::editor::git_sidebar::GitFileSection;
use crate::placeholder::render_placeholder;

const MAX_DIFF_BYTES: usize = 200 * 1024;

#[derive(Clone, Debug)]
pub struct GitdiffHeaderChanged {
    pub filename: String,
    pub added: usize,
    pub removed: usize,
}

#[derive(Clone, Debug)]
pub enum GitdiffState {
    Loading,
    Loaded,
    TooLarge,
    Error { error: String },
}

pub struct WorkspaceGitdiff {
    state: GitdiffState,
    diff_view: Entity<GitDiffView>,
    session_handle: SessionHandle,
    diff_task: Option<Task<()>>,
}

impl EventEmitter<GitdiffHeaderChanged> for WorkspaceGitdiff {}

impl WorkspaceGitdiff {
    pub fn new(session_handle: SessionHandle, cx: &mut App) -> Self {
        Self {
            state: GitdiffState::Loading,
            diff_view: cx.new(|cx| GitDiffView::new(cx)),
            session_handle,
            diff_task: None,
        }
    }

    /// Request loading a git diff for a file.
    /// The diff is loaded asynchronously and rendered when ready.
    pub fn open_diff(&mut self, path: String, section: GitFileSection, cx: &mut Context<Self>) {
        let filename = path.rsplit('/').next().unwrap_or(&path).to_string();
        let filename_clone = filename.clone();
        self.state = GitdiffState::Loading;
        cx.emit(GitdiffHeaderChanged {
            filename,
            added: 0,
            removed: 0,
        });
        cx.notify();

        // Drop any previous task before starting a new one.
        let prev_task = self.diff_task.take();
        drop(prev_task);

        let handle = self.session_handle.clone();
        let read_task = cx.spawn(async move |this, cx| {
            let (state, diff) = match section {
                GitFileSection::Staged | GitFileSection::Unstaged | GitFileSection::Untracked => {
                    let staged = matches!(section, GitFileSection::Staged);
                    match handle.git_diff(Some(&path), staged).await {
                        Ok(diff_text) => {
                            if diff_text.len() > MAX_DIFF_BYTES {
                                (GitdiffState::TooLarge, None)
                            } else {
                                let diffs = parse_unified_diff(&diff_text);
                                let diff = diffs
                                    .into_iter()
                                    .find(|d| d.new_path == path || d.old_path == path)
                                    .unwrap_or(FileDiff {
                                        old_path: path.clone(),
                                        new_path: path.clone(),
                                        hunks: Vec::new(),
                                    });
                                let (added, removed) = diff.change_counts();
                                let _ = this.update(cx, |_this, cx| {
                                    cx.emit(GitdiffHeaderChanged {
                                        filename: filename_clone.clone(),
                                        added,
                                        removed,
                                    });
                                });
                                (GitdiffState::Loaded, Some(diff))
                            }
                        }
                        Err(e) => {
                            error!("git_diff RPC failed for {}: {}", path, e);
                            (
                                GitdiffState::Error {
                                    error: e.to_string(),
                                },
                                None,
                            )
                        }
                    }
                }
            };

            if let Err(e) = this.update(cx, |this, cx| {
                this.state = state;
                if let Some(diff) = diff {
                    this.diff_view.update(cx, |diff_view, cx| {
                        diff_view.set_diff(filename_clone, diff, cx)
                    });
                }
                cx.notify();
            }) {
                error!("failed to update gitdiff state: {}", e);
            }
        });

        self.diff_task = Some(read_task)
    }
}

impl Render for WorkspaceGitdiff {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        match self.state.clone() {
            GitdiffState::Loading => render_placeholder("Loading ..."),
            GitdiffState::TooLarge => render_placeholder("Diff too large (>200 KB)"),
            GitdiffState::Error { error } => render_placeholder(&format!("Error: {}", error)),
            GitdiffState::Loaded => div().size_full().child(self.diff_view.clone()),
        }
    }
}
