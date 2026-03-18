// Per-session workspace: DrawerHost + header/main-view stack, wired to a SessionHandle.

use gpui::{prelude::FluentBuilder as _, *};

use crate::active_terminal;
use crate::connecting_view;
use crate::editor::code_editor::EditorView;
use crate::editor::git_diff_view::{FileDiff, GitDiffView, parse_unified_diff};
use crate::fonts;
use crate::keyboard;
use crate::mgpui::DrawerHost;
use crate::pending::{SharedPendingSlot, shared_pending_slot};
use crate::platform_bridge::{self, AlertButton, status_bar_inset};
use crate::theme;
use crate::workspace_drawer::{WorkspaceDrawer, WorkspaceDrawerEvent};
use zedra_session::SessionHandle;
use zedra_terminal::view::{DisconnectRequested, TerminalView};

/// Sentinel terminal ID used before the server assigns a real ID.
const PENDING_TERMINAL_ID: &str = "__pending__";

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

/// Published to HomeView and QuickActionPanel.
#[derive(Clone, Debug)]
pub struct WorkspaceSummary {
    pub index: usize,
    pub project_path: Option<String>,
    pub is_connected: bool,
    pub connect_phase: zedra_session::ConnectPhase,
    pub terminal_count: usize,
    /// Excludes `__pending__` slots; used for direct terminal navigation.
    pub terminal_ids: Vec<String>,
    /// The currently focused terminal ID, if any.
    pub active_terminal_id: Option<String>,
    /// base64-url encoded endpoint address for matching saved workspaces.
    pub endpoint_addr_encoded: Option<String>,
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

/// Header bar `[≡ | title | ⚡]` with a swappable main view below.
/// The connecting overlay is rendered on top of the main view and lingers for
/// ~1s after the session becomes Connected so the workspace can fully repaint
/// before the overlay disappears (avoids a terminal-resume glitch).
pub struct WorkspaceContent {
    pub main_view: AnyView,
    pub header_title: SharedString,
    focus_handle: FocusHandle,
    session_handle: SessionHandle,
    /// Whether the connecting overlay is currently shown.
    overlay_visible: bool,
    /// True once Connected is detected; waiting for the 1s dismiss timer.
    overlay_pending_hide: bool,
    /// Kept alive so the dismiss timer is not cancelled.
    _overlay_task: Option<Task<()>>,
}

impl WorkspaceContent {
    pub fn new(
        main_view: AnyView,
        title: impl Into<SharedString>,
        session_handle: SessionHandle,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            main_view,
            header_title: title.into(),
            focus_handle: cx.focus_handle(),
            session_handle,
            overlay_visible: true,
            overlay_pending_hide: false,
            _overlay_task: None,
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
        // Manage the full opaque overlay (initial connect, resume, failed).
        // Reconnecting gets its own semi-transparent overlay rendered separately.
        let phase = self.session_handle.connect_state().phase;
        let needs_full_overlay =
            !phase.is_connected() && !phase.is_idle() && !phase.is_reconnecting();

        if needs_full_overlay {
            // Active connect/failed phase — ensure full overlay is showing.
            self.overlay_visible = true;
            self.overlay_pending_hide = false;
            self._overlay_task = None;
        } else if self.overlay_visible && !self.overlay_pending_hide {
            // Just became Connected — start 2s linger before hiding.
            self.overlay_pending_hide = true;
            self._overlay_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor()
                    // Better UX, make sure the terminal resuming is fully completed
                    .timer(std::time::Duration::from_millis(2000))
                    .await;
                let _ = this.update(cx, |this: &mut WorkspaceContent, cx| {
                    this.overlay_visible = false;
                    this.overlay_pending_hide = false;
                    this._overlay_task = None;
                    cx.notify();
                });
            }));
        }

        let top_inset = status_bar_inset();
        let title = self.header_title.clone();

        let project_name: Option<SharedString> = {
            let name = self.session_handle.project_name();
            if name.is_empty() {
                None
            } else {
                Some(name.into())
            }
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
                        div().flex_1().flex().items_center().justify_center().child(
                            div()
                                .flex()
                                .flex_col()
                                .items_center()
                                .children(project_name.map(|name| {
                                    div()
                                        .text_color(rgb(theme::TEXT_MUTED))
                                        .text_size(px(theme::FONT_DETAIL))
                                        .child(name)
                                }))
                                .child(
                                    div()
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
                let is_reconnecting = phase.is_reconnecting();
                let overlay_visible = self.overlay_visible;
                let handle = self.session_handle.clone();
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
                                .child(connecting_view::render_connecting(&handle)),
                        )
                    })
                    .when(is_reconnecting, |d: Div| {
                        d.child(connecting_view::render_reconnecting_overlay(&handle))
                    })
            })
    }
}

pub struct WorkspaceView {
    drawer_host: Entity<DrawerHost>,
    workspace_content: Entity<WorkspaceContent>,
    workspace_drawer: Entity<WorkspaceDrawer>,
    session_handle: SessionHandle,
    /// `(terminal_id, view)` pairs in creation order; pending slots use `PENDING_TERMINAL_ID`.
    terminal_views: Vec<(String, Entity<TerminalView>)>,
    pub active_terminal_id: Option<String>,
    /// Filled by ZedraApp after `terminal_create` resolves.
    pub pending_terminal_id: SharedPendingSlot<String>,
    /// Filled by ZedraApp on session resume with existing server-side terminal IDs.
    pub pending_existing_terminals: SharedPendingSlot<Vec<String>>,
    pending_file: SharedPendingSlot<(String, String, bool)>,
    pending_git_diff: SharedPendingSlot<(String, Option<FileDiff>)>,
    /// Set by the native alert callback when user confirms terminal deletion.
    pending_terminal_delete: SharedPendingSlot<String>,
    /// Terminal IDs whose initial resize RPC has completed; used to set connected=true.
    pending_terminal_ready: SharedPendingSlot<String>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<WorkspaceEvent> for WorkspaceView {}

impl WorkspaceView {
    pub fn new(session_handle: SessionHandle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut subscriptions = Vec::new();

        let pending_file: SharedPendingSlot<(String, String, bool)> = shared_pending_slot();
        let pending_git_diff: SharedPendingSlot<(String, Option<FileDiff>)> = shared_pending_slot();
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
                log::info!("DisconnectRequested from terminal view");
                this.disconnect(cx);
            },
        );
        subscriptions.push(sub);

        let initial_terminals = vec![(PENDING_TERMINAL_ID.to_string(), terminal_view.clone())];

        let workspace_content = cx.new(|cx| {
            WorkspaceContent::new(terminal_view.into(), "Terminal", session_handle.clone(), cx)
        });

        let workspace_drawer = cx.new(|cx| WorkspaceDrawer::new(cx));
        workspace_drawer.update(cx, |drawer, cx| {
            drawer.set_session_handle(session_handle.clone(), cx);
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
                        WorkspaceDrawerEvent::DisconnectRequested => {
                            log::info!("DisconnectRequested from session panel");
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
                                content.set_main_view(terminal_view.clone().into(), "Terminal", cx);
                            });
                            this.terminal_views
                                .push((PENDING_TERMINAL_ID.to_string(), terminal_view));

                            let handle = this.session_handle.clone();
                            let ptid = pending_terminal_id_clone.clone();
                            zedra_session::session_runtime().spawn(async move {
                                match handle.terminal_create(cols_u16, rows_u16).await {
                                    Ok(term_id) => {
                                        log::info!("terminal created: id={}", term_id);
                                        ptid.set(term_id);
                                        zedra_session::push_callback(Box::new(|| {}));
                                    }
                                    Err(e) => {
                                        log::error!("Failed to create terminal: {}", e);
                                    }
                                }
                            });
                            cx.notify();
                        }
                        WorkspaceDrawerEvent::TerminalSelected(tid) => {
                            drawer_host_clone.update(cx, |host, cx| host.close(cx));
                            let tid = tid.clone();
                            this.switch_to_terminal(&tid, cx);
                        }
                        WorkspaceDrawerEvent::FileSelected(path) => {
                            drawer_host_clone.update(cx, |host, cx| host.close(cx));
                            if !path.is_empty() {
                                let path = path.clone();
                                let filename = path.rsplit('/').next().unwrap_or(&path).to_string();
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
                                            zedra_session::push_callback(Box::new(|| {}));
                                        }
                                        Err(e) => {
                                            log::error!("fs/read failed for {}: {}", path, e);
                                        }
                                    }
                                });
                            }
                        }
                        WorkspaceDrawerEvent::TerminalDeleteRequested(tid) => {
                            let tid = tid.clone();
                            let index = this
                                .terminal_views
                                .iter()
                                .position(|(id, _)| id == &tid)
                                .map(|i| i + 1)
                                .unwrap_or(1);
                            let pending = this.pending_terminal_delete.clone();
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
                                        zedra_session::push_callback(Box::new(|| {}));
                                    }
                                },
                            );
                        }
                        WorkspaceDrawerEvent::GitFileSelected(path, untracked) => {
                            drawer_host_clone.update(cx, |host, cx| host.close(cx));
                            let path = path.clone();
                            let filename = path.rsplit('/').next().unwrap_or(&path).to_string();
                            if this.session_handle.is_connected() {
                                let handle = this.session_handle.clone();
                                let path_clone = path.clone();
                                let filename_clone = filename.clone();
                                let pgit = pending_git_diff_clone.clone();
                                let is_untracked = *untracked;
                                zedra_session::session_runtime().spawn(async move {
                                    const MAX_DIFF_BYTES: usize = 200 * 1024;
                                    let maybe_diff: Option<FileDiff> = if is_untracked {
                                        // Untracked files have no git diff; read content and
                                        // synthesize an all-added hunk.
                                        match handle.fs_read(&path_clone).await {
                                            Ok(result) if result.too_large => None,
                                            Ok(result) => {
                                                let content = result.content;
                                                if content.len() > MAX_DIFF_BYTES {
                                                    None
                                                } else {
                                                    let lines: Vec<_> = content.lines().map(|l| format!("+{}", l)).collect();
                                                    let hunk_body = lines.join("\n");
                                                    let fake_diff = format!(
                                                        "--- /dev/null\n+++ b/{}\n@@ -0,0 +1,{} @@\n{}\n",
                                                        path_clone,
                                                        lines.len(),
                                                        hunk_body
                                                    );
                                                    Some(parse_unified_diff(&fake_diff)
                                                        .into_iter()
                                                        .next()
                                                        .unwrap_or(FileDiff {
                                                            old_path: path_clone.clone(),
                                                            new_path: path_clone.clone(),
                                                            hunks: Vec::new(),
                                                        }))
                                                }
                                            }
                                            Err(e) => {
                                                log::error!("fs_read failed for {}: {}", path_clone, e);
                                                return;
                                            }
                                        }
                                    } else {
                                        let diff_text = match handle.git_diff(Some(&path_clone), false).await {
                                            Ok(text) if !text.is_empty() => text,
                                            Ok(_) => handle
                                                .git_diff(Some(&path_clone), true)
                                                .await
                                                .unwrap_or_default(),
                                            Err(e) => {
                                                log::error!(
                                                    "git_diff RPC failed for {}: {}",
                                                    path_clone,
                                                    e
                                                );
                                                return;
                                            }
                                        };
                                        if diff_text.len() > MAX_DIFF_BYTES {
                                            None
                                        } else {
                                            let diffs = parse_unified_diff(&diff_text);
                                            Some(diffs
                                                .into_iter()
                                                .find(|d| d.new_path == path_clone)
                                                .unwrap_or(FileDiff {
                                                    old_path: path_clone.clone(),
                                                    new_path: path_clone.clone(),
                                                    hunks: Vec::new(),
                                                }))
                                        }
                                    };
                                    pgit.set((filename_clone, maybe_diff));
                                    zedra_session::push_callback(Box::new(|| {}));
                                });
                            } else {
                                log::warn!("git diff requested while disconnected, ignoring");
                            }
                        }
                    }
                },
            );
            subscriptions.push(sub);
        }

        Self {
            drawer_host,
            workspace_content,
            workspace_drawer,
            session_handle,
            terminal_views: initial_terminals,
            active_terminal_id: None,
            pending_terminal_id,
            pending_existing_terminals,
            pending_file,
            pending_git_diff,
            pending_terminal_delete: shared_pending_slot(),
            pending_terminal_ready,
            _subscriptions: subscriptions,
        }
    }

    /// Returns a summary of this workspace for HomeView / QuickActionPanel.
    pub fn summary(&self, index: usize) -> WorkspaceSummary {
        let cs = self.session_handle.connect_state();
        let is_connected = cs.phase.is_connected();
        let workdir = self.session_handle.workdir();
        let project_path = if workdir.is_empty() {
            None
        } else {
            Some(workdir)
        };
        let terminal_ids: Vec<String> = self
            .terminal_views
            .iter()
            .filter(|(id, _)| id != PENDING_TERMINAL_ID)
            .map(|(id, _)| id.clone())
            .collect();
        let terminal_count = terminal_ids.len();
        let endpoint_addr_encoded = self
            .session_handle
            .endpoint_addr()
            .and_then(|addr| zedra_rpc::pairing::encode_endpoint_addr(&addr).ok());
        WorkspaceSummary {
            index,
            project_path,
            is_connected,
            connect_phase: cs.phase,
            terminal_count,
            terminal_ids,
            active_terminal_id: self.active_terminal_id.clone(),
            endpoint_addr_encoded,
        }
    }

    /// Called when this workspace becomes the active workspace.
    /// Notifies session-dependent children to re-render and reloads the file explorer.
    pub fn on_activate(&mut self, cx: &mut Context<Self>) {
        self.workspace_content.update(cx, |_, cx| cx.notify());
        let handle = self.session_handle.clone();
        self.workspace_drawer.update(cx, |drawer, cx| {
            drawer.set_session_handle(handle, cx);
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
        view.update(cx, |v, _cx| {
            v.set_keyboard_request(keyboard::make_keyboard_handler());
            v.set_is_keyboard_visible_fn(keyboard::make_is_keyboard_visible());
            if let Some(ref t) = terminal {
                v.set_output_buffer(t.output.clone(), t.needs_render.clone());
                v.set_send_bytes(t.make_input_fn());
            }
            let h = handle_clone.clone();
            let t = tid_clone.clone();
            v.set_resize_fn(Box::new(move |cols, rows| {
                let h2 = h.clone();
                let t2 = t.clone();
                zedra_session::session_runtime().spawn(async move {
                    if let Err(e) = h2.terminal_resize(&t2, cols, rows).await {
                        log::warn!("Remote PTY resize failed: {}", e);
                    }
                });
            }));
            v.set_connected(true);
            v.set_status(status.to_string());
        });
    }

    /// Switch the main view to a specific terminal by ID.
    pub fn switch_to_terminal(&mut self, terminal_id: &str, cx: &mut Context<Self>) {
        if let Some((_, view)) = self.terminal_views.iter().find(|(id, _)| id == terminal_id) {
            let view = view.clone();
            let tid = terminal_id.to_string();
            self.workspace_content.update(cx, |content, cx| {
                content.set_main_view(view.into(), "Terminal", cx);
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
                self.terminal_views.clear();
                // Prefetch file explorer + git content in parallel during resume.
                self.workspace_drawer
                    .update(cx, |d, cx| d.prefetch_for_resume(cx));

                let (columns, rows, cell_width, line_height) = compute_terminal_dimensions(window);
                let cols_u16 = columns as u16;
                let rows_u16 = rows as u16;
                for id in &existing_ids {
                    let terminal_view =
                        cx.new(|cx| TerminalView::new(columns, rows, cell_width, line_height, cx));
                    // Wire but keep disconnected until resize RPC confirms dimensions.
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
                            log::warn!("Initial resize on session resume failed: {}", e);
                        }
                        ready_slot.set(tid_resize);
                        zedra_session::push_callback(Box::new(|| {}));
                    });
                    self.terminal_views.push((id.clone(), terminal_view));
                }

                if let Some((first_id, _)) = self.terminal_views.first() {
                    let first_id = first_id.clone();
                    self.switch_to_terminal(&first_id, cx);
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
                let editor_view = cx.new(|cx| EditorView::new(content, cx));
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
            self.switch_to_terminal(&term_id, cx);
        }

        if let Some(tid) = self.pending_terminal_delete.take() {
            let was_active = self.active_terminal_id.as_deref() == Some(tid.as_str());
            self.terminal_views.retain(|(id, _)| id != &tid);

            let handle = self.session_handle.clone();
            let tid_close = tid.clone();
            zedra_session::session_runtime().spawn(async move {
                if let Err(e) = handle.terminal_close(&tid_close).await {
                    log::error!("terminal_close failed: {}", e);
                }
            });

            if was_active {
                if let Some((new_id, _)) = self.terminal_views.first() {
                    let new_id = new_id.clone();
                    self.switch_to_terminal(&new_id, cx);
                } else {
                    self.active_terminal_id = None;
                    self.workspace_drawer.update(cx, |drawer, cx| {
                        drawer.set_active_terminal(None, cx);
                    });
                }
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

    let columns = ((viewport.width / cell_width).floor() as usize)
        .saturating_sub(1)
        .clamp(20, 200);
    let rows = ((viewport.height / line_height).floor() as usize)
        .saturating_sub(1)
        .clamp(5, 200);

    (columns, rows, cell_width, line_height)
}
