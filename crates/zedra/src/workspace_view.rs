// Per-session workspace: DrawerHost + header/main-view stack, wired to a SessionHandle.

use std::time::Duration;

use gpui::{prelude::FluentBuilder as _, *};
use zedra_telemetry::*;

use crate::active_terminal;
use crate::connecting_view;
use crate::editor::code_editor::EditorView;
use crate::editor::git_diff_view::{FileDiff, GitDiffView, parse_unified_diff};
use crate::editor::git_sidebar::GitFileSection;
use crate::fonts;
use crate::keyboard;
use crate::mgpui::DrawerHost;
use crate::pending::{SharedPendingSlot, shared_pending_slot};
use crate::platform_bridge::{self, AlertButton, status_bar_inset};
use crate::theme;
use crate::workspace_drawer::{WorkspaceDrawer, WorkspaceDrawerEvent};
use zedra_session::{SessionHandle, SessionState};
use zedra_terminal::view::{DisconnectRequested, TerminalView};

/// Sentinel terminal ID used before the server assigns a real ID.
const PENDING_TERMINAL_ID: &str = "__pending__";

/// Full-screen centered placeholder shown when a file is loading.
struct FileLoadingView;

impl Render for FileLoadingView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .child(
                div()
                    .top(px(-32.0))
                    .text_color(rgb(theme::TEXT_MUTED))
                    .text_size(px(theme::FONT_BODY))
                    .text_align(TextAlign::Center)
                    .child("Loading..."),
            )
    }
}

/// Full-screen centered placeholder shown when all terminals have been deleted.
struct NoTerminalPlaceholder;

impl Render for NoTerminalPlaceholder {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .child(
                div()
                    .top(px(-32.0))
                    .text_color(rgb(theme::TEXT_MUTED))
                    .text_size(px(theme::FONT_BODY))
                    .text_align(TextAlign::Center)
                    .child("No terminals"),
            )
    }
}

/// Full-screen centered placeholder shown when a file is too large to preview.
struct FileTooLargeView {
    message: String,
}

impl Render for FileTooLargeView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .child(
                div()
                    // Magic! It's more balance with this
                    .top(px(-32.0))
                    .text_color(rgb(theme::TEXT_SECONDARY))
                    .text_size(px(theme::FONT_BODY))
                    .text_align(TextAlign::Center)
                    .child(self.message.clone()),
            )
    }
}

#[derive(Clone, Debug)]
pub enum WorkspaceEvent {
    GoHome,
    OpenQuickAction,
    Disconnected,
}

#[derive(Clone, Debug)]
pub enum WorkspaceContentEvent {
    ToggleDrawer,
    OpenQuickAction,
}

#[derive(Clone, Debug)]
enum GitItemMenuAction {
    OpenDiff {
        path: String,
        section: GitFileSection,
    },
    Stage(String),
    Unstage(String),
}

#[derive(Clone, Copy, Debug)]
enum GitIndexOperation {
    Stage,
    Unstage,
}

/// Header bar `[≡ | title | ⚡]` with a swappable main view below.
/// The connecting overlay is rendered on top of the main view and lingers for
/// ~1s after the session becomes Connected so the workspace can fully repaint
/// before the overlay disappears (avoids a terminal-resume glitch).
pub struct WorkspaceContent {
    pub main_view: AnyView,
    pub header_title: SharedString,
    /// ID of the terminal currently shown as main view; `None` for file/editor views.
    active_terminal_id: Option<String>,
    focus_handle: FocusHandle,
    session_handle: SessionHandle,
    session_state: SessionState,
    workspace_state: crate::workspace_state::WorkspaceState,
    /// Whether the connecting overlay is currently shown.
    overlay_visible: bool,
    connecting_view: Entity<connecting_view::ConnectingView>,
}

impl WorkspaceContent {
    fn workspace_has_cached_context(state: &crate::workspace_state::WorkspaceState) -> bool {
        !state.session_id().is_empty()
            && (!state.project_name().is_empty()
                || !state.hostname().is_empty()
                || !state.strip_path().is_empty())
    }

    fn has_cached_workspace_context(&self) -> bool {
        Self::workspace_has_cached_context(&self.workspace_state)
    }

    pub fn new(
        main_view: AnyView,
        title: impl Into<SharedString>,
        session_handle: SessionHandle,
        session_state: SessionState,
        cx: &mut Context<Self>,
    ) -> Self {
        let connecting_view =
            cx.new(|_cx| connecting_view::ConnectingView::new(session_state.clone()));
        Self {
            main_view,
            header_title: title.into(),
            active_terminal_id: None,
            focus_handle: cx.focus_handle(),
            session_handle,
            session_state,
            workspace_state: crate::workspace_state::WorkspaceState::default(),
            overlay_visible: true,
            connecting_view,
        }
    }

    pub fn set_main_view(
        &mut self,
        view: AnyView,
        title: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.main_view = view;
        self.header_title = title.into();
        self.active_terminal_id = None;
        cx.notify();
    }

    pub fn set_main_terminal(
        &mut self,
        view: AnyView,
        terminal_id: String,
        cx: &mut Context<Self>,
    ) {
        self.main_view = view;
        self.active_terminal_id = Some(terminal_id);
        cx.notify();
    }

    pub fn set_workspace_state(
        &mut self,
        state: crate::workspace_state::WorkspaceState,
        cx: &mut Context<Self>,
    ) {
        self.workspace_state = state;
        cx.notify();
    }

    /// Show the connecting overlay (e.g. user taps session-tab progress during reconnect).
    /// Only auto-dismisses when the phase reaches Connected or Idle.
    pub fn show_overlay(&mut self, cx: &mut Context<Self>) {
        self.overlay_visible = true;
        cx.notify();
    }
}

impl EventEmitter<WorkspaceContentEvent> for WorkspaceContent {}

impl Focusable for WorkspaceContent {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for WorkspaceContent {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Show the full connecting overlay for all fresh connects (first pairing or saved
        // workspace).  Only suppress it during an auto-reconnect (connection dropped while
        // the user is already inside the workspace) where we have cached context to show.
        let inner = self.session_state.get();
        let phase = &inner.phase;
        let is_auto_reconnect = inner.reconnect_attempt.is_some();
        let show_inline_status = self.has_cached_workspace_context()
            && is_auto_reconnect
            && (phase.is_connecting() || phase.is_reconnecting() || phase.is_failed());
        let needs_full_overlay = !phase.is_connected() && !phase.is_idle() && !show_inline_status;

        if needs_full_overlay {
            // Reconnecting/failed overlay should always dismiss IME so it doesn't
            // remain floating above the blocked terminal surface.
            if !self.overlay_visible {
                platform_bridge::bridge().hide_keyboard();
            }
            self.overlay_visible = true;
        } else if self.overlay_visible && (phase.is_connected() || phase.is_idle()) {
            // Only auto-dismiss when the session reaches a terminal state.
            // This allows show_overlay() to keep the overlay up during reconnect.
            self.overlay_visible = false;
        }

        let top_inset = status_bar_inset();
        // For terminal views, derive the title live from OSC 2 metadata so it
        // updates automatically as the shell / running process changes the title.
        let title: SharedString = if let Some(ref tid) = self.active_terminal_id {
            self.session_handle
                .terminal(tid)
                .and_then(|t| {
                    let raw = t.meta().title?;
                    // Strip the default PS1 "user@host:path" prefix — show just the path.
                    let s = if let Some(at) = raw.find('@') {
                        if let Some(colon_off) = raw[at..].find(':') {
                            let path = &raw[at + colon_off + 1..];
                            if !path.is_empty() { path } else { raw.as_str() }
                        } else {
                            raw.as_str()
                        }
                    } else {
                        raw.as_str()
                    };
                    if s.is_empty() {
                        None
                    } else {
                        Some(SharedString::from(s.to_owned()))
                    }
                })
                .unwrap_or_else(|| SharedString::from("Terminal"))
        } else {
            self.header_title.clone()
        };

        let project_name: Option<SharedString> = {
            let name = self.workspace_state.project_name().to_string();
            if name.is_empty() {
                None
            } else {
                Some(name.into())
            }
        };
        // Small dot in the header corner: green=connected, yellow=connecting/reconnecting,
        // red=failed, hidden when idle.  No text or pill — detailed status lives in the
        // session drawer tab.
        let header_dot_color: Option<u32> = if phase.is_connected() {
            Some(theme::ACCENT_GREEN)
        } else if phase.is_connecting() || phase.is_reconnecting() {
            Some(theme::ACCENT_YELLOW)
        } else if phase.is_failed() {
            Some(theme::ACCENT_RED)
        } else {
            None
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(theme::BG_PRIMARY))
            .child(div().h(px(top_inset)))
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
                            .id("drawer-toggle-btn")
                            .w(px(theme::HEADER_BUTTON_SIZE))
                            .h(px(theme::HEADER_BUTTON_SIZE))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hit_slop(px(10.0))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|_this, _event, _window, cx| {
                                    platform_bridge::bridge().hide_keyboard();
                                    cx.emit(WorkspaceContentEvent::ToggleDrawer);
                                }),
                            )
                            .child(
                                svg()
                                    .path("icons/menu.svg")
                                    .size(px(16.0))
                                    .text_color(rgb(theme::TEXT_SECONDARY)),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .items_center()
                                    .w_full()
                                    .min_w_0()
                                    .children(project_name.map(|name| {
                                        // Dot is an inline flex sibling — row centers in
                                        // the header via justify_center on the outer div.
                                        // max_w_full lets the text truncate if too long.
                                        div()
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .gap(px(5.0))
                                            .max_w_full()
                                            .children(header_dot_color.map(|color| {
                                                div()
                                                    .w(px(6.0))
                                                    .h(px(6.0))
                                                    .rounded(px(3.0))
                                                    .flex_shrink_0()
                                                    .bg(rgb(color))
                                            }))
                                            .child(
                                                div()
                                                    .min_w_0()
                                                    .truncate()
                                                    .text_color(rgb(theme::TEXT_MUTED))
                                                    .text_size(px(theme::FONT_DETAIL))
                                                    .child(name),
                                            )
                                    }))
                                    .child(
                                        div()
                                            .w_full()
                                            .min_w_0()
                                            .truncate()
                                            .text_center()
                                            .text_color(rgb(theme::TEXT_SECONDARY))
                                            .text_size(px(theme::FONT_BODY))
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(title),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .id("quick-action-btn")
                            .w(px(theme::HEADER_BUTTON_SIZE))
                            .h(px(theme::HEADER_BUTTON_SIZE))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hit_slop(px(10.0))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|_this, _event, _window, cx| {
                                    platform_bridge::bridge().hide_keyboard();
                                    cx.emit(WorkspaceContentEvent::OpenQuickAction);
                                }),
                            )
                            .child(
                                svg()
                                    .path("icons/package.svg")
                                    .size(px(18.0))
                                    .text_color(rgb(theme::TEXT_SECONDARY)),
                            ),
                    ),
            )
            .child({
                let overlay_visible = self.overlay_visible;
                let connecting_view = self.connecting_view.clone();
                div()
                    .flex_1()
                    .relative()
                    .child(self.main_view.clone())
                    .when(overlay_visible, |d: Div| {
                        d.child(
                            div()
                                .absolute()
                                .top_0()
                                .left_0()
                                .right_0()
                                .bottom_0()
                                .bg(rgb(theme::BG_PRIMARY))
                                // Block pointer events from reaching the terminal underneath.
                                .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                    cx.stop_propagation();
                                })
                                .child(connecting_view),
                        )
                    })
            })
    }
}

pub struct WorkspaceView {
    drawer_host: Entity<DrawerHost>,
    workspace_content: Entity<WorkspaceContent>,
    workspace_drawer: Entity<WorkspaceDrawer>,
    session_handle: SessionHandle,
    session_state: SessionState,
    workspace_state: crate::workspace_state::WorkspaceState,
    /// `(terminal_id, view)` pairs in creation order; pending slots use `PENDING_TERMINAL_ID`.
    terminal_views: Vec<(String, Entity<TerminalView>)>,
    pub active_terminal_id: Option<String>,
    /// Filled by ZedraApp after `terminal_create` resolves.
    pub pending_terminal_id: SharedPendingSlot<String>,
    /// Filled by ZedraApp on session resume with existing server-side terminal IDs.
    pub pending_existing_terminals: SharedPendingSlot<Vec<String>>,
    pending_file: SharedPendingSlot<(String, String, bool)>,
    pending_git_diff: SharedPendingSlot<(String, Option<FileDiff>)>,
    pending_git_item_action: SharedPendingSlot<GitItemMenuAction>,
    pending_git_operation_result: SharedPendingSlot<(GitIndexOperation, Result<(), String>)>,
    pending_git_commit_request: SharedPendingSlot<(String, Vec<String>)>,
    pending_git_commit_result: SharedPendingSlot<Result<String, String>>,
    /// Set by the native alert callback when user confirms terminal deletion.
    pending_terminal_delete: SharedPendingSlot<String>,
    /// Terminal IDs whose initial resize RPC has completed; used to set connected=true.
    pending_terminal_ready: SharedPendingSlot<String>,
    _subscriptions: Vec<Subscription>,
    /// Polling task that checks pending slots and notifies view.
    _poll_task: Task<()>,
}

impl EventEmitter<WorkspaceEvent> for WorkspaceView {}

impl WorkspaceView {
    pub fn new(
        session_handle: SessionHandle,
        session_state: SessionState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut subscriptions = Vec::new();

        let pending_file: SharedPendingSlot<(String, String, bool)> = shared_pending_slot();
        let pending_git_diff: SharedPendingSlot<(String, Option<FileDiff>)> = shared_pending_slot();
        let pending_git_item_action: SharedPendingSlot<GitItemMenuAction> = shared_pending_slot();
        let pending_git_operation_result: SharedPendingSlot<(
            GitIndexOperation,
            Result<(), String>,
        )> = shared_pending_slot();
        let pending_git_commit_request: SharedPendingSlot<(String, Vec<String>)> =
            shared_pending_slot();
        let pending_git_commit_result: SharedPendingSlot<Result<String, String>> =
            shared_pending_slot();
        let pending_terminal_id: SharedPendingSlot<String> = shared_pending_slot();
        let pending_existing_terminals: SharedPendingSlot<Vec<String>> = shared_pending_slot();
        let pending_terminal_ready: SharedPendingSlot<String> = shared_pending_slot();

        let (columns, rows, cell_width, line_height) = compute_terminal_dimensions(window);
        let terminal_view =
            cx.new(|cx| TerminalView::new(columns, rows, cell_width, line_height, cx));
        terminal_view.update(cx, |view, _cx| {
            view.set_keyboard_request(keyboard::make_keyboard_handler());
            view.set_is_keyboard_visible_fn(keyboard::make_is_keyboard_visible());
            view.set_status("Connecting...".to_string());
        });

        let sub = cx.subscribe(
            &terminal_view,
            |this: &mut WorkspaceView, _terminal, _event: &DisconnectRequested, cx| {
                tracing::info!("DisconnectRequested from terminal view");
                this.disconnect(cx);
            },
        );
        subscriptions.push(sub);

        let initial_terminals = vec![(PENDING_TERMINAL_ID.to_string(), terminal_view.clone())];

        let workspace_content = cx.new(|cx| {
            WorkspaceContent::new(
                terminal_view.into(),
                "Terminal",
                session_handle.clone(),
                session_state.clone(),
                cx,
            )
        });

        let workspace_drawer = cx.new(|cx| WorkspaceDrawer::new(cx));
        workspace_drawer.update(cx, |drawer, cx| {
            let workdir = session_state.get().snapshot.workdir;
            drawer.set_session_handle(session_handle.clone(), workdir, cx);
            drawer.set_session_state(session_state.clone());
        });

        let drawer_host = cx.new(|cx| DrawerHost::new(workspace_content.clone().into(), cx));
        drawer_host.update(cx, |host, _cx| {
            host.set_drawer(workspace_drawer.clone().into());
        });

        // workspace_content events
        {
            let drawer_host_clone = drawer_host.clone();
            let sub = cx.subscribe_in(
                &workspace_content,
                window,
                move |_this: &mut WorkspaceView,
                      _emitter,
                      event: &WorkspaceContentEvent,
                      _window,
                      cx| match event {
                    WorkspaceContentEvent::ToggleDrawer => {
                        if drawer_host_clone.read(cx).is_open() {
                            drawer_host_clone.update(cx, |host, cx| host.close(cx));
                        } else {
                            drawer_host_clone.update(cx, |host, cx| host.open(cx));
                        }
                    }
                    WorkspaceContentEvent::OpenQuickAction => {
                        cx.emit(WorkspaceEvent::OpenQuickAction);
                    }
                },
            );
            subscriptions.push(sub);
        }

        // workspace_drawer events
        {
            let drawer_host_clone = drawer_host.clone();
            let workspace_drawer_clone = workspace_drawer.clone();
            let workspace_content_clone = workspace_content.clone();
            let pending_file_clone = pending_file.clone();
            let pending_git_diff_clone = pending_git_diff.clone();
            let pending_git_item_action_clone = pending_git_item_action.clone();
            let pending_git_commit_request_clone = pending_git_commit_request.clone();
            let pending_terminal_id_clone = pending_terminal_id.clone();

            let sub = cx.subscribe_in(
                &workspace_drawer,
                window,
                move |this: &mut WorkspaceView,
                      _emitter: &Entity<WorkspaceDrawer>,
                      event: &WorkspaceDrawerEvent,
                      window: &mut Window,
                      cx: &mut Context<WorkspaceView>| {
                    match event {
                        WorkspaceDrawerEvent::GoHome => {
                            drawer_host_clone.update(cx, |host, cx| host.close(cx));
                            cx.emit(WorkspaceEvent::GoHome);
                        }
                        WorkspaceDrawerEvent::CloseRequested => {
                            drawer_host_clone.update(cx, |host, cx| host.close(cx));
                        }
                        WorkspaceDrawerEvent::ShowConnectingOverlay => {
                            drawer_host_clone.update(cx, |host, cx| host.close(cx));
                            workspace_content_clone.update(cx, |content, cx| {
                                content.show_overlay(cx);
                            });
                        }
                        WorkspaceDrawerEvent::DisconnectRequested => {
                            tracing::info!("DisconnectRequested from session panel");
                            drawer_host_clone.update(cx, |host, cx| host.close(cx));
                            workspace_drawer_clone.update(cx, |drawer, cx| {
                                drawer.reset_for_disconnect(cx);
                            });
                            this.disconnect(cx);
                        }
                        WorkspaceDrawerEvent::NewTerminalRequested => {
                            drawer_host_clone.update(cx, |host, cx| host.close(cx));
                            let (columns, rows, cell_width, line_height) =
                                compute_terminal_dimensions(window);
                            let cols_u16 = columns as u16;
                            let rows_u16 = rows as u16;

                            let terminal_view = cx.new(|cx| {
                                TerminalView::new(columns, rows, cell_width, line_height, cx)
                            });
                            terminal_view.update(cx, |view, _cx| {
                                view.set_keyboard_request(keyboard::make_keyboard_handler());
                                view.set_is_keyboard_visible_fn(
                                    keyboard::make_is_keyboard_visible(),
                                );
                                view.set_status("Creating terminal...".to_string());
                            });
                            workspace_content_clone.update(cx, |content, cx| {
                                // Active terminal ID not yet known (pending RPC); set to None so
                                // the header shows "Terminal" until switch_to_terminal is called.
                                content.set_main_view(terminal_view.clone().into(), "Terminal", cx);
                            });
                            this.terminal_views
                                .push((PENDING_TERMINAL_ID.to_string(), terminal_view));

                            let handle = this.session_handle.clone();
                            let ptid = pending_terminal_id_clone.clone();
                            let term_count = this.terminal_views.len();
                            zedra_session::session_runtime().spawn(async move {
                                match handle.terminal_create(cols_u16, rows_u16).await {
                                    Ok(term_id) => {
                                        tracing::info!("terminal created: id={}", term_id);
                                        zedra_telemetry::send(Event::TerminalOpened {
                                            source: "user_action",
                                            terminal_count: term_count,
                                        });
                                        ptid.set(term_id);
                                    }
                                    Err(e) => {
                                        tracing::error!("Failed to create terminal: {}", e);
                                    }
                                }
                            });
                            cx.notify();
                        }
                        WorkspaceDrawerEvent::TerminalSelected(tid) => {
                            drawer_host_clone.update(cx, |host, cx| host.close(cx));
                            let tid = tid.clone();
                            // Record intent even if the view doesn't exist yet — the
                            // pending_existing_terminals block will honor this when it creates
                            // views (race window: reattach_terminals fires notify_state_change
                            // before app.rs has set pending_existing_terminals).
                            this.active_terminal_id = Some(tid.clone());
                            this.switch_to_terminal(&tid, cx);
                        }
                        WorkspaceDrawerEvent::FileSelected(path) => {
                            drawer_host_clone.update(cx, |host, cx| host.close(cx));
                            if !path.is_empty() {
                                let path = path.clone();
                                let filename = path.rsplit('/').next().unwrap_or(&path).to_string();
                                let loading = cx.new(|_cx| FileLoadingView);
                                workspace_content_clone.update(cx, |c, cx| {
                                    c.set_main_view(loading.into(), filename.clone(), cx);
                                });
                                let handle = this.session_handle.clone();
                                let filename_clone = filename.clone();
                                let pfile = pending_file_clone.clone();
                                zedra_session::session_runtime().spawn(async move {
                                    match handle.fs_read(&path).await {
                                        Ok(result) => {
                                            pfile.set((
                                                filename_clone,
                                                result.content,
                                                result.too_large,
                                            ));
                                        }
                                        Err(e) => {
                                            tracing::error!("fs/read failed for {}: {}", path, e);
                                        }
                                    }
                                });
                            }
                        }
                        WorkspaceDrawerEvent::TerminalDeleteRequested(tid) => {
                            this.request_terminal_delete(tid.clone(), cx);
                        }
                        WorkspaceDrawerEvent::TerminalReordered {
                            dragged_id,
                            target_id,
                        } => {
                            let dragged_id = dragged_id.clone();
                            let target_id = target_id.clone();
                            if let Some(from) = this
                                .terminal_views
                                .iter()
                                .position(|(id, _)| id == &dragged_id)
                            {
                                let entry = this.terminal_views.remove(from);
                                if target_id.is_empty() {
                                    this.terminal_views.push(entry);
                                } else if let Some(to) = this
                                    .terminal_views
                                    .iter()
                                    .position(|(id, _)| id == &target_id)
                                {
                                    this.terminal_views.insert(to, entry);
                                } else {
                                    this.terminal_views.push(entry);
                                }
                                this.sync_terminal_order_to_drawer(cx);
                            }
                        }
                        WorkspaceDrawerEvent::GitFileSelected(path, section) => {
                            drawer_host_clone.update(cx, |host, cx| host.close(cx));
                            this.open_git_diff(
                                path.clone(),
                                *section,
                                workspace_content_clone.clone(),
                                pending_git_diff_clone.clone(),
                                cx,
                            );
                        }
                        WorkspaceDrawerEvent::GitFileLongPressed(path, section) => {
                            let path = path.clone();
                            let section = *section;
                            let pending = pending_git_item_action_clone.clone();
                            let display_path = path.clone();
                            let main_action_label = match section {
                                GitFileSection::Staged => "Unstage",
                                GitFileSection::Unstaged | GitFileSection::Untracked => "Stage",
                            };
                            platform_bridge::show_selection(
                                "",
                                &display_path,
                                vec![
                                    AlertButton::default(main_action_label),
                                    AlertButton::default("Open Diff"),
                                ],
                                move |selection| {
                                    let action = match selection {
                                        Some(0) => Some(match section {
                                            GitFileSection::Staged => {
                                                GitItemMenuAction::Unstage(path.clone())
                                            }
                                            GitFileSection::Unstaged
                                            | GitFileSection::Untracked => {
                                                GitItemMenuAction::Stage(path.clone())
                                            }
                                        }),
                                        Some(1) => Some(GitItemMenuAction::OpenDiff {
                                            path: path.clone(),
                                            section,
                                        }),
                                        _ => None,
                                    };
                                    if let Some(action) = action {
                                        pending.set(action);
                                    }
                                },
                            );
                        }
                        WorkspaceDrawerEvent::GitCommitRequested { message, paths } => {
                            let message = message.trim().to_string();
                            let paths = paths.clone();
                            if message.is_empty() || paths.is_empty() {
                                return;
                            }

                            let file_label = if paths.len() == 1 {
                                "1 staged file".to_string()
                            } else {
                                format!("{} staged files", paths.len())
                            };
                            let confirm_message = format!("Commit {file_label}?\n\n{message}");
                            let pending = pending_git_commit_request_clone.clone();
                            platform_bridge::bridge().hide_keyboard();
                            platform_bridge::show_alert(
                                "",
                                &confirm_message,
                                vec![
                                    AlertButton::default("Commit"),
                                    AlertButton::cancel("Cancel"),
                                ],
                                move |button_index| {
                                    if button_index == 0 {
                                        pending.set((message.clone(), paths.clone()));
                                    }
                                },
                            );
                        }
                    }
                },
            );
            subscriptions.push(sub);
        }

        // Start polling task to check pending slots and notify view
        let poll_pending_terminal_id = pending_terminal_id.clone();
        let poll_pending_existing = pending_existing_terminals.clone();
        let poll_pending_file = pending_file.clone();
        let poll_pending_git_diff = pending_git_diff.clone();
        let poll_pending_terminal_ready = pending_terminal_ready.clone();
        let poll_task = cx.spawn(async move |weak, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(32))
                    .await;
                let has_pending = poll_pending_terminal_id.has_pending()
                    || poll_pending_existing.has_pending()
                    || poll_pending_file.has_pending()
                    || poll_pending_git_diff.has_pending()
                    || poll_pending_terminal_ready.has_pending();
                if has_pending {
                    if weak.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                }
            }
        });

        Self {
            drawer_host,
            workspace_content,
            workspace_drawer,
            session_handle,
            session_state,
            terminal_views: initial_terminals,
            active_terminal_id: None,
            pending_terminal_id,
            pending_existing_terminals,
            pending_file,
            pending_git_diff,
            pending_git_item_action,
            pending_git_operation_result,
            pending_git_commit_request,
            pending_git_commit_result,
            pending_terminal_delete: shared_pending_slot(),
            pending_terminal_ready,
            workspace_state: crate::workspace_state::WorkspaceState::default(),
            _subscriptions: subscriptions,
            _poll_task: poll_task,
        }
    }

    /// Returns the live terminal IDs (excluding pending) and the active terminal ID.
    /// Used by ZedraApp to update the workspace's WorkspaceState each render.
    pub fn terminal_state(&self) -> (Vec<String>, Option<String>) {
        let ids: Vec<String> = self
            .terminal_views
            .iter()
            .filter(|(id, _)| id.as_str() != PENDING_TERMINAL_ID)
            .map(|(id, _)| id.clone())
            .collect();
        (ids, self.active_terminal_id.clone())
    }

    /// Update the workspace state used for header display and drawer subtitle.
    /// Called by ZedraApp to propagate the live WorkspaceState into all display components.
    pub fn set_workspace_state(
        &mut self,
        state: crate::workspace_state::WorkspaceState,
        cx: &mut Context<Self>,
    ) {
        self.workspace_state = state.clone();
        self.workspace_content
            .update(cx, |c, cx| c.set_workspace_state(state.clone(), cx));
        self.workspace_drawer.update(cx, |drawer, cx| {
            drawer.set_workspace_state(state, cx);
        });
    }

    /// Called when this workspace becomes the active workspace.
    /// Notifies session-dependent children to re-render and reloads the file explorer.
    pub fn on_activate(&mut self, cx: &mut Context<Self>) {
        self.workspace_content.update(cx, |_, cx| cx.notify());
        let handle = self.session_handle.clone();
        let workdir = self.session_state.get().snapshot.workdir;
        self.workspace_drawer.update(cx, |drawer, cx| {
            drawer.set_session_handle(handle, workdir, cx);
            drawer.on_activate(cx);
        });
        cx.notify();
    }

    /// Programmatically disconnect this workspace (e.g. user deleted a saved workspace).
    pub fn disconnect(&mut self, cx: &mut Context<Self>) {
        self.session_handle.clear_session();
        self.terminal_views.clear();
        self.active_terminal_id = None;
        cx.emit(WorkspaceEvent::Disconnected);
        cx.notify();
    }

    /// Wire a TerminalView to the session: output buffer, send bytes, resize fn, and status.
    ///
    /// Call this whenever a real terminal ID has been assigned and the view needs to be
    /// connected to the remote PTY. The keyboard callbacks are set here too.
    fn wire_terminal_view(
        view: &Entity<TerminalView>,
        handle: &SessionHandle,
        tid: &str,
        status: &str,
        cx: &mut Context<Self>,
    ) {
        let terminal = handle.terminal(tid);
        let handle_clone = handle.clone();
        let tid_clone = tid.to_string();
        view.update(cx, |v, cx| {
            v.set_keyboard_request(keyboard::make_keyboard_handler());
            v.set_is_keyboard_visible_fn(keyboard::make_is_keyboard_visible());
            if let Some(ref t) = terminal {
                v.set_output_buffer(t.output.clone(), t.needs_render.clone());
                v.set_send_bytes(t.make_input_fn());
                v.start_output_listener(t.subscribe_output(), cx);
            }
            let h = handle_clone.clone();
            let t = tid_clone.clone();
            v.set_resize_fn(Box::new(move |cols, rows| {
                let h2 = h.clone();
                let t2 = t.clone();
                zedra_session::session_runtime().spawn(async move {
                    if let Err(e) = h2.terminal_resize(&t2, cols, rows).await {
                        tracing::warn!("Remote PTY resize failed: {}", e);
                    }
                });
            }));
            v.set_connected(true);
            v.set_status(status.to_string());
        });
    }

    fn open_git_diff(
        &mut self,
        path: String,
        section: GitFileSection,
        workspace_content: Entity<WorkspaceContent>,
        pending_git_diff: SharedPendingSlot<(String, Option<FileDiff>)>,
        cx: &mut Context<Self>,
    ) {
        let filename = path.rsplit('/').next().unwrap_or(&path).to_string();
        let loading = cx.new(|_cx| FileLoadingView);
        workspace_content.update(cx, |content, cx| {
            content.set_main_view(loading.into(), filename.clone(), cx);
        });

        if !self.session_state.phase().is_connected() {
            tracing::warn!("git diff requested while disconnected, ignoring");
            return;
        }

        let handle = self.session_handle.clone();
        zedra_session::session_runtime().spawn(async move {
            const MAX_DIFF_BYTES: usize = 200 * 1024;

            let maybe_diff: Option<FileDiff> = match section {
                GitFileSection::Untracked => match handle.fs_read(&path).await {
                    Ok(result) if result.too_large => None,
                    Ok(result) => {
                        let content = result.content;
                        if content.len() > MAX_DIFF_BYTES {
                            None
                        } else {
                            let lines: Vec<_> =
                                content.lines().map(|line| format!("+{}", line)).collect();
                            let hunk_body = lines.join("\n");
                            let fake_diff = format!(
                                "--- /dev/null\n+++ b/{}\n@@ -0,0 +1,{} @@\n{}\n",
                                path,
                                lines.len(),
                                hunk_body
                            );
                            Some(parse_unified_diff(&fake_diff).into_iter().next().unwrap_or(
                                FileDiff {
                                    old_path: path.clone(),
                                    new_path: path.clone(),
                                    hunks: Vec::new(),
                                },
                            ))
                        }
                    }
                    Err(error) => {
                        tracing::error!("fs_read failed for {}: {}", path, error);
                        return;
                    }
                },
                GitFileSection::Staged | GitFileSection::Unstaged => {
                    let staged = matches!(section, GitFileSection::Staged);
                    let diff_text = match handle.git_diff(Some(&path), staged).await {
                        Ok(text) => text,
                        Err(error) => {
                            tracing::error!("git_diff RPC failed for {}: {}", path, error);
                            return;
                        }
                    };
                    if diff_text.len() > MAX_DIFF_BYTES {
                        None
                    } else {
                        let diffs = parse_unified_diff(&diff_text);
                        Some(
                            diffs
                                .into_iter()
                                .find(|diff| diff.new_path == path)
                                .unwrap_or(FileDiff {
                                    old_path: path.clone(),
                                    new_path: path.clone(),
                                    hunks: Vec::new(),
                                }),
                        )
                    }
                }
            };

            pending_git_diff.set((filename, maybe_diff));
        });
    }

    /// Push the current `terminal_views` order to the workspace drawer.
    /// Call this after any operation that changes the terminal list or order.
    fn sync_terminal_order_to_drawer(&self, cx: &mut Context<Self>) {
        let order: Vec<String> = self
            .terminal_views
            .iter()
            .filter(|(id, _)| id != PENDING_TERMINAL_ID)
            .map(|(id, _)| id.clone())
            .collect();
        self.workspace_drawer.update(cx, |drawer, cx| {
            drawer.set_terminal_order(order, cx);
        });
    }

    /// Show a native confirmation dialog and delete the terminal on confirm.
    pub fn request_terminal_delete(&mut self, tid: String, _cx: &mut Context<Self>) {
        let index = self
            .terminal_views
            .iter()
            .position(|(id, _)| id == &tid)
            .map(|i| i + 1)
            .unwrap_or(1);
        let pending = self.pending_terminal_delete.clone();
        platform_bridge::show_alert(
            "",
            &format!("Delete Terminal {}?", index),
            vec![
                AlertButton::destructive("Delete"),
                AlertButton::cancel("Cancel"),
            ],
            move |button_index| {
                if button_index == 0 {
                    pending.set(tid.clone());
                }
            },
        );
    }

    /// Switch the main view to a specific terminal by ID.
    pub fn switch_to_terminal(&mut self, terminal_id: &str, cx: &mut Context<Self>) {
        if let Some((_, view)) = self.terminal_views.iter().find(|(id, _)| id == terminal_id) {
            let view = view.clone();
            let tid = terminal_id.to_string();
            self.workspace_content.update(cx, |content, cx| {
                content.set_main_terminal(view.into(), tid.clone(), cx);
            });
            self.active_terminal_id = Some(tid.clone());
            if let Some(t) = self.session_handle.terminal(&tid) {
                active_terminal::set_active_input(t.make_input_fn());
            }
            self.workspace_drawer.update(cx, |drawer, cx| {
                drawer.set_active_terminal(Some(tid), cx);
            });
        }
    }
}

impl Render for WorkspaceView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Session resume: replace the placeholder with views for each existing terminal.
        if let Some(existing_ids) = self.pending_existing_terminals.take() {
            if !existing_ids.is_empty() {
                // Preserve the user's terminal order across reconnects.
                // Sort server-provided IDs to match our previous order, appending new ones.
                let prev_order: Vec<String> = self
                    .terminal_views
                    .iter()
                    .filter(|(id, _)| id != PENDING_TERMINAL_ID)
                    .map(|(id, _)| id.clone())
                    .collect();
                let mut ordered_ids: Vec<String> = prev_order
                    .iter()
                    .filter(|id| existing_ids.contains(id))
                    .cloned()
                    .collect();
                for id in &existing_ids {
                    if !ordered_ids.contains(id) {
                        ordered_ids.push(id.clone());
                    }
                }

                // Build a lookup of existing views so we can reuse them across reconnects.
                // Reusing views preserves VTE state (scroll history, cursor, colours), so the
                // terminal content is visible immediately after reconnect instead of blank.
                // Without this, old views drain the shared output buffer while still in the
                // element tree during reconnect, leaving new views with an empty buffer.
                let existing_views: std::collections::HashMap<String, Entity<TerminalView>> =
                    std::mem::take(&mut self.terminal_views)
                        .into_iter()
                        .filter(|(id, _)| id != PENDING_TERMINAL_ID)
                        .collect();

                // Prefetch file explorer + git content in parallel during resume.
                self.workspace_drawer
                    .update(cx, |d, cx| d.prefetch_for_resume(cx));

                let (columns, rows, cell_width, line_height) = compute_terminal_dimensions(window);
                let cols_u16 = columns as u16;
                let rows_u16 = rows as u16;
                for id in &ordered_ids {
                    // Reuse an existing view to preserve VTE state; create a fresh one for new
                    // terminals that weren't present in the previous connection.
                    let terminal_view = if let Some(view) = existing_views.get(id) {
                        view.clone()
                    } else {
                        cx.new(|cx| TerminalView::new(columns, rows, cell_width, line_height, cx))
                    };
                    // Re-wire to the new connection's output buffer (same RemoteTerminal Arc on
                    // reconnect) and keep disconnected until resize RPC confirms dimensions.
                    Self::wire_terminal_view(
                        &terminal_view,
                        &self.session_handle,
                        id,
                        "Resuming",
                        cx,
                    );
                    terminal_view.update(cx, |v, _cx| v.set_connected(false));
                    // Sync terminal dimensions to server PTY; mark connected on success.
                    let handle_resize = self.session_handle.clone();
                    let tid_resize = id.clone();
                    let ready_slot = self.pending_terminal_ready.clone();
                    zedra_session::session_runtime().spawn(async move {
                        if let Err(e) = handle_resize
                            .terminal_resize(&tid_resize, cols_u16, rows_u16)
                            .await
                        {
                            tracing::warn!("Initial resize on session resume failed: {}", e);
                        }
                        ready_slot.set(tid_resize);
                    });
                    self.terminal_views.push((id.clone(), terminal_view));
                }

                // Sync client order to the drawer so it renders in the preserved order.
                self.sync_terminal_order_to_drawer(cx);

                // Switch to the terminal the user tapped (if they tapped a card before
                // views were created), otherwise fall back to the first terminal.
                let target_id = self
                    .active_terminal_id
                    .as_ref()
                    .filter(|id| ordered_ids.contains(id))
                    .cloned()
                    .or_else(|| self.terminal_views.first().map(|(id, _)| id.clone()));
                if let Some(tid) = target_id {
                    self.switch_to_terminal(&tid, cx);
                }
            }
        }

        if let Some((filename, content, too_large)) = self.pending_file.take() {
            let fname = filename.clone();
            if too_large {
                let msg = format!("File too large to preview\n(>500 KB)");
                let placeholder = cx.new(move |_cx| FileTooLargeView { message: msg });
                self.workspace_content.update(cx, |c, cx| {
                    c.set_main_view(placeholder.into(), fname, cx);
                });
            } else {
                let editor_view = cx.new(|cx| EditorView::with_filename(content, &filename, cx));
                self.workspace_content.update(cx, |c, cx| {
                    c.set_main_view(editor_view.into(), fname, cx);
                });
            }
        }

        if let Some((filename, maybe_diff)) = self.pending_git_diff.take() {
            let title = format!("Diff: {}", filename);
            if let Some(diff) = maybe_diff {
                let path = diff.new_path.clone();
                let diff_view = cx.new(|cx| GitDiffView::new(diff, path, cx));
                self.workspace_content.update(cx, |c, cx| {
                    c.set_main_view(diff_view.into(), title, cx);
                });
            } else {
                let msg = "File too large to diff\n(>200 KB)".to_string();
                let placeholder = cx.new(move |_cx| FileTooLargeView { message: msg });
                self.workspace_content.update(cx, |c, cx| {
                    c.set_main_view(placeholder.into(), title, cx);
                });
            }
        }

        if let Some(action) = self.pending_git_item_action.take() {
            match action {
                GitItemMenuAction::OpenDiff { path, section } => {
                    self.drawer_host.update(cx, |host, cx| host.close(cx));
                    self.open_git_diff(
                        path,
                        section,
                        self.workspace_content.clone(),
                        self.pending_git_diff.clone(),
                        cx,
                    );
                }
                GitItemMenuAction::Stage(path) => {
                    let handle = self.session_handle.clone();
                    let pending_result = self.pending_git_operation_result.clone();
                    zedra_session::session_runtime().spawn(async move {
                        let result = handle
                            .git_stage(&[path])
                            .await
                            .map_err(|error| error.to_string());
                        pending_result.set((GitIndexOperation::Stage, result));
                    });
                }
                GitItemMenuAction::Unstage(path) => {
                    let handle = self.session_handle.clone();
                    let pending_result = self.pending_git_operation_result.clone();
                    zedra_session::session_runtime().spawn(async move {
                        let result = handle
                            .git_unstage(&[path])
                            .await
                            .map_err(|error| error.to_string());
                        pending_result.set((GitIndexOperation::Unstage, result));
                    });
                }
            }
        }

        if let Some((operation, result)) = self.pending_git_operation_result.take() {
            match result {
                Ok(()) => {
                    self.workspace_drawer.update(cx, |drawer, _cx| {
                        drawer.refresh_git_status();
                    });
                }
                Err(error) => {
                    let title = match operation {
                        GitIndexOperation::Stage => "Stage failed.",
                        GitIndexOperation::Unstage => "Unstage failed.",
                    };
                    let message = if error.trim().is_empty() {
                        title.to_string()
                    } else {
                        format!("{title}\n\n{error}")
                    };
                    platform_bridge::show_alert(
                        "",
                        &message,
                        vec![AlertButton::default("OK")],
                        |_| {},
                    );
                }
            }
        }

        if let Some((message, paths)) = self.pending_git_commit_request.take() {
            self.workspace_drawer.update(cx, |drawer, cx| {
                drawer.set_git_committing(true, cx);
            });

            let handle = self.session_handle.clone();
            let pending_result = self.pending_git_commit_result.clone();
            zedra_session::session_runtime().spawn(async move {
                let result = handle
                    .git_commit(&message, &paths)
                    .await
                    .map_err(|error| error.to_string());
                pending_result.set(result);
            });
        }

        if let Some(result) = self.pending_git_commit_result.take() {
            self.workspace_drawer.update(cx, |drawer, cx| {
                drawer.set_git_committing(false, cx);
            });

            match result {
                Ok(_) => {
                    platform_bridge::bridge().hide_keyboard();
                    self.workspace_drawer.update(cx, |drawer, cx| {
                        drawer.clear_git_commit_message(cx);
                        drawer.refresh_git_status();
                    });
                }
                Err(error) => {
                    let message = if error.trim().is_empty() {
                        "Commit failed.".to_string()
                    } else {
                        format!("Commit failed.\n\n{error}")
                    };
                    platform_bridge::show_alert(
                        "",
                        &message,
                        vec![AlertButton::default("OK")],
                        |_| {},
                    );
                }
            }
        }

        if let Some(term_id) = self.pending_terminal_id.take() {
            if let Some(entry) = self
                .terminal_views
                .iter_mut()
                .rev()
                .find(|(id, _)| id == PENDING_TERMINAL_ID)
            {
                entry.0 = term_id.clone();
                let view = entry.1.clone();
                Self::wire_terminal_view(&view, &self.session_handle, &term_id, "Connected", cx);
            }
            self.sync_terminal_order_to_drawer(cx);
            self.switch_to_terminal(&term_id, cx);
        }

        if let Some(tid) = self.pending_terminal_delete.take() {
            let was_active = self.active_terminal_id.as_deref() == Some(tid.as_str());
            self.terminal_views.retain(|(id, _)| id != &tid);
            zedra_telemetry::send(Event::TerminalClosed {
                remaining: self.terminal_views.len(),
            });

            let handle = self.session_handle.clone();
            let tid_close = tid.clone();
            zedra_session::session_runtime().spawn(async move {
                if let Err(e) = handle.terminal_close(&tid_close).await {
                    tracing::error!("terminal_close failed: {}", e);
                }
            });

            self.sync_terminal_order_to_drawer(cx);
            if self.terminal_views.is_empty() {
                self.active_terminal_id = None;
                self.workspace_drawer.update(cx, |drawer, cx| {
                    drawer.set_active_terminal(None, cx);
                });
                let placeholder = cx.new(|_| NoTerminalPlaceholder);
                self.workspace_content.update(cx, |content, cx| {
                    content.set_main_view(placeholder.into(), "Terminals", cx);
                });
            } else if was_active {
                let new_id = self.terminal_views[0].0.clone();
                self.switch_to_terminal(&new_id, cx);
            }
        }

        // Mark resumed terminals connected once their initial resize RPC completes.
        if let Some(tid) = self.pending_terminal_ready.take() {
            if let Some((_, view)) = self.terminal_views.iter().find(|(id, _)| id == &tid) {
                view.update(cx, |v, _cx| {
                    v.set_connected(true);
                    v.set_status("Connected".to_string());
                });
            }
        }

        div().size_full().child(self.drawer_host.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::WorkspaceContent;
    use crate::workspace_state::WorkspaceState;

    #[test]
    fn cached_workspace_context_requires_session_and_display_data() {
        let with_context = WorkspaceState::update_inner(WorkspaceState::default(), |s| {
            s.session_id = "session-1".into();
            s.project_name = "zedra".into();
        });
        assert!(WorkspaceContent::workspace_has_cached_context(
            &with_context
        ));

        let without_session = WorkspaceState::update_inner(WorkspaceState::default(), |s| {
            s.project_name = "zedra".into();
        });
        assert!(!WorkspaceContent::workspace_has_cached_context(
            &without_session
        ));

        let without_display_fields = WorkspaceState::update_inner(WorkspaceState::default(), |s| {
            s.session_id = "session-1".into();
        });
        assert!(!WorkspaceContent::workspace_has_cached_context(
            &without_display_fields
        ));
    }
}

/// Compute terminal grid dimensions from the current viewport.
/// Returns `(columns, rows, cell_width, line_height)`.
///
/// Calls `load_fonts()` to ensure the monospace font is registered with the
/// GPUI text system before measuring glyph advance width. This is a no-op
/// after the first call — `load_fonts` is idempotent.
pub fn compute_terminal_dimensions(window: &mut Window) -> (usize, usize, Pixels, Pixels) {
    let viewport = window.viewport_size();
    let line_height = px(16.0);

    fonts::load_fonts(window);

    let font = gpui::Font {
        family: fonts::MONO_FONT_FAMILY.into(),
        features: gpui::FontFeatures::default(),
        fallbacks: None,
        weight: gpui::FontWeight::NORMAL,
        style: gpui::FontStyle::Normal,
    };
    let font_size = line_height * 0.75;
    let text_system = window.text_system();
    let font_id = text_system.resolve_font(&font);
    let cell_width = text_system
        .advance(font_id, font_size, 'm')
        .map(|size| size.width)
        .unwrap_or(px(9.0));

    // Subtract chrome (status bar, header, home indicator) so the PTY row count
    // matches what's actually visible, preventing TUI overflow.
    let top_reserved = crate::platform_bridge::status_bar_inset() + theme::HEADER_HEIGHT;
    let bottom_reserved = crate::platform_bridge::home_indicator_inset();
    let terminal_height = viewport.height - px(top_reserved + bottom_reserved);

    let columns = ((viewport.width / cell_width).floor() as usize)
        .saturating_sub(1)
        .clamp(20, 200);
    let rows = ((terminal_height / line_height).floor() as usize)
        .saturating_sub(1)
        .clamp(5, 200);

    (columns, rows, cell_width, line_height)
}
