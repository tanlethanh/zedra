// WorkspaceView — one per session, owns all workspace state.
// Contains: DrawerHost (left drawer) + WorkspaceContent (header + swappable main view)
// Session access goes through the per-workspace SessionHandle.

use gpui::*;

use crate::editor::code_editor::EditorView;
use crate::editor::git_diff_view::GitDiffView;
use crate::mgpui::DrawerHost;
use crate::pending::{SharedPendingSlot, shared_pending_slot};
use crate::theme;
use crate::workspace_drawer::{WorkspaceDrawer, WorkspaceDrawerEvent};
use zedra_session::SessionHandle;
use zedra_terminal::view::{DisconnectRequested, TerminalView};

// ---------------------------------------------------------------------------
// WorkspaceSummary — published to HomeView and QuickActionPanel
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct WorkspaceSummary {
    pub index: usize,
    pub project_path: Option<String>,
    pub is_connected: bool,
    pub session_state: zedra_session::SessionState,
    pub terminal_count: usize,
    /// Actual terminal IDs (excluding pending slots), used for direct navigation.
    pub terminal_ids: Vec<String>,
    /// base64-url encoded endpoint address, used to match against saved workspaces.
    pub endpoint_addr_encoded: Option<String>,
}

// ---------------------------------------------------------------------------
// WorkspaceEvent — emitted to ZedraApp
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub enum WorkspaceEvent {
    OpenQuickAction,
    Disconnected,
}

// ---------------------------------------------------------------------------
// WorkspaceContentEvent
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub enum WorkspaceContentEvent {
    ToggleDrawer,
    OpenQuickAction,
}

// ---------------------------------------------------------------------------
// WorkspaceContent — header [≡ | title | ⚡] + swappable main view
// ---------------------------------------------------------------------------

pub struct WorkspaceContent {
    pub main_view: AnyView,
    pub header_title: SharedString,
    focus_handle: FocusHandle,
}

impl WorkspaceContent {
    pub fn new(main_view: AnyView, title: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        Self {
            main_view,
            header_title: title.into(),
            focus_handle: cx.focus_handle(),
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
        let top_inset = crate::platform_bridge::status_bar_inset();
        let title = self.header_title.clone();

        // Network badge from active session state + handle reconnect fields
        let (reconnect_attempt, reconnect_reason, next_retry_secs) = zedra_session::active_handle()
            .map_or(
                (0, zedra_session::ReconnectReason::ConnectionLost, 0),
                |h| {
                    (
                        h.reconnect_attempt(),
                        h.reconnect_reason(),
                        h.next_retry_secs(),
                    )
                },
            );
        let badge: Option<(String, String, u32)> = zedra_session::active_session().map(|session| {
            let state = session.state();
            let latency = session.latency_ms();
            let conn_info = session.connection_info();
            let (label, color) = crate::transport_badge::transport_badge_info(
                &state,
                reconnect_attempt,
                &reconnect_reason,
                next_retry_secs,
                latency,
                conn_info.as_ref(),
            );
            // Project name for subtitle
            let project = match &state {
                zedra_session::SessionState::Connected { workdir, .. } if !workdir.is_empty() => {
                    workdir.rsplit('/').next().unwrap_or(workdir).to_string()
                }
                _ => String::new(),
            };
            (project, label, color)
        });

        let project_name = badge.as_ref().and_then(|(p, _, _)| {
            if p.is_empty() {
                None
            } else {
                Some(SharedString::from(p.clone()))
            }
        });
        let badge_element = badge
            .map(|(_, label, color)| crate::transport_badge::render_transport_badge(label, color));

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(theme::BG_PRIMARY))
            .child(div().h(px(top_inset)))
            .child(
                div()
                    .h(px(48.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .px(px(8.0))
                    .border_b_1()
                    .border_color(rgb(theme::BORDER_SUBTLE))
                    // ≡ drawer toggle
                    .child(
                        div()
                            .id("drawer-toggle-btn")
                            .w(px(36.0))
                            .h(px(36.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(6.0))
                            .cursor_pointer()
                            .hover(|s| s.bg(theme::hover_bg()))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|_this, _event, _window, cx| {
                                    crate::platform_bridge::bridge().hide_keyboard();
                                    cx.emit(WorkspaceContentEvent::ToggleDrawer);
                                }),
                            )
                            .child(
                                div()
                                    .text_color(rgb(theme::TEXT_PRIMARY))
                                    .text_size(px(theme::FONT_HEADING))
                                    .child("\u{2630}"),
                            ),
                    )
                    // Title area (flex-1, centered): project name + title + network badge
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
                                )
                                .children(badge_element),
                        ),
                    )
                    // ⚡ quick-action toggle
                    .child(
                        div()
                            .id("quick-action-btn")
                            .w(px(36.0))
                            .h(px(36.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(6.0))
                            .cursor_pointer()
                            .hover(|s| s.bg(theme::hover_bg()))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|_this, _event, _window, cx| {
                                    crate::platform_bridge::bridge().hide_keyboard();
                                    cx.emit(WorkspaceContentEvent::OpenQuickAction);
                                }),
                            )
                            .child(
                                div()
                                    .text_color(rgb(theme::TEXT_PRIMARY))
                                    .text_size(px(theme::FONT_HEADING))
                                    .child("\u{26a1}"),
                            ),
                    ),
            )
            .child(div().flex_1().child(self.main_view.clone()))
    }
}

// ---------------------------------------------------------------------------
// WorkspaceView
// ---------------------------------------------------------------------------

pub struct WorkspaceView {
    drawer_host: Entity<DrawerHost>,
    workspace_content: Entity<WorkspaceContent>,
    workspace_drawer: Entity<WorkspaceDrawer>,
    /// Per-workspace session state (shared with async tasks).
    pub session_handle: SessionHandle,
    /// (terminal_id, view entity) pairs in creation order.
    pub terminal_views: Vec<(String, Entity<TerminalView>)>,
    pub active_terminal_id: Option<String>,
    /// Written by ZedraApp after async terminal_create returns.
    pub pending_terminal_id: SharedPendingSlot<String>,
    /// Written by ZedraApp when an existing session is resumed with server-side terminals.
    /// Carries the full list of terminal IDs to attach; replaces the placeholder terminal.
    pub pending_existing_terminals: SharedPendingSlot<Vec<String>>,
    pending_file: SharedPendingSlot<(String, String)>,
    pending_git_diff: SharedPendingSlot<(String, String, String)>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<WorkspaceEvent> for WorkspaceView {}

impl WorkspaceView {
    pub fn new(session_handle: SessionHandle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut subscriptions = Vec::new();

        let pending_file: SharedPendingSlot<(String, String)> = shared_pending_slot();
        let pending_git_diff: SharedPendingSlot<(String, String, String)> = shared_pending_slot();
        let pending_terminal_id: SharedPendingSlot<String> = shared_pending_slot();
        let pending_existing_terminals: SharedPendingSlot<Vec<String>> = shared_pending_slot();

        // Create initial terminal view
        let (columns, rows, cell_width, line_height) = compute_terminal_dimensions(window);
        let terminal_view =
            cx.new(|cx| TerminalView::new(columns, rows, cell_width, line_height, cx));
        terminal_view.update(cx, |view, _cx| {
            view.set_keyboard_request(crate::keyboard::make_keyboard_handler());
            view.set_is_keyboard_visible_fn(crate::keyboard::make_is_keyboard_visible());
            view.set_status("Connecting...".to_string());
        });

        // Subscribe to terminal disconnect
        let sub = cx.subscribe(
            &terminal_view,
            |this: &mut WorkspaceView, _terminal, _event: &DisconnectRequested, cx| {
                log::info!("DisconnectRequested from terminal view");
                this.session_handle.clear_session();
                this.terminal_views.clear();
                this.active_terminal_id = None;
                cx.emit(WorkspaceEvent::Disconnected);
                cx.notify();
            },
        );
        subscriptions.push(sub);

        let initial_terminals = vec![("__pending__".to_string(), terminal_view.clone())];

        // WorkspaceContent with terminal as initial main view
        let workspace_content =
            cx.new(|cx| WorkspaceContent::new(terminal_view.into(), "Terminal", cx));

        // WorkspaceDrawer
        let workspace_drawer = cx.new(|cx| WorkspaceDrawer::new(cx));

        // DrawerHost
        let drawer_host = cx.new(|cx| DrawerHost::new(workspace_content.clone().into(), cx));
        drawer_host.update(cx, |host, _cx| {
            host.set_drawer(workspace_drawer.clone().into());
        });

        // Subscribe: WorkspaceContent events
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

        // Subscribe: WorkspaceDrawer events
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
                        WorkspaceDrawerEvent::CloseRequested => {
                            drawer_host_clone.update(cx, |host, cx| host.close(cx));
                        }
                        WorkspaceDrawerEvent::DisconnectRequested => {
                            log::info!("DisconnectRequested from session panel");
                            drawer_host_clone.update(cx, |host, cx| host.close(cx));
                            this.session_handle.clear_session();
                            this.terminal_views.clear();
                            this.active_terminal_id = None;
                            workspace_drawer_clone.update(cx, |drawer, cx| {
                                drawer.reset_for_disconnect(cx);
                            });
                            cx.emit(WorkspaceEvent::Disconnected);
                            cx.notify();
                        }
                        WorkspaceDrawerEvent::NewTerminalRequested => {
                            drawer_host_clone.update(cx, |host, cx| host.close(cx));
                            if let Some(session) = zedra_session::active_session() {
                                let (columns, rows, cell_width, line_height) =
                                    compute_terminal_dimensions(window);
                                let cols_u16 = columns as u16;
                                let rows_u16 = rows as u16;

                                let terminal_view = cx.new(|cx| {
                                    TerminalView::new(columns, rows, cell_width, line_height, cx)
                                });
                                terminal_view.update(cx, |view, _cx| {
                                    view.set_keyboard_request(
                                        crate::keyboard::make_keyboard_handler(),
                                    );
                                    view.set_is_keyboard_visible_fn(
                                        crate::keyboard::make_is_keyboard_visible(),
                                    );
                                    view.set_status("Creating terminal...".to_string());
                                });
                                workspace_content_clone.update(cx, |content, cx| {
                                    content.set_main_view(
                                        terminal_view.clone().into(),
                                        "Terminal",
                                        cx,
                                    );
                                });
                                this.terminal_views
                                    .push(("__pending__".to_string(), terminal_view));

                                let ptid = pending_terminal_id_clone.clone();
                                let handle_for_create = this.session_handle.clone();
                                zedra_session::session_runtime().spawn(async move {
                                    match session
                                        .terminal_create(cols_u16, rows_u16, &handle_for_create)
                                        .await
                                    {
                                        Ok(term_id) => {
                                            log::info!("terminal created: id={}", term_id);
                                            ptid.set(term_id);
                                            zedra_session::signal_terminal_data();
                                        }
                                        Err(e) => {
                                            log::error!("Failed to create terminal: {}", e);
                                        }
                                    }
                                });
                            }
                            cx.notify();
                        }
                        WorkspaceDrawerEvent::TerminalSelected(tid) => {
                            drawer_host_clone.update(cx, |host, cx| host.close(cx));
                            let tid = tid.clone();
                            if let Some((_id, view)) =
                                this.terminal_views.iter().find(|(id, _)| id == &tid)
                            {
                                let view = view.clone();
                                workspace_content_clone.update(cx, |content, cx| {
                                    content.set_main_view(view.into(), "Terminal", cx);
                                });
                                this.active_terminal_id = Some(tid.clone());
                                if let Some(session) = zedra_session::active_session() {
                                    session.set_active_terminal(&tid);
                                }
                                workspace_drawer_clone.update(cx, |drawer, cx| {
                                    drawer.set_active_terminal(Some(tid), cx);
                                });
                            }
                            cx.notify();
                        }
                        WorkspaceDrawerEvent::FileSelected(path) => {
                            drawer_host_clone.update(cx, |host, cx| host.close(cx));
                            if !path.is_empty() {
                                let path = path.clone();
                                let filename = path.rsplit('/').next().unwrap_or(&path).to_string();
                                if let Some(session) = zedra_session::active_session() {
                                    let filename_clone = filename.clone();
                                    let pfile = pending_file_clone.clone();
                                    zedra_session::session_runtime().spawn(async move {
                                        match session.fs_read(&path).await {
                                            Ok(content) => {
                                                pfile.set((filename_clone, content));
                                                zedra_session::signal_terminal_data();
                                            }
                                            Err(e) => {
                                                log::error!("fs/read failed for {}: {}", path, e);
                                            }
                                        }
                                    });
                                }
                            }
                        }
                        WorkspaceDrawerEvent::GitFileSelected(path) => {
                            drawer_host_clone.update(cx, |host, cx| host.close(cx));
                            let path = path.clone();
                            let filename = path.rsplit('/').next().unwrap_or(&path).to_string();
                            if let Some(session) = zedra_session::active_session() {
                                let path_clone = path.clone();
                                let filename_clone = filename.clone();
                                let pgit = pending_git_diff_clone.clone();
                                zedra_session::session_runtime().spawn(async move {
                                    match session.git_diff(Some(&path_clone), false).await {
                                        Ok(diff_text) => {
                                            pgit.set((path_clone, filename_clone, diff_text));
                                            zedra_session::signal_terminal_data();
                                        }
                                        Err(e) => {
                                            log::error!(
                                                "git_diff RPC failed for {}: {}",
                                                path_clone,
                                                e
                                            );
                                        }
                                    }
                                });
                            } else {
                                // Fallback: sample data
                                let diffs = crate::editor::git_diff_view::sample_diffs();
                                if let Some(diff) = diffs.into_iter().find(|d| d.new_path == path) {
                                    let diff_view =
                                        cx.new(|cx| GitDiffView::new(diff, path.clone(), cx));
                                    workspace_content_clone.update(cx, |content, cx| {
                                        content.set_main_view(
                                            diff_view.into(),
                                            format!("Diff: {}", filename),
                                            cx,
                                        );
                                    });
                                }
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
            _subscriptions: subscriptions,
        }
    }

    /// Returns a summary of this workspace for HomeView / QuickActionPanel.
    pub fn summary(&self, index: usize) -> WorkspaceSummary {
        let state = self.session_handle.state();
        let is_connected = matches!(state, zedra_session::SessionState::Connected { .. });
        let project_path = match &state {
            zedra_session::SessionState::Connected { workdir, .. } => {
                if workdir.is_empty() {
                    None
                } else {
                    Some(workdir.clone())
                }
            }
            _ => None,
        };
        let terminal_ids: Vec<String> = self
            .terminal_views
            .iter()
            .filter(|(id, _)| id != "__pending__")
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
            session_state: state,
            terminal_count,
            terminal_ids,
            endpoint_addr_encoded,
        }
    }

    /// Called when this workspace becomes the active workspace.
    /// Notifies session-dependent child entities to re-render with the updated global session.
    pub fn on_activate(&mut self, cx: &mut Context<Self>) {
        self.workspace_content.update(cx, |_, cx| cx.notify());
        self.workspace_drawer.update(cx, |_, cx| cx.notify());
        cx.notify();
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
            if let Some(session) = zedra_session::active_session() {
                session.set_active_terminal(&tid);
            }
            self.workspace_drawer.update(cx, |drawer, cx| {
                drawer.set_active_terminal(Some(tid), cx);
            });
        }
    }
}

impl Render for WorkspaceView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Drain pending existing terminals (cold-start session resume).
        // Replaces the placeholder "__pending__" terminal with views for each resumed terminal.
        if let Some(existing_ids) = self.pending_existing_terminals.take() {
            if !existing_ids.is_empty() {
                self.terminal_views.clear();

                let (columns, rows, cell_width, line_height) = compute_terminal_dimensions(window);
                for id in &existing_ids {
                    let terminal_view =
                        cx.new(|cx| TerminalView::new(columns, rows, cell_width, line_height, cx));
                    terminal_view.update(cx, |view, _cx| {
                        view.set_keyboard_request(crate::keyboard::make_keyboard_handler());
                        view.set_is_keyboard_visible_fn(
                            crate::keyboard::make_is_keyboard_visible(),
                        );
                        view.set_terminal_id(id.clone());
                        view.set_connected(true);
                        view.set_status("Resumed".to_string());
                    });
                    self.terminal_views.push((id.clone(), terminal_view));
                }

                // Show the first terminal and mark it active
                if let Some((first_id, first_view)) = self.terminal_views.first() {
                    let first_id = first_id.clone();
                    let first_view = first_view.clone();
                    self.workspace_content.update(cx, |content, cx| {
                        content.set_main_view(first_view.into(), "Terminal", cx);
                    });
                    self.active_terminal_id = Some(first_id.clone());
                    if let Some(session) = zedra_session::active_session() {
                        session.set_active_terminal(&first_id);
                    }
                    self.workspace_drawer.update(cx, |drawer, cx| {
                        drawer.set_active_terminal(Some(first_id), cx);
                    });
                }
            }
        }

        // Drain pending file content → swap main view to EditorView
        if let Some((filename, content)) = self.pending_file.take() {
            let editor_view = cx.new(|cx| EditorView::new(content, cx));
            let fname = filename.clone();
            self.workspace_content.update(cx, |c, cx| {
                c.set_main_view(editor_view.into(), fname, cx);
            });
        }

        // Drain pending git diff → swap main view to GitDiffView
        if let Some((path, filename, diff_text)) = self.pending_git_diff.take() {
            let diffs = crate::editor::git_diff_view::parse_unified_diff(&diff_text);
            let diff = diffs
                .into_iter()
                .find(|d| d.new_path == path)
                .unwrap_or_else(|| {
                    crate::editor::git_diff_view::parse_unified_diff(&diff_text)
                        .into_iter()
                        .next()
                        .unwrap_or(crate::editor::git_diff_view::FileDiff {
                            old_path: path.clone(),
                            new_path: path.clone(),
                            hunks: Vec::new(),
                        })
                });
            let diff_view = cx.new(|cx| GitDiffView::new(diff, path.clone(), cx));
            let title = format!("Diff: {}", filename);
            self.workspace_content.update(cx, |c, cx| {
                c.set_main_view(diff_view.into(), title, cx);
            });
        }

        // Drain pending terminal ID → assign to pending slot and mark connected
        if let Some(term_id) = self.pending_terminal_id.take() {
            if let Some(entry) = self
                .terminal_views
                .iter_mut()
                .rev()
                .find(|(id, _)| id == "__pending__")
            {
                entry.0 = term_id.clone();
                entry.1.update(cx, |view, _cx| {
                    view.set_terminal_id(term_id.clone());
                    view.set_connected(true);
                    view.set_status("Connected".to_string());
                });
            }
            self.active_terminal_id = Some(term_id.clone());
            if let Some(session) = zedra_session::active_session() {
                session.set_active_terminal(&term_id);
            }
            self.workspace_drawer.update(cx, |drawer, cx| {
                drawer.set_active_terminal(Some(term_id), cx);
            });
        }

        div().size_full().child(self.drawer_host.clone())
    }
}

/// Compute terminal grid dimensions from the current viewport.
/// Returns `(columns, rows, cell_width, line_height)`.
pub fn compute_terminal_dimensions(window: &mut Window) -> (usize, usize, Pixels, Pixels) {
    let viewport = window.viewport_size();
    let line_height = px(16.0);

    zedra_terminal::load_terminal_font(window);

    let font = gpui::Font {
        family: zedra_terminal::TERMINAL_FONT_FAMILY.into(),
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
    let rows = 24;

    (columns, rows, cell_width, line_height)
}
