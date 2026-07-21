use gpui::*;
use tokio::sync::broadcast;
use tracing::*;

use zedra_rpc::proto::HostEvent;
use zedra_session::{Session, SessionHandle};

use crate::editor::combined_diff_view::{CombinedDiffView, DiffFileEntry};
use crate::editor::git_diff_view::FileDiff;
use crate::editor::git_sidebar::GitFileSection;
use crate::placeholder::render_placeholder;

#[derive(Clone, Debug)]
pub struct GitdiffHeaderChanged {
    pub added: usize,
    pub removed: usize,
}

#[derive(Clone, Debug)]
pub enum GitdiffState {
    Loading,
    Loaded,
    Error { error: String },
}

pub struct WorkspaceGitdiff {
    state: GitdiffState,
    diff_view: Entity<CombinedDiffView>,
    session_handle: SessionHandle,
    diff_task: Option<Task<()>>,
    /// Set once the file list (from `git_status`) has been fetched — further
    /// `open_combined` calls just scroll instead of re-fetching, since
    /// `CombinedDiffView` lazily loads each file's actual diff content on its
    /// own and caches it. Reset to `false` on `HostEvent::GitChanged` so the
    /// next open refetches a list that staging/commit/external edits changed.
    status_loaded: bool,
    _diff_view_subscription: Subscription,
    _host_event_task: Task<()>,
}

impl EventEmitter<GitdiffHeaderChanged> for WorkspaceGitdiff {}

impl WorkspaceGitdiff {
    pub fn new(session: Session, cx: &mut Context<Self>) -> Self {
        let session_handle = session.handle().clone();
        let diff_view = cx.new(|cx| CombinedDiffView::new(session_handle.clone(), cx));
        // `CombinedDiffView` emits `()` whenever a lazily-fetched file finishes
        // loading — recompute the running total so the header summary grows
        // incrementally instead of only appearing once everything is loaded.
        let diff_view_subscription = cx.subscribe(
            &diff_view,
            |_this: &mut Self, diff_view, _event: &(), cx| {
                let (added, removed) = diff_view.read(cx).total_change_counts();
                cx.emit(GitdiffHeaderChanged { added, removed });
            },
        );
        // Invalidate the cached file list on any git change so the next open
        // refetches instead of scrolling a list that no longer matches the repo.
        let mut host_event_rx = session.subscribe_host_events();
        let host_event_task = cx.spawn(async move |this, cx| {
            loop {
                match host_event_rx.recv().await {
                    Ok(HostEvent::GitChanged) => {
                        if this
                            .update(cx, |this, _cx| this.status_loaded = false)
                            .is_err()
                        {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!("gitdiff host event listener lagged by {}", skipped);
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        Self {
            state: GitdiffState::Loading,
            diff_view,
            session_handle,
            diff_task: None,
            status_loaded: false,
            _diff_view_subscription: diff_view_subscription,
            _host_event_task: host_event_task,
        }
    }

    pub fn diff_view(&self) -> &Entity<CombinedDiffView> {
        &self.diff_view
    }

    /// Fetch the changed-file list (cheap: `git_status` only, no diff content)
    /// and scroll to `scroll_to` once ready. A file's actual diff content is
    /// fetched lazily by `CombinedDiffView` itself, only for files near where
    /// the user is looking — a workspace with many changed files doesn't pay
    /// for N diff RPCs it may never scroll to. Once the file list is loaded,
    /// this just scrolls (no RPC) since the list doesn't change out from
    /// under the view while it's open.
    pub fn open_combined(
        &mut self,
        scroll_to: Option<(String, GitFileSection)>,
        cx: &mut Context<Self>,
    ) {
        if self.status_loaded {
            if let Some((path, section)) = scroll_to {
                self.diff_view
                    .update(cx, |view, cx| view.scroll_to(&path, section, cx));
            }
            return;
        }

        self.state = GitdiffState::Loading;
        cx.notify();

        let prev_task = self.diff_task.take();
        drop(prev_task);

        let handle = self.session_handle.clone();
        let read_task = cx.spawn(async move |this, cx| {
            let status = match handle.git_status().await {
                Ok(status) => status,
                Err(e) => {
                    error!("git_status RPC failed: {}", e);
                    let _ = this.update(cx, |this, cx| {
                        this.state = GitdiffState::Error {
                            error: e.to_string(),
                        };
                        cx.notify();
                    });
                    return;
                }
            };

            // Bucket by section first (matches the sidebar and the view's
            // Staged -> Unstaged -> Untracked grouping) rather than pushing
            // in `git status`'s per-path order, which would interleave
            // sections whenever a staged and an unstaged file sort next to
            // each other alphabetically.
            let mut staged = Vec::new();
            let mut unstaged = Vec::new();
            let mut untracked = Vec::new();
            for entry in &status.entries {
                if entry.staged_status.is_some() {
                    staged.push(unloaded_entry(&entry.path, GitFileSection::Staged));
                }
                if let Some(unstaged_status) = &entry.unstaged_status {
                    if unstaged_status == "untracked" {
                        untracked.push(unloaded_entry(&entry.path, GitFileSection::Untracked));
                    } else {
                        unstaged.push(unloaded_entry(&entry.path, GitFileSection::Unstaged));
                    }
                }
            }
            let files: Vec<DiffFileEntry> = staged
                .into_iter()
                .chain(unstaged)
                .chain(untracked)
                .collect();

            if let Err(e) = this.update(cx, |this, cx| {
                this.state = GitdiffState::Loaded;
                this.status_loaded = true;
                this.diff_view.update(cx, |view, cx| {
                    view.set_files(files, cx);
                    if let Some((path, section)) = &scroll_to {
                        view.scroll_to(path, *section, cx);
                    }
                });
                cx.notify();
            }) {
                error!("failed to update gitdiff state: {}", e);
            }
        });

        self.diff_task = Some(read_task)
    }
}

/// Unloaded placeholder for a changed file — `CombinedDiffView` fetches the
/// real diff content lazily and swaps this in place.
fn unloaded_entry(path: &str, section: GitFileSection) -> DiffFileEntry {
    DiffFileEntry {
        file: FileDiff {
            old_path: path.to_string(),
            new_path: path.to_string(),
            hunks: Vec::new(),
        },
        section,
        loaded: false,
    }
}

impl Render for WorkspaceGitdiff {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match self.state.clone() {
            GitdiffState::Loading => render_placeholder(cx, "Loading ..."),
            GitdiffState::Error { error } => render_placeholder(cx, &format!("Error: {}", error)),
            GitdiffState::Loaded => div().size_full().child(self.diff_view.clone()),
        }
    }
}
