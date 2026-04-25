use gpui::*;
use tracing::*;

use zedra_rpc::proto::{GitStatusEntry, HostEvent};
use zedra_session::{Session, SessionHandle, SessionState};

use crate::editor::git_sidebar::{
    GitCommitRequested, GitFileEntry, GitFileLongPressed, GitFileSection, GitFileSelected,
    GitFileStatus, GitRepoState, GitSidebar,
};
use crate::workspace_action;
use crate::workspace_state::WorkspaceState;

pub struct GitPanel {
    #[allow(dead_code)]
    workspace_state: Entity<WorkspaceState>,
    #[allow(dead_code)]
    session_state: Entity<SessionState>,
    session_handle: SessionHandle,
    content: Entity<GitSidebar>,
    branch: String,
    tasks: Vec<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl GitPanel {
    pub fn new(
        workspace_state: Entity<WorkspaceState>,
        session_state: Entity<SessionState>,
        session: Session,
        window: AnyWindowHandle,
        session_handle: SessionHandle,
        cx: &mut Context<Self>,
    ) -> Self {
        let content = cx.new(|cx| GitSidebar::new(cx));
        let mut host_event_rx = session.subscribe_host_events();
        let host_event_task = cx.spawn(async move |this, cx| {
            loop {
                match host_event_rx.recv().await {
                    Ok(HostEvent::GitChanged) => {
                        let should_break = this
                            .update(cx, |this, cx| {
                                this.fetch_git_status(cx).detach();
                            })
                            .is_err();
                        if should_break {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!("git panel host event listener lagged by {}", skipped);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        let mut subscriptions = Vec::new();
        let open_diff_window = window;
        subscriptions.push(cx.subscribe(
            &content,
            move |_this, _sidebar, event: &GitFileSelected, cx| {
                let action = workspace_action::OpenGitDiff {
                    path: event.path.clone(),
                    section: section_to_u8(event.section),
                };
                let _ = cx.update_window(open_diff_window, |_, window, cx| {
                    window.dispatch_action(action.boxed_clone(), cx);
                });
            },
        ));
        let item_actions_window = window;
        subscriptions.push(cx.subscribe(
            &content,
            move |_this, _sidebar, event: &GitFileLongPressed, cx| {
                let action = workspace_action::GitShowItemActions {
                    path: event.path.clone(),
                    section: section_to_u8(event.section),
                };
                let _ = cx.update_window(item_actions_window, |_, window, cx| {
                    window.dispatch_action(action.boxed_clone(), cx);
                });
            },
        ));
        subscriptions.push(cx.subscribe(
            &content,
            |this, _sidebar, event: &GitCommitRequested, cx| {
                this.handle_commit(event.message.clone(), event.paths.clone(), cx);
            },
        ));
        Self {
            workspace_state,
            session_state,
            session_handle,
            content,
            branch: String::new(),
            tasks: vec![host_event_task],
            _subscriptions: subscriptions,
        }
    }

    pub fn branch(&self) -> &str {
        &self.branch
    }

    pub fn refresh_after_sync(&mut self, cx: &mut Context<Self>) -> Task<()> {
        self.fetch_git_status(cx)
    }

    fn fetch_git_status(&mut self, cx: &mut Context<Self>) -> Task<()> {
        let handle = self.session_handle.clone();
        let content = self.content.clone();
        cx.spawn(async move |this, cx| match handle.git_status().await {
            Ok(result) => {
                let repo_state = status_to_repo_state(&result.branch, &result.entries);
                let _ = content.update(cx, |sidebar, cx| {
                    sidebar.set_repo_state(repo_state, cx);
                    let _ = this.update(cx, |this, _cx| {
                        this.branch = result.branch.clone();
                    });
                });
            }
            Err(e) => {
                error!("git_status failed: {}", e);
            }
        })
    }

    fn handle_commit(&mut self, message: String, paths: Vec<String>, cx: &mut Context<Self>) {
        let handle = self.session_handle.clone();
        let content = self.content.clone();
        content.update(cx, |sidebar, cx| {
            sidebar.set_committing(true, cx);
        });

        let task = cx.spawn(async move |this, cx| {
            let result = handle.git_commit(&message, &paths).await;
            let _ = content.update(cx, |sidebar, cx| {
                sidebar.set_committing(false, cx);
                match &result {
                    Ok(hash) => {
                        info!("committed: {}", hash);
                        sidebar.clear_commit_message(cx);
                    }
                    Err(e) => {
                        error!("git commit failed: {}", e);
                    }
                }
            });
            // Refresh status after commit
            if result.is_ok() {
                let _ = this.update(cx, |this, cx| {
                    this.fetch_git_status(cx).detach();
                });
            }
        });
        self.tasks.push(task);
    }
}

impl Render for GitPanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.content.clone()
    }
}

/// Convert `GitStatusEntry` list into `GitRepoState` for the sidebar.
fn status_to_repo_state(branch: &str, entries: &[GitStatusEntry]) -> GitRepoState {
    let mut staged = Vec::new();
    let mut unstaged = Vec::new();
    let mut untracked = Vec::new();

    for entry in entries {
        // Staged change
        if let Some(ref status) = entry.staged_status {
            let file_status = GitFileStatus::from_status_str(status);
            staged.push(GitFileEntry::new(
                &entry.path,
                file_status,
                GitFileSection::Staged,
                0,
                0,
            ));
        }

        // Unstaged change
        if let Some(ref status) = entry.unstaged_status {
            let file_status = GitFileStatus::from_status_str(status);
            if file_status == GitFileStatus::Untracked {
                untracked.push(GitFileEntry::new(
                    &entry.path,
                    file_status,
                    GitFileSection::Untracked,
                    0,
                    0,
                ));
            } else {
                unstaged.push(GitFileEntry::new(
                    &entry.path,
                    file_status,
                    GitFileSection::Unstaged,
                    0,
                    0,
                ));
            }
        }
    }

    GitRepoState {
        branch: branch.to_string(),
        staged_files: staged,
        unstaged_files: unstaged,
        untracked_files: untracked,
    }
}

fn section_to_u8(section: GitFileSection) -> u8 {
    match section {
        GitFileSection::Staged => 0,
        GitFileSection::Unstaged => 1,
        GitFileSection::Untracked => 2,
    }
}
