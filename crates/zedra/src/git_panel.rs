use gpui::*;
use tracing::*;

use zedra_rpc::proto::{GitStatusEntry, HostEvent};
use zedra_session::{Session, SessionHandle, SessionState};

use crate::editor::git_sidebar::{
    GitCommitRequested, GitFileEntry, GitFileLongPressed, GitFileSection, GitFileSelected,
    GitFileStatus, GitRepoState, GitSidebar,
};
use crate::workspace_state::{WorkspaceState, WorkspaceStateEvent};

pub struct GitPanel {
    #[allow(dead_code)]
    workspace_state: Entity<WorkspaceState>,
    #[allow(dead_code)]
    session_state: Entity<SessionState>,
    session_handle: SessionHandle,
    content: Entity<GitSidebar>,
    tasks: Vec<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<GitFileSelected> for GitPanel {}
impl EventEmitter<GitFileLongPressed> for GitPanel {}

impl GitPanel {
    pub fn new(
        workspace_state: Entity<WorkspaceState>,
        session_state: Entity<SessionState>,
        session: Session,
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
                                this.fetch_git_status(cx);
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

        // Forward events from GitSidebar
        let mut subscriptions = Vec::new();
        subscriptions.push(cx.subscribe(
            &content,
            |_this, _sidebar, event: &GitFileSelected, cx| {
                cx.emit(event.clone());
            },
        ));
        subscriptions.push(cx.subscribe(
            &content,
            |_this, _sidebar, event: &GitFileLongPressed, cx| {
                cx.emit(event.clone());
            },
        ));
        subscriptions.push(cx.subscribe(
            &content,
            |this, _sidebar, event: &GitCommitRequested, cx| {
                this.handle_commit(event.message.clone(), event.paths.clone(), cx);
            },
        ));
        subscriptions.push(cx.subscribe(
            &workspace_state,
            |this, _workspace, event: &WorkspaceStateEvent, cx| {
                if matches!(event, WorkspaceStateEvent::SyncComplete) {
                    this.fetch_git_status(cx);
                }
            },
        ));

        Self {
            workspace_state,
            session_state,
            session_handle,
            content,
            tasks: vec![host_event_task],
            _subscriptions: subscriptions,
        }
    }

    fn fetch_git_status(&mut self, cx: &mut Context<Self>) {
        let handle = self.session_handle.clone();
        let content = self.content.clone();
        let task = cx.spawn(async move |_this, cx| match handle.git_status().await {
            Ok(result) => {
                let repo_state = status_to_repo_state(&result.branch, &result.entries);
                let _ = content.update(cx, |sidebar, cx| {
                    sidebar.set_repo_state(repo_state, cx);
                });
            }
            Err(e) => {
                error!("git_status failed: {}", e);
            }
        });
        self.tasks.push(task);
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
                    this.fetch_git_status(cx);
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
