// Root application view for Zedra
// Screen-based navigation: Home → Editor
// Drawer overlays full screen, bottom nav bar switches drawer tab content.

use std::sync::Arc;

use gpui::*;

use crate::app_drawer::{AppDrawer, AppDrawerEvent};
use crate::editor::code_editor::EditorView;
use crate::editor::git_diff_view::GitDiffView;
use crate::home_view::{HomeEvent, HomeView};
use crate::mgpui::{DrawerHost, HeaderConfig, StackNavigator};
use crate::theme;
use zedra_session::RemoteSession;
use zedra_terminal::view::{DisconnectRequested, TerminalView};

// ---------------------------------------------------------------------------
// AppScreen — which screen is currently displayed
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
enum AppScreen {
    Home,
    Editor,
}

// ---------------------------------------------------------------------------
// EditorContent — header + separator + stack (rendered inside DrawerHost)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub enum EditorContentEvent {
    ToggleDrawer,
}

pub struct EditorContent {
    editor_stack: Entity<StackNavigator>,
}

impl EventEmitter<EditorContentEvent> for EditorContent {}

impl EditorContent {
    pub fn new(editor_stack: Entity<StackNavigator>, _cx: &mut Context<Self>) -> Self {
        Self { editor_stack }
    }
}

impl Render for EditorContent {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = self
            .editor_stack
            .read(cx)
            .current_title()
            .cloned()
            .unwrap_or_default();

        let stack_depth = self.editor_stack.read(cx).stack_depth();

        // Status bar inset (applied locally so backdrop stays full-screen)
        let top_inset = crate::platform_bridge::status_bar_inset();

        // Adaptive header: root shows logo+title, pushed views show back+title
        let header = if stack_depth > 1 {
            // Pushed view: "< Back" button + title
            div()
                .h(px(48.0))
                .flex()
                .flex_row()
                .items_center()
                .px(px(16.0))
                .border_b_1()
                .border_color(rgb(theme::BORDER_SUBTLE))
                .child(
                    div()
                        .id("back-btn")
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(4.0))
                        .cursor_pointer()
                        .hover(|s| s.bg(theme::hover_bg()).rounded(px(4.0)))
                        .px(px(4.0))
                        .py(px(4.0))
                        .rounded(px(4.0))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _event, _window, cx| {
                                this.editor_stack.update(cx, |s, cx| {
                                    s.pop(cx);
                                });
                            }),
                        )
                        .child(
                            div()
                                .text_color(rgb(theme::TEXT_SECONDARY))
                                .text_size(px(theme::FONT_BODY))
                                .child("\u{2039} Back"),
                        ),
                )
                .child(
                    div().ml_3().flex_1().child(
                        div()
                            .text_color(rgb(theme::TEXT_SECONDARY))
                            .text_size(px(theme::FONT_BODY))
                            .font_weight(FontWeight::MEDIUM)
                            .child(title),
                    ),
                )
        } else {
            // Root view: logo button + title
            div()
                .h(px(48.0))
                .flex()
                .flex_row()
                .items_center()
                .px(px(16.0))
                .border_b_1()
                .border_color(rgb(theme::BORDER_SUBTLE))
                .child(
                    div()
                        .id("logo-btn")
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
                                cx.emit(EditorContentEvent::ToggleDrawer);
                            }),
                        )
                        .child(
                            svg()
                                .path("icons/logo.svg")
                                .size(px(theme::ICON_HEADER))
                                .text_color(rgb(theme::TEXT_PRIMARY)),
                        ),
                )
                .child(
                    div().ml_3().flex_1().child(
                        div()
                            .text_color(rgb(theme::TEXT_SECONDARY))
                            .text_size(px(theme::FONT_BODY))
                            .font_weight(FontWeight::MEDIUM)
                            .child(title),
                    ),
                )
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(theme::BG_PRIMARY))
            // Status bar inset spacer
            .child(div().h(px(top_inset)))
            // Header (48px, matches drawer header)
            .child(header)
            // Main content (stack navigator)
            .child(div().flex_1().child(self.editor_stack.clone()))
    }
}

// ---------------------------------------------------------------------------
// FileLoadingView — placeholder when a file is loading or unavailable
// ---------------------------------------------------------------------------

struct FileLoadingView {
    message: SharedString,
    focus_handle: FocusHandle,
}

impl FileLoadingView {
    fn new(message: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        Self {
            message: message.into(),
            focus_handle: cx.focus_handle(),
        }
    }
}

impl Focusable for FileLoadingView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for FileLoadingView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .text_color(rgb(theme::TEXT_MUTED))
                    .text_size(px(theme::FONT_BODY))
                    .child(self.message.clone()),
            )
    }
}

// ---------------------------------------------------------------------------
// ZedraApp — screen-based navigation (Home → Editor)
// ---------------------------------------------------------------------------

pub struct ZedraApp {
    screen: AppScreen,
    home_view: Entity<HomeView>,
    drawer_host: Entity<DrawerHost>,
    editor_stack: Entity<StackNavigator>,
    app_drawer: Entity<AppDrawer>,
    /// (terminal_id, view entity) pairs in creation order.
    terminal_views: Vec<(String, Entity<TerminalView>)>,
    active_terminal_id: Option<String>,
    session: Option<Arc<RemoteSession>>,
    editor_showing_project: bool,
    render_count: u64,
    _subscriptions: Vec<Subscription>,
}

impl ZedraApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Load JetBrains Mono font for all UI text
        zedra_terminal::load_terminal_font(window);

        let mut subscriptions = Vec::new();

        // --- Home view ---
        let home_view = cx.new(|cx| HomeView::new(cx));
        let sub = cx.subscribe_in(
            &home_view,
            window,
            |_this: &mut Self, _emitter, event: &HomeEvent, _window, _cx| match event {
                HomeEvent::ConnectTapped | HomeEvent::ScanQrTapped => {
                    log::info!("Home: Scan QR tapped");
                    crate::platform_bridge::bridge().launch_qr_scanner();
                }
            },
        );
        subscriptions.push(sub);

        // --- Editor stack ---
        let editor_stack = cx.new(|cx| {
            let mut stack = StackNavigator::new(
                HeaderConfig {
                    show_header: false,
                    ..Default::default()
                },
                cx,
            );
            let placeholder = cx.new(|cx| FileLoadingView::new("No active session", cx));
            stack.push(placeholder.into(), "Zedra", cx);
            stack
        });

        // --- EditorContent (header + stack) ---
        let editor_stack_for_content = editor_stack.clone();
        let editor_content = cx.new(|cx| EditorContent::new(editor_stack_for_content, cx));

        // --- DrawerHost wrapping EditorContent ---
        let drawer_host = cx.new(|cx| DrawerHost::new(editor_content.clone().into(), cx));

        // Subscribe to EditorContent toggle-drawer events
        let drawer_host_for_toggle = drawer_host.clone();
        let sub = cx.subscribe_in(
            &editor_content,
            window,
            move |_this: &mut Self, _emitter, event: &EditorContentEvent, _window, cx| match event {
                EditorContentEvent::ToggleDrawer => {
                    if drawer_host_for_toggle.read(cx).is_open() {
                        drawer_host_for_toggle.update(cx, |host, cx| host.close(cx));
                    } else {
                        drawer_host_for_toggle.update(cx, |host, cx| host.open(cx));
                    }
                }
            },
        );
        subscriptions.push(sub);

        // --- Pre-create AppDrawer and register with DrawerHost ---
        let app_drawer = cx.new(|cx| AppDrawer::new(cx));
        drawer_host.update(cx, |host, _cx| {
            host.set_drawer(app_drawer.clone().into());
        });

        // Subscribe to AppDrawer events
        let drawer_host_for_sub = drawer_host.clone();
        let editor_stack_for_sub = editor_stack.clone();
        let app_drawer_for_sub = app_drawer.clone();
        let sub = cx.subscribe_in(
            &app_drawer,
            window,
            move |this: &mut ZedraApp,
                  _emitter: &Entity<AppDrawer>,
                  event: &AppDrawerEvent,
                  _window: &mut Window,
                  cx: &mut Context<ZedraApp>| {
                match event {
                    AppDrawerEvent::CloseRequested => {
                        drawer_host_for_sub.update(cx, |host, cx| host.close(cx));
                    }
                    AppDrawerEvent::DisconnectRequested => {
                        log::info!("Disconnect requested from Session tab");
                        drawer_host_for_sub.update(cx, |host, cx| host.close(cx));
                        zedra_session::clear_active_session();
                        this.session = None;
                        this.terminal_views.clear();
                        this.active_terminal_id = None;
                        this.editor_showing_project = false;
                        this.screen = AppScreen::Home;
                        app_drawer_for_sub.update(cx, |drawer, cx| {
                            drawer.reset_for_disconnect(cx);
                        });
                        let placeholder =
                            cx.new(|cx| FileLoadingView::new("No active session", cx));
                        editor_stack_for_sub.update(cx, |stack, cx| {
                            stack.replace(placeholder.into(), "Zedra", cx);
                        });
                        cx.notify();
                    }
                    AppDrawerEvent::NewTerminalRequested => {
                        log::info!("New terminal requested from drawer");
                        drawer_host_for_sub.update(cx, |host, cx| host.close(cx));
                        if let Some(session) = zedra_session::active_session() {
                            let (columns, rows, cell_width, line_height) =
                                compute_terminal_dimensions(_window);
                            let cols_u16 = columns as u16;
                            let rows_u16 = rows as u16;

                            let terminal_view = cx.new(|cx| {
                                TerminalView::new(columns, rows, cell_width, line_height, cx)
                            });
                            terminal_view.update(cx, |view, _cx| {
                                view.set_keyboard_request(crate::keyboard::make_keyboard_handler());
                                view.set_status("Creating terminal...".to_string());
                            });
                            editor_stack_for_sub.update(cx, |stack, cx| {
                                stack.replace(terminal_view.clone().into(), "Terminal", cx);
                            });

                            // Store as pending (no ID yet), will be registered on callback
                            this.terminal_views
                                .push(("__pending__".to_string(), terminal_view));

                            zedra_session::session_runtime().spawn(async move {
                                match session.terminal_create(cols_u16, rows_u16).await {
                                    Ok(term_id) => {
                                        log::info!("New terminal created: {}", term_id);
                                        PENDING_TERMINAL_ID.set(term_id);
                                        zedra_session::signal_terminal_data();
                                    }
                                    Err(e) => {
                                        log::error!("Failed to create new terminal: {}", e);
                                    }
                                }
                            });
                        }
                        cx.notify();
                    }
                    AppDrawerEvent::TerminalSelected(tid) => {
                        log::info!("Terminal selected from drawer: {}", tid);
                        drawer_host_for_sub.update(cx, |host, cx| host.close(cx));
                        let tid = tid.clone();
                        if let Some((_id, view)) =
                            this.terminal_views.iter().find(|(id, _)| id == &tid)
                        {
                            editor_stack_for_sub.update(cx, |stack, cx| {
                                stack.replace(view.clone().into(), "Terminal", cx);
                            });
                            this.active_terminal_id = Some(tid.clone());
                            if let Some(session) = zedra_session::active_session() {
                                session.set_active_terminal(&tid);
                            }
                            app_drawer_for_sub.update(cx, |drawer, cx| {
                                drawer.set_active_terminal(Some(tid), cx);
                            });
                        }
                        cx.notify();
                    }
                    AppDrawerEvent::FileSelected(path) => {
                        drawer_host_for_sub.update(cx, |host, cx| host.close(cx));
                        if !path.is_empty() {
                            log::info!("File selected from drawer: {}", path);
                            let path = path.clone();
                            let filename = path.rsplit('/').next().unwrap_or(&path).to_string();

                            if let Some(session) = zedra_session::active_session() {
                                // Remote file — push loading placeholder, then swap
                                let loading_view =
                                    cx.new(|cx| FileLoadingView::new("Loading\u{2026}", cx));
                                let fname = filename.clone();
                                editor_stack_for_sub.update(cx, |stack, cx| {
                                    stack.push(loading_view.into(), &fname, cx);
                                });
                                let filename_clone = filename.clone();
                                zedra_session::session_runtime().spawn(async move {
                                    match session.fs_read(&path).await {
                                        Ok(content) => {
                                            PENDING_FILE_CONTENT.set((filename_clone, content));
                                            zedra_session::signal_terminal_data();
                                        }
                                        Err(e) => {
                                            log::error!("fs/read failed for {}: {}", path, e);
                                        }
                                    }
                                });
                            } else {
                                // No session, not in samples — show placeholder
                                let placeholder =
                                    cx.new(|cx| FileLoadingView::new("No preview available", cx));
                                let fname = filename.clone();
                                editor_stack_for_sub.update(cx, |stack, cx| {
                                    stack.push(placeholder.into(), &fname, cx);
                                });
                            }
                        }
                    }
                    AppDrawerEvent::GitFileSelected(path) => {
                        drawer_host_for_sub.update(cx, |host, cx| host.close(cx));
                        log::info!("Git file selected: {}", path);
                        let path = path.clone();
                        let filename = path.rsplit('/').next().unwrap_or(&path).to_string();

                        if let Some(session) = zedra_session::active_session() {
                            let path_clone = path.clone();
                            let filename_clone = filename.clone();
                            zedra_session::session_runtime().spawn(async move {
                                match session.git_diff(Some(&path_clone), false).await {
                                    Ok(diff_text) => {
                                        PENDING_GIT_DIFF.set((path_clone, filename_clone, diff_text));
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
                            // Fallback to sample data when no session
                            let diffs = crate::editor::git_diff_view::sample_diffs();
                            if let Some(diff) = diffs.into_iter().find(|d| d.new_path == path) {
                                let diff_view =
                                    cx.new(|cx| GitDiffView::new(diff, path.clone(), cx));
                                editor_stack_for_sub.update(cx, |stack, cx| {
                                    stack.push(
                                        diff_view.into(),
                                        &format!("Diff: {}", filename),
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

        Self {
            screen: AppScreen::Home,
            home_view,
            drawer_host,
            editor_stack,
            app_drawer,
            terminal_views: Vec::new(),
            active_terminal_id: None,
            session: None,
            editor_showing_project: false,
            render_count: 0,
            _subscriptions: subscriptions,
        }
    }

    /// Create a terminal view with proper viewport-based dimensions and wire up
    /// disconnect handling. Returns (cols, rows) for the remote terminal.
    fn create_terminal_view(
        &mut self,
        hostname: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (u16, u16) {
        let (columns, rows, cell_width, line_height) = compute_terminal_dimensions(window);

        let terminal_view =
            cx.new(|cx| TerminalView::new(columns, rows, cell_width, line_height, cx));

        terminal_view.update(cx, |view, _cx| {
            view.set_keyboard_request(crate::keyboard::make_keyboard_handler());
        });

        let disconnect_sub = cx.subscribe(
            &terminal_view,
            |this, _terminal, _event: &DisconnectRequested, cx| {
                log::info!("DisconnectRequested received, returning to Home");
                zedra_session::clear_active_session();
                this.session = None;
                this.terminal_views.clear();
                this.active_terminal_id = None;
                this.editor_showing_project = false;
                this.screen = AppScreen::Home;
                this.app_drawer.update(cx, |drawer, cx| {
                    drawer.reset_for_disconnect(cx);
                });
                let placeholder = cx.new(|cx| FileLoadingView::new("No active session", cx));
                this.editor_stack.update(cx, |stack, cx| {
                    stack.replace(placeholder.into(), "Zedra", cx);
                });
                cx.notify();
            },
        );
        self._subscriptions.push(disconnect_sub);

        self.editor_stack.update(cx, |stack, cx| {
            stack.replace(terminal_view.clone().into(), "Terminal", cx);
        });

        terminal_view.update(cx, |view, _cx| {
            view.set_status(format!("Connecting to {}...", hostname));
        });

        // Store with placeholder ID — will be updated when terminal_create returns
        self.terminal_views
            .push(("__pending__".to_string(), terminal_view.clone()));

        (columns as u16, rows as u16)
    }

    fn connect_with_iroh_addr(
        &mut self,
        addr: iroh::EndpointAddr,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let endpoint_short = addr.id.fmt_short().to_string();
        log::info!("QR connect: starting iroh connection to {}", endpoint_short);

        self.screen = AppScreen::Editor;

        let (cols, rows) = self.create_terminal_view(&endpoint_short, window, cx);

        zedra_session::session_runtime().spawn(async move {
            log::info!("RemoteSession: connecting via iroh to {}...", endpoint_short);
            match RemoteSession::connect_with_iroh(addr).await {
                Ok(session) => {
                    log::info!("RemoteSession: connected via iroh!");
                    match session.terminal_create(cols, rows).await {
                        Ok(term_id) => {
                            log::info!("Remote terminal created: {}", term_id);
                            PENDING_TERMINAL_ID.set(term_id);
                        }
                        Err(e) => log::error!("Failed to create remote terminal: {}", e),
                    }
                    zedra_session::set_active_session(session);
                    zedra_session::signal_terminal_data();
                }
                Err(e) => {
                    log::error!("RemoteSession iroh connect failed: {}", e);
                }
            }
        });
    }
}

impl Render for ZedraApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_count += 1;
        if self.render_count % 60 == 1 {
            log::warn!(
                "ZedraApp::render #{}, screen={:?}",
                self.render_count,
                self.screen
            );
        }

        // Check for pending remote file content (replaces loading placeholder)
        if self.screen == AppScreen::Editor && !self.editor_showing_project {
            if let Some((filename, content)) = PENDING_FILE_CONTENT.take() {
                let editor_view = cx.new(|cx| EditorView::new(content, cx));
                let fname = filename.clone();
                self.editor_stack.update(cx, |stack, cx| {
                    stack.replace(editor_view.into(), &fname, cx);
                });
            }
        }

        // Check for pending git diff from async RPC
        if self.screen == AppScreen::Editor {
            if let Some((path, filename, diff_text)) = PENDING_GIT_DIFF.take() {
                let diffs = crate::editor::git_diff_view::parse_unified_diff(&diff_text);
                let diff = diffs
                    .into_iter()
                    .find(|d| d.new_path == path)
                    .unwrap_or_else(|| {
                        // If no matching path found, use the first diff or create empty
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
                self.editor_stack.update(cx, |stack, cx| {
                    stack.push(diff_view.into(), &format!("Diff: {}", filename), cx);
                });
            }
        }

        // Check for pending terminal ID from async terminal_create
        if let Some(term_id) = PENDING_TERMINAL_ID.take() {
            // Find the most recent pending terminal view and assign the ID
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
            self.app_drawer.update(cx, |drawer, cx| {
                drawer.set_active_terminal(Some(term_id), cx);
            });
        }

        // Check for QR-scanned endpoint address
        if let Some(addr) = PENDING_QR_ADDR.take() {
            self.connect_with_iroh_addr(addr, window, cx);
        }

        let screen_content: AnyElement = match self.screen {
            AppScreen::Home => div()
                .size_full()
                .child(self.home_view.clone())
                .into_any_element(),
            AppScreen::Editor => {
                let mut root = div()
                    .size_full()
                    .bg(rgb(theme::BG_PRIMARY))
                    .flex()
                    .flex_col()
                    // DrawerHost (contains EditorContent + drawer overlay)
                    .child(div().flex_1().child(self.drawer_host.clone()));

                // Transport badge (top-right, centered in 48px header)
                if let Some((label, dot_color)) =
                    zedra_session::active_session().map(|s| {
                        let latency = s.latency_ms();
                        let conn_info = s.connection_info();
                        crate::transport_badge::transport_badge_info(latency, conn_info.as_ref())
                    })
                {
                    root = root.child(
                        deferred(crate::transport_badge::render_transport_badge(
                            label, dot_color,
                        ))
                        .with_priority(100),
                    );
                }

                root.into_any_element()
            }
        };

        // Wrap in root div with JetBrains Mono font for all text
        div()
            .size_full()
            .font_family(zedra_terminal::TERMINAL_FONT_FAMILY)
            .child(screen_content)
    }
}

// ---------------------------------------------------------------------------
// Global pending state for async → main thread (via PendingSlot)
// ---------------------------------------------------------------------------

use crate::pending::PendingSlot;

static PENDING_FILE_CONTENT: PendingSlot<(String, String)> = PendingSlot::new();
static PENDING_TERMINAL_ID: PendingSlot<String> = PendingSlot::new();
static PENDING_GIT_DIFF: PendingSlot<(String, String, String)> = PendingSlot::new();
static PENDING_QR_ADDR: PendingSlot<iroh::EndpointAddr> = PendingSlot::new();

pub fn set_pending_qr_addr(addr: iroh::EndpointAddr) {
    PENDING_QR_ADDR.set(addr);
}

/// Compute terminal grid dimensions from the current viewport.
/// Returns `(columns, rows, cell_width, line_height)`.
fn compute_terminal_dimensions(window: &mut Window) -> (usize, usize, Pixels, Pixels) {
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

    let top_inset_px = crate::platform_bridge::status_bar_inset();
    let available_height = viewport.height - px(top_inset_px + 48.0);

    let columns = ((viewport.width / cell_width).floor() as usize)
        .saturating_sub(1)
        .clamp(20, 200);
    let rows = ((available_height / line_height).floor() as usize).clamp(5, 100);

    (columns, rows, cell_width, line_height)
}

