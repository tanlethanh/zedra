// Root application view for Zedra
// Screen-based navigation: Home → Editor
// Drawer overlays full screen, bottom nav bar switches drawer tab content.

use std::sync::Arc;

use gpui::prelude::FluentBuilder;
use gpui::*;

use crate::app_drawer::{AppDrawer, AppDrawerEvent};
use crate::code_editor::EditorView;
use crate::git_diff_view::GitDiffView;
use crate::home_view::{HomeEvent, HomeView};
use crate::project_editor::ProjectEditor;
use crate::theme;
use zedra_nav::{DrawerHost, HeaderConfig, StackNavigator};
use zedra_session::RemoteSession;
use zedra_terminal::view::{DisconnectRequested, TerminalView};
use zedra_transport::PairingPayload;

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

pub struct EditorContent {
    editor_stack: Entity<StackNavigator>,
    drawer_host: Entity<DrawerHost>,
}

impl EditorContent {
    pub fn new(
        editor_stack: Entity<StackNavigator>,
        drawer_host: Entity<DrawerHost>,
        _cx: &mut Context<Self>,
    ) -> Self {
        Self {
            editor_stack,
            drawer_host,
        }
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
        let density = crate::android_jni::get_density();
        let top_inset = if density > 0.0 {
            crate::android_jni::get_system_inset_top() as f32 / density
        } else {
            0.0
        };

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
                            cx.listener(|this, _event, _window, cx| {
                                if this.drawer_host.read(cx).is_open() {
                                    this.drawer_host.update(cx, |host, cx| host.close(cx));
                                } else {
                                    this.drawer_host.update(cx, |host, cx| host.open(cx));
                                }
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
    editor_content: Entity<EditorContent>,
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
                    crate::android_jni::launch_qr_scanner();
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

        // --- DrawerHost (initially wraps editor_stack, we'll replace content below) ---
        let editor_stack_clone = editor_stack.clone();
        let drawer_host = cx.new(|cx| DrawerHost::new(editor_stack_clone.into(), cx));

        // --- EditorContent (header + separator + stack) ---
        let drawer_host_for_content = drawer_host.clone();
        let editor_stack_for_content = editor_stack.clone();
        let editor_content =
            cx.new(|cx| EditorContent::new(editor_stack_for_content, drawer_host_for_content, cx));

        // Update DrawerHost to wrap EditorContent (so overlay covers header)
        drawer_host.update(cx, |host, _cx| {
            host.set_content(editor_content.clone().into());
        });

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
                            // Create the terminal view on the main thread
                            let viewport = _window.viewport_size();
                            let line_height = px(16.0);
                            zedra_terminal::load_terminal_font(_window);
                            let font = gpui::Font {
                                family: zedra_terminal::TERMINAL_FONT_FAMILY.into(),
                                features: gpui::FontFeatures::default(),
                                fallbacks: None,
                                weight: gpui::FontWeight::NORMAL,
                                style: gpui::FontStyle::Normal,
                            };
                            let font_size = line_height * 0.75;
                            let text_system = _window.text_system();
                            let font_id = text_system.resolve_font(&font);
                            let cell_width = text_system
                                .advance(font_id, font_size, 'm')
                                .map(|size| size.width)
                                .unwrap_or(px(9.0));
                            let density = crate::android_jni::get_density();
                            let top_inset_px = if density > 0.0 {
                                crate::android_jni::get_system_inset_top() as f32 / density
                            } else {
                                0.0
                            };
                            let available_height = viewport.height - px(top_inset_px + 48.0);
                            let columns = ((viewport.width / cell_width).floor() as usize)
                                .saturating_sub(1)
                                .clamp(20, 200);
                            let rows = (available_height / line_height).floor() as usize;
                            let rows = rows.clamp(5, 100);
                            let cols_u16 = columns as u16;
                            let rows_u16 = rows as u16;

                            let terminal_view = cx.new(|cx| {
                                TerminalView::new(columns, rows, cell_width, line_height, cx)
                            });
                            terminal_view.update(cx, |view, _cx| {
                                view.set_keyboard_request(Box::new(|show| {
                                    if show && zedra_nav::is_drawer_overlay_visible() {
                                        return;
                                    }
                                    if show {
                                        crate::android_jni::show_keyboard();
                                    } else {
                                        crate::android_jni::hide_keyboard();
                                    }
                                }));
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
                                        set_pending_terminal_id(term_id);
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
                                            set_pending_file_content(filename_clone, content);
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
                                        set_pending_git_diff(path_clone, filename_clone, diff_text);
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
                            let diffs = crate::git_stack::GitStack::sample_diffs_public();
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
            editor_content,
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

        let available_width = viewport.width;
        // Vertical overhead: status bar inset + header (48px)
        let density = crate::android_jni::get_density();
        let top_inset_px = if density > 0.0 {
            crate::android_jni::get_system_inset_top() as f32 / density
        } else {
            0.0
        };
        let available_height = viewport.height - px(top_inset_px + 48.0);

        let columns = ((available_width / cell_width).floor() as usize).saturating_sub(1);
        let rows = (available_height / line_height).floor() as usize;
        let columns = columns.clamp(20, 200);
        let rows = rows.clamp(5, 100);

        let terminal_view =
            cx.new(|cx| TerminalView::new(columns, rows, cell_width, line_height, cx));

        terminal_view.update(cx, |view, _cx| {
            view.set_keyboard_request(Box::new(|show| {
                if show && zedra_nav::is_drawer_overlay_visible() {
                    return; // Don't show keyboard when drawer overlay is active
                }
                if show {
                    crate::android_jni::show_keyboard();
                } else {
                    crate::android_jni::hide_keyboard();
                }
            }));
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

    fn connect_with_iroh_payload(
        &mut self,
        payload: PairingPayload,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let hostname = payload.name.clone();
        log::info!("QR connect: starting iroh connection to {}", hostname);

        self.screen = AppScreen::Editor;

        let (cols, rows) = self.create_terminal_view(&hostname, window, cx);

        zedra_session::session_runtime().spawn(async move {
            log::info!("RemoteSession: connecting via iroh to {}...", hostname);
            match RemoteSession::connect_with_iroh(payload).await {
                Ok(session) => {
                    log::info!("RemoteSession: connected via iroh!");
                    match session.terminal_create(cols, rows).await {
                        Ok(term_id) => {
                            log::info!("Remote terminal created: {}", term_id);
                            set_pending_terminal_id(term_id);
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

        // Swap editor stack to ProjectEditor when session becomes active,
        // but only if no terminal view is showing (terminal takes priority)
        if self.screen == AppScreen::Editor
            && zedra_session::active_session().is_some()
            && !self.editor_showing_project
            && self.terminal_views.is_empty()
        {
            let project_editor = cx.new(|cx| ProjectEditor::new(cx));
            self.editor_stack.update(cx, |stack, cx| {
                stack.replace(project_editor.into(), "Project", cx);
            });
            self.editor_showing_project = true;
        }

        // Check for pending remote file content (replaces loading placeholder)
        if self.screen == AppScreen::Editor && !self.editor_showing_project {
            if let Some((filename, content)) = take_pending_file_content() {
                let editor_view = cx.new(|cx| EditorView::new(content, cx));
                let fname = filename.clone();
                self.editor_stack.update(cx, |stack, cx| {
                    stack.replace(editor_view.into(), &fname, cx);
                });
            }
        }

        // Check for pending git diff from async RPC
        if self.screen == AppScreen::Editor {
            if let Some((path, filename, diff_text)) = take_pending_git_diff() {
                let diffs = crate::diff_view::parse_unified_diff(&diff_text);
                let diff = diffs
                    .into_iter()
                    .find(|d| d.new_path == path)
                    .unwrap_or_else(|| {
                        // If no matching path found, use the first diff or create empty
                        crate::diff_view::parse_unified_diff(&diff_text)
                            .into_iter()
                            .next()
                            .unwrap_or(crate::diff_view::FileDiff {
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
        if let Some(term_id) = take_pending_terminal_id() {
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

        // Check for QR-scanned pairing payload
        if let Some(payload) = take_pending_qr_payload() {
            self.connect_with_iroh_payload(payload, window, cx);
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
                let transport_badge = zedra_session::active_session().map(|s| {
                    let latency = s.latency_ms();
                    transport_badge_info(latency)
                });

                if let Some((label, dot_color)) = transport_badge {
                    // Vertically center badge in the 48px header below the status bar inset.
                    // Badge height ≈ 18px (10px text + 2×2px py + line padding).
                    let density = crate::android_jni::get_density();
                    let top_inset = if density > 0.0 {
                        crate::android_jni::get_system_inset_top() as f32 / density
                    } else {
                        0.0
                    };
                    let badge_top = top_inset + (48.0 - 18.0) / 2.0;

                    root = root.child(
                        deferred(
                            div()
                                .absolute()
                                .top(px(badge_top))
                                .right(px(8.0))
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(4.0))
                                .px_2()
                                .py(px(2.0))
                                .rounded(px(4.0))
                                .bg(theme::badge_bg())
                                .child(
                                    div()
                                        .w(px(theme::ICON_STATUS))
                                        .h(px(theme::ICON_STATUS))
                                        .rounded(px(3.0))
                                        .bg(rgb(dot_color)),
                                )
                                .child(
                                    div()
                                        .text_size(px(theme::FONT_DETAIL))
                                        .text_color(rgb(dot_color))
                                        .child(label),
                                ),
                        )
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
// Global pending state for async file content → main thread
// ---------------------------------------------------------------------------

use std::sync::Mutex;

static PENDING_FILE_CONTENT: Mutex<Option<(String, String)>> = Mutex::new(None);
/// Pending terminal ID from async terminal_create → main thread
static PENDING_TERMINAL_ID: Mutex<Option<String>> = Mutex::new(None);
/// (path, filename, diff_text) for async git diff → main thread
static PENDING_GIT_DIFF: Mutex<Option<(String, String, String)>> = Mutex::new(None);
static PENDING_QR_PAYLOAD: Mutex<Option<PairingPayload>> = Mutex::new(None);

pub fn set_pending_qr_payload(payload: PairingPayload) {
    if let Ok(mut slot) = PENDING_QR_PAYLOAD.lock() {
        *slot = Some(payload);
    }
}

fn take_pending_qr_payload() -> Option<PairingPayload> {
    if let Ok(mut slot) = PENDING_QR_PAYLOAD.lock() {
        slot.take()
    } else {
        None
    }
}

fn set_pending_terminal_id(id: String) {
    if let Ok(mut slot) = PENDING_TERMINAL_ID.lock() {
        *slot = Some(id);
    }
}

fn take_pending_terminal_id() -> Option<String> {
    if let Ok(mut slot) = PENDING_TERMINAL_ID.lock() {
        slot.take()
    } else {
        None
    }
}

fn set_pending_file_content(filename: String, content: String) {
    if let Ok(mut slot) = PENDING_FILE_CONTENT.lock() {
        *slot = Some((filename, content));
    }
}

fn take_pending_file_content() -> Option<(String, String)> {
    if let Ok(mut slot) = PENDING_FILE_CONTENT.lock() {
        slot.take()
    } else {
        None
    }
}

fn set_pending_git_diff(path: String, filename: String, diff_text: String) {
    if let Ok(mut slot) = PENDING_GIT_DIFF.lock() {
        *slot = Some((path, filename, diff_text));
    }
}

fn take_pending_git_diff() -> Option<(String, String, String)> {
    if let Ok(mut slot) = PENDING_GIT_DIFF.lock() {
        slot.take()
    } else {
        None
    }
}

pub(crate) fn transport_badge_info(latency_ms: u64) -> (String, u32) {
    let label = if latency_ms > 0 {
        format!("Connected \u{00b7} {}ms", latency_ms)
    } else {
        "Connected".to_string()
    };
    (label, theme::ACCENT_GREEN)
}
