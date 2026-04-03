use gpui::*;

use crate::editor::git_sidebar::{
    GitCommitRequested, GitFileEntry, GitFileLongPressed, GitFileSection, GitFileSelected,
    GitFileStatus, GitRepoState, GitSidebar,
};
use crate::file_explorer::{FileExplorer, FileSelected};
use crate::pending::{SharedPendingSlot, shared_pending_slot, spawn_notify_poll};
use crate::platform_bridge;
use crate::theme;
use crate::{session_panel, terminal_panel};
use zedra_session::{ConnectPhase, SessionState};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DrawerSection {
    Files,
    Git,
    Terminal,
    Session,
}

#[derive(Clone, Debug)]
pub enum WorkspaceDrawerEvent {
    GoHome,
    FileSelected(String),
    GitFileSelected(String, GitFileSection),
    GitFileLongPressed(String, GitFileSection),
    GitCommitRequested {
        message: String,
        paths: Vec<String>,
    },
    CloseRequested,
    ShowConnectingOverlay,
    DisconnectRequested,
    NewTerminalRequested,
    TerminalSelected(String),
    TerminalDeleteRequested(String),
    /// User dragged `dragged_id` onto `target_id`; move dragged to just before target.
    /// `target_id` is empty to mean "append at end".
    TerminalReordered {
        dragged_id: String,
        target_id: String,
    },
}

impl EventEmitter<WorkspaceDrawerEvent> for WorkspaceDrawer {}

pub struct WorkspaceDrawer {
    file_explorer: Entity<FileExplorer>,
    git_sidebar: Entity<GitSidebar>,
    active_section: DrawerSection,
    focus_handle: FocusHandle,
    pending_git_status: SharedPendingSlot<GitRepoState>,
    git_loaded: bool,
    active_terminal_id: Option<String>,
    /// Client-side terminal display order (persists across reconnects, updated on drag-reorder).
    terminal_order: Vec<String>,
    session_handle: Option<zedra_session::SessionHandle>,
    session_state: Option<SessionState>,
    workspace_state: crate::workspace_state::WorkspaceState,
    /// Kept alive to poll session state every 2 s and re-render the session tab.
    /// Dropped (and cancelled) when replaced by a new session.
    _session_refresh_task: Option<Task<()>>,
    /// Held to keep GPUI event subscriptions alive; dropped when the view is dropped.
    _subscriptions: Vec<Subscription>,
    /// Background task polling pending slots.
    _poll_task: Task<()>,
}

impl WorkspaceDrawer {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let file_explorer = cx.new(|cx| FileExplorer::new(cx));
        let git_sidebar = cx.new(|cx| GitSidebar::new(cx));

        let mut subscriptions = Vec::new();

        let sub = cx.subscribe(
            &file_explorer,
            |_this: &mut Self, _emitter, event: &FileSelected, cx| {
                cx.emit(WorkspaceDrawerEvent::FileSelected(event.path.clone()));
            },
        );
        subscriptions.push(sub);

        let sub = cx.subscribe(
            &git_sidebar,
            |_this: &mut Self, _emitter, event: &GitFileSelected, cx| {
                cx.emit(WorkspaceDrawerEvent::GitFileSelected(
                    event.path.clone(),
                    event.section,
                ));
            },
        );
        subscriptions.push(sub);

        let sub = cx.subscribe(
            &git_sidebar,
            |_this: &mut Self, _emitter, event: &GitFileLongPressed, cx| {
                cx.emit(WorkspaceDrawerEvent::GitFileLongPressed(
                    event.path.clone(),
                    event.section,
                ));
            },
        );
        subscriptions.push(sub);

        let sub = cx.subscribe(
            &git_sidebar,
            |_this: &mut Self, _emitter, event: &GitCommitRequested, cx| {
                cx.emit(WorkspaceDrawerEvent::GitCommitRequested {
                    message: event.message.clone(),
                    paths: event.paths.clone(),
                });
            },
        );
        subscriptions.push(sub);

        let pending_git_status: SharedPendingSlot<GitRepoState> = shared_pending_slot();

        let poll_git = pending_git_status.clone();
        let poll_task = spawn_notify_poll(
            cx,
            std::time::Duration::from_millis(50),
            move || poll_git.has_pending(),
        );

        Self {
            file_explorer,
            git_sidebar,
            active_section: DrawerSection::Files,
            focus_handle: cx.focus_handle(),
            pending_git_status,
            git_loaded: false,
            active_terminal_id: None,
            terminal_order: Vec::new(),
            session_handle: None,
            session_state: None,
            workspace_state: crate::workspace_state::WorkspaceState::default(),
            _session_refresh_task: None,
            _subscriptions: subscriptions,
            _poll_task: poll_task,
        }
    }

    /// Set the session state for this drawer.
    pub fn set_session_state(&mut self, state: SessionState) {
        self.session_state = Some(state);
    }

    pub fn set_section(&mut self, section: DrawerSection, cx: &mut Context<Self>) {
        self.active_section = section;
        if section == DrawerSection::Git {
            self.load_git_status();
        }
        cx.notify();
    }

    /// Called when this workspace becomes active.
    /// Reloads the file explorer from the new session and clears stale git data.
    pub fn on_activate(&mut self, cx: &mut Context<Self>) {
        self.git_loaded = false;
        self.file_explorer.update(cx, |fe, cx| fe.reload(cx));
        cx.notify();
    }

    /// Provide the current workspace's session handle.
    ///
    /// Called by `WorkspaceView::on_activate` so the drawer can access
    /// session data (git status, terminal list, connection info) without globals.
    pub fn set_session_handle(
        &mut self,
        handle: zedra_session::SessionHandle,
        workdir: String,
        cx: &mut Context<Self>,
    ) {
        self.session_handle = Some(handle.clone());
        self.file_explorer
            .update(cx, |fe, cx| fe.set_session_handle(handle, workdir, cx));
        // Spawn a polling task that triggers a re-render every 2 s so that
        // live transport stats (RTT, bytes, etc.) stay up to date in the session tab.
        // Dropping the old task cancels it before the new one starts.
        self._session_refresh_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(2))
                    .await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        }));
    }

    /// Reset state after disconnect so next session triggers fresh loads.
    pub fn reset_for_disconnect(&mut self, cx: &mut Context<Self>) {
        self.git_loaded = false;
        self.active_terminal_id = None;
        self.file_explorer.update(cx, |fe, cx| fe.reset_to_demo(cx));
    }

    /// Prefetch file explorer and git content in parallel during session resume.
    /// Both fetches are independent tokio tasks so they run concurrently.
    pub fn prefetch_for_resume(&mut self, cx: &mut Context<Self>) {
        self.git_loaded = false;
        self.file_explorer.update(cx, |fe, cx| fe.reload(cx));
        self.load_git_status();
    }

    /// Update the active terminal indicator in the Terminal tab.
    pub fn set_active_terminal(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        self.active_terminal_id = id;
        cx.notify();
    }

    pub fn set_git_committing(&mut self, committing: bool, cx: &mut Context<Self>) {
        self.git_sidebar
            .update(cx, |sidebar, cx| sidebar.set_committing(committing, cx));
    }

    pub fn clear_git_commit_message(&mut self, cx: &mut Context<Self>) {
        self.git_sidebar
            .update(cx, |sidebar, cx| sidebar.clear_commit_message(cx));
    }

    pub fn refresh_git_status(&mut self) {
        self.git_loaded = false;
        self.load_git_status();
    }

    /// Update the client-side terminal display order.
    /// Called by WorkspaceView after any order change (create, delete, drag-reorder, reconnect).
    pub fn set_terminal_order(&mut self, order: Vec<String>, cx: &mut Context<Self>) {
        self.terminal_order = order;
        cx.notify();
    }

    fn load_git_status(&mut self) {
        if self.git_loaded {
            return;
        }
        let is_connected = self
            .session_state
            .as_ref()
            .map_or(false, |s| s.phase().is_connected());
        let handle = match self.session_handle.as_ref() {
            Some(h) if is_connected => h.clone(),
            _ => return,
        };
        self.git_loaded = true;
        let pending = self.pending_git_status.clone();
        zedra_session::session_runtime().spawn(async move {
            match handle.git_status().await {
                Ok(result) => {
                    let mut staged = Vec::new();
                    let mut unstaged = Vec::new();
                    let mut untracked = Vec::new();

                    for entry in &result.entries {
                        if let Some(status) = entry.staged_status.as_deref() {
                            staged.push(GitFileEntry::new(
                                &entry.path,
                                GitFileStatus::from_status_str(status),
                                GitFileSection::Staged,
                                0,
                                0,
                            ));
                        }

                        if let Some(status) = entry.unstaged_status.as_deref() {
                            let file = GitFileEntry::new(
                                &entry.path,
                                GitFileStatus::from_status_str(status),
                                if status == "untracked" {
                                    GitFileSection::Untracked
                                } else {
                                    GitFileSection::Unstaged
                                },
                                0,
                                0,
                            );
                            if status == "untracked" {
                                untracked.push(file);
                            } else {
                                unstaged.push(file);
                            }
                        }
                    }

                    let repo_state = GitRepoState {
                        branch: result.branch,
                        staged_files: staged,
                        unstaged_files: unstaged,
                        untracked_files: untracked,
                    };

                    pending.set(repo_state);
                }
                Err(e) => {
                    tracing::error!("git_status RPC failed: {}", e);
                }
            }
        });
    }

    pub fn active_section(&self) -> DrawerSection {
        self.active_section
    }

    pub fn set_workspace_state(
        &mut self,
        state: crate::workspace_state::WorkspaceState,
        cx: &mut Context<Self>,
    ) {
        self.workspace_state = state;
        cx.notify();
    }

    fn project_name(&self) -> String {
        self.workspace_state.project_name().to_string()
    }

    fn tab_subtitle(&self, cx: &App) -> String {
        match self.active_section {
            DrawerSection::Files => {
                let wd = self.workspace_state.workdir().to_string();
                if wd.is_empty() {
                    return String::new();
                }
                let home = self.workspace_state.homedir().to_string();
                if !home.is_empty() {
                    if let Some(rest) = wd.strip_prefix(&home) {
                        return format!("~{rest}");
                    }
                }
                wd
            }
            DrawerSection::Git => {
                let branch = self.git_sidebar.read(cx).branch();
                if branch.is_empty() {
                    "git".to_string()
                } else {
                    branch.into()
                }
            }
            DrawerSection::Terminal => "terminals".into(),
            DrawerSection::Session => {
                let inner = self.session_state.as_ref().map(|s| s.get());
                let phase = inner.as_ref().map(|s| &s.phase);
                let status = match phase {
                    Some(ConnectPhase::Connected) => "Connected",
                    Some(p) if p.is_connecting() => "Connecting",
                    Some(ConnectPhase::Reconnecting { .. }) => "Reconnecting",
                    Some(ConnectPhase::Failed(_)) => "Error",
                    _ => "Disconnected",
                };
                let mode = inner
                    .as_ref()
                    .and_then(|s| s.snapshot.transport.as_ref())
                    .map(|t| {
                        if t.is_direct {
                            match &t.network_hint {
                                Some(h) => format!("P2P \u{00b7} {}", h.label()),
                                None => "P2P".into(),
                            }
                        } else {
                            "Relay".into()
                        }
                    })
                    .unwrap_or_else(|| "...".into());
                format!("{status} - {mode}")
            }
        }
    }

    fn nav_icon(
        &self,
        icon_path: &'static str,
        section: DrawerSection,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let is_active = self.active_section == section;
        let color = if is_active {
            rgb(theme::TEXT_PRIMARY)
        } else {
            rgb(theme::TEXT_MUTED)
        };

        div()
            .w(px(36.0))
            .h(px(36.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .cursor_pointer()
            .hit_slop(px(10.0))
            .hover(|s| s.bg(theme::hover_bg()))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _event, _window, cx| {
                    if this.active_section == section {
                        cx.emit(WorkspaceDrawerEvent::CloseRequested);
                    } else {
                        this.set_section(section, cx);
                    }
                }),
            )
            .child(
                svg()
                    .path(icon_path)
                    .size(px(theme::ICON_NAV))
                    .text_color(color),
            )
    }

    fn render_terminal_tab(&self, cx: &mut Context<Self>) -> Div {
        // Use client-side order if set; otherwise fall back to server order.
        let ids_from_handle;
        let terminal_ids: &[String] = if !self.terminal_order.is_empty() {
            &self.terminal_order
        } else {
            ids_from_handle = self
                .session_handle
                .as_ref()
                .map(|h| h.terminal_ids())
                .unwrap_or_default();
            &ids_from_handle
        };
        terminal_panel::render_terminal_tab(
            self.session_handle.as_ref(),
            terminal_ids,
            self.active_terminal_id.as_deref(),
            cx,
        )
    }

    fn render_session_tab(&self, cx: &mut Context<Self>) -> Div {
        session_panel::render_session_tab(self.session_state.as_ref(), cx)
    }
}

impl Focusable for WorkspaceDrawer {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for WorkspaceDrawer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self
            .session_handle
            .as_ref()
            .is_some_and(|h| h.take_git_refresh())
        {
            self.git_loaded = false;
            if self.active_section == DrawerSection::Git {
                self.load_git_status();
            }
        }

        // Check for pending git status from async RPC
        if let Some(state) = self.pending_git_status.take() {
            self.git_sidebar
                .update(cx, |sidebar, cx| sidebar.set_repo_state(state, cx));
        }

        let project_name = self.project_name();
        let tab_subtitle = self.tab_subtitle(cx);
        let phase = self.session_state.as_ref().map(|s| s.phase());
        let status_color = match phase.as_ref() {
            Some(ConnectPhase::Connected) => theme::ACCENT_GREEN,
            Some(p) if p.is_connecting() || p.is_reconnecting() => theme::ACCENT_YELLOW,
            _ => theme::ACCENT_RED,
        };
        let viewport_h = window.viewport_size().height;

        let tab_content: AnyElement = match self.active_section {
            DrawerSection::Files => div()
                .id("drawer-file-tree")
                .flex_1()
                .overflow_hidden()
                .child(self.file_explorer.clone())
                .into_any_element(),
            DrawerSection::Git => div()
                .flex_1()
                .overflow_hidden()
                .child(self.git_sidebar.clone())
                .into_any_element(),
            DrawerSection::Terminal => self.render_terminal_tab(cx).into_any_element(),
            DrawerSection::Session => div()
                .id("session-scroll")
                .flex_1()
                .overflow_y_scroll()
                .child(self.render_session_tab(cx))
                .into_any_element(),
        };

        let top_inset = platform_bridge::status_bar_inset();
        let bottom_inset = platform_bridge::home_indicator_inset().max(10.0);

        div()
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .w_full()
            .h(viewport_h)
            .bg(rgb(theme::BG_PRIMARY))
            // Status bar spacer
            .child(div().h(px(top_inset)))
            // Section header (fixed 48px)
            .child(
                div()
                    .h(px(theme::HEADER_HEIGHT))
                    .flex()
                    .flex_row()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(theme::BORDER_SUBTLE))
                    .child(
                        div()
                            .id("drawer-home-icon")
                            .w(px(theme::DRAWER_ICON_ZONE))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hit_slop(px(10.0))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|_this, _event, _window, cx| {
                                    cx.emit(WorkspaceDrawerEvent::GoHome);
                                }),
                            )
                            .child(
                                svg()
                                    .path("icons/logo.svg")
                                    .size(px(theme::ICON_LOGO))
                                    .text_color(rgb(theme::TEXT_PRIMARY)),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .items_center()
                            .child(
                                div()
                                    .id("project_name")
                                    .w_full()
                                    .min_w_0()
                                    .truncate()
                                    .text_center()
                                    .text_color(rgb(theme::TEXT_SECONDARY))
                                    .text_size(px(theme::FONT_BODY))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(project_name),
                            )
                            .child(
                                div()
                                    .id("tab_title")
                                    .w_full()
                                    .min_w_0()
                                    .truncate()
                                    .text_center()
                                    .text_color(rgb(theme::TEXT_MUTED))
                                    .text_size(px(theme::FONT_BODY))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(tab_subtitle),
                            ),
                    )
                    .child(
                        div()
                            .w(px(theme::DRAWER_ICON_ZONE))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .w(px(6.0))
                                    .h(px(6.0))
                                    .rounded(px(3.0))
                                    .bg(rgb(status_color)),
                            ),
                    ),
            )
            // Tab content
            .child(tab_content)
            // Footer nav bar
            .child(
                div()
                    .flex()
                    .flex_row()
                    .pt(px(10.0))
                    .pb(px(bottom_inset))
                    .justify_center()
                    .gap(px(36.0))
                    .border_t_1()
                    .border_color(rgb(theme::BORDER_SUBTLE))
                    .child(self.nav_icon("icons/folder.svg", DrawerSection::Files, cx))
                    .child(self.nav_icon("icons/git-branch.svg", DrawerSection::Git, cx))
                    .child(self.nav_icon("icons/terminal.svg", DrawerSection::Terminal, cx))
                    .child(self.nav_icon("icons/server.svg", DrawerSection::Session, cx)),
            )
    }
}
