// Root application view for Zedra
// Screen-based navigation: Home → Editor
// Drawer overlays full screen, bottom nav bar switches drawer tab content.

use std::sync::Arc;

use gpui::prelude::FluentBuilder;
use gpui::*;

use crate::app_drawer::{AppDrawer, AppDrawerEvent};
use crate::file_preview_list::{FilePreviewList, PreviewSelected, SAMPLE_FILES};
use crate::input::{Input, InputChanged};
use crate::project_editor::ProjectEditor;
use crate::home_view::{HomeEvent, HomeView};
use crate::theme;
use zedra_editor::{EditorView, GitDiffView};
use zedra_nav::{DrawerHost, HeaderConfig, StackNavigator};
use zedra_session::RemoteSession;
use zedra_terminal::view::{DisconnectRequested, TerminalView};
use zedra_transport::{PeerInfo, TransportState};

// ---------------------------------------------------------------------------
// ConnectView — connection form (shown as modal overlay)
// ---------------------------------------------------------------------------

pub struct ConnectView {
    host_input: Entity<Input>,
    port_input: Entity<Input>,
    _subscriptions: Vec<Subscription>,
}

impl ConnectView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let host_input = cx.new(|cx| Input::new(cx).placeholder("127.0.0.1"));
        let port_input = cx.new(|cx| Input::new(cx).placeholder("2123"));

        let mut subscriptions = Vec::new();

        let sub = cx.subscribe(
            &host_input,
            |_this: &mut Self, _input, event: &InputChanged, cx| {
                log::debug!("Host changed: {}", event.value);
                cx.notify();
            },
        );
        subscriptions.push(sub);

        let sub = cx.subscribe(
            &port_input,
            |_this: &mut Self, _input, event: &InputChanged, cx| {
                log::debug!("Port changed: {}", event.value);
                cx.notify();
            },
        );
        subscriptions.push(sub);

        Self {
            host_input,
            port_input,
            _subscriptions: subscriptions,
        }
    }

    fn get_connection_params(&self, cx: &App) -> (String, u16) {
        let host = self.host_input.read(cx).get_value().to_string();
        let host = if host.is_empty() {
            "127.0.0.1".to_string()
        } else {
            host
        };

        let port_str = self.port_input.read(cx).get_value();
        let port: u16 = port_str.parse().unwrap_or(2123);

        (host, port)
    }
}

#[derive(Clone, Debug)]
pub struct ConnectRequested {
    pub host: String,
    pub port: u16,
}

impl EventEmitter<ConnectRequested> for ConnectView {}

impl Render for ConnectView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .size_full()
            .bg(rgb(theme::BG_PRIMARY))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .p_6()
                    .w(px(300.0))
                    .bg(rgb(theme::BG_CARD))
                    .rounded(px(12.0))
                    .border_1()
                    .border_color(rgb(theme::BORDER_SUBTLE))
                    // Title
                    .child(
                        div()
                            .text_color(rgb(theme::TEXT_PRIMARY))
                            .text_xl()
                            .child("Connect"),
                    )
                    .child(
                        div()
                            .text_color(rgb(theme::TEXT_MUTED))
                            .text_sm()
                            .child("Connect to zedra-host daemon"),
                    )
                    // Host input
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_color(rgb(theme::TEXT_SECONDARY))
                                    .text_sm()
                                    .child("Host"),
                            )
                            .child(self.host_input.clone()),
                    )
                    // Port input
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_color(rgb(theme::TEXT_SECONDARY))
                                    .text_sm()
                                    .child("Port"),
                            )
                            .child(self.port_input.clone()),
                    )
                    // Connect button
                    .child(
                        div()
                            .mt_2()
                            .px_4()
                            .py_2()
                            .rounded(px(6.0))
                            .border_1()
                            .border_color(rgb(theme::BORDER_DEFAULT))
                            .text_color(rgb(theme::TEXT_PRIMARY))
                            .cursor_pointer()
                            .hover(|s| s.bg(theme::hover_bg()))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _event, _window, cx| {
                                    log::info!("Connect button pressed!");
                                    let (host, port) = this.get_connection_params(cx);
                                    log::info!("Connecting to {}:{}", host, port);
                                    cx.emit(ConnectRequested { host, port });
                                }),
                            )
                            .child(div().flex().justify_center().child("Connect")),
                    )
                    // Divider
                    .child(
                        div()
                            .mt_2()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(rgb(theme::TEXT_MUTED))
                            .text_sm()
                            .child("— or —"),
                    )
                    // Scan QR Code button
                    .child(
                        div()
                            .mt_2()
                            .px_4()
                            .py_2()
                            .rounded(px(6.0))
                            .border_1()
                            .border_color(rgb(theme::BORDER_DEFAULT))
                            .text_color(rgb(theme::TEXT_PRIMARY))
                            .cursor_pointer()
                            .hover(|s| s.bg(theme::hover_bg()))
                            .on_mouse_down(
                                MouseButton::Left,
                                |_event, _window, _cx| {
                                    log::info!("Scan QR Code button pressed!");
                                    crate::android_jni::launch_qr_scanner();
                                },
                            )
                            .child(div().flex().justify_center().child("Scan QR Code")),
                    ),
            )
    }
}

// ---------------------------------------------------------------------------
// AppScreen — which screen is currently displayed
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
enum AppScreen {
    Home,
    Connect,
    Editor,
}

// ---------------------------------------------------------------------------
// EditorContent — header + separator + stack (rendered inside DrawerHost)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct DisconnectEvent;

impl EventEmitter<DisconnectEvent> for EditorContent {}

pub struct EditorContent {
    editor_stack: Entity<StackNavigator>,
    drawer_host: Entity<DrawerHost>,
    terminal_view: Option<Entity<TerminalView>>,
}

impl EditorContent {
    pub fn new(
        editor_stack: Entity<StackNavigator>,
        drawer_host: Entity<DrawerHost>,
        terminal_view: Option<Entity<TerminalView>>,
        _cx: &mut Context<Self>,
    ) -> Self {
        Self {
            editor_stack,
            drawer_host,
            terminal_view,
        }
    }

    pub fn set_terminal_view(
        &mut self,
        tv: Option<Entity<TerminalView>>,
        cx: &mut Context<Self>,
    ) {
        self.terminal_view = tv;
        cx.notify();
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

        let (terminal_connected, terminal_status) = self
            .terminal_view
            .as_ref()
            .map(|tv| {
                let tv = tv.read(cx);
                (tv.is_connected(), tv.status_text().to_string())
            })
            .unwrap_or((false, String::new()));

        let has_terminal = self.terminal_view.is_some();

        // Status bar inset (applied locally so backdrop stays full-screen)
        let density = crate::android_jni::get_density();
        let top_inset = if density > 0.0 {
            crate::android_jni::get_system_inset_top() as f32 / density
        } else {
            0.0
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(theme::BG_PRIMARY))
            // Header (top_inset + 88px)
            .child(
                div()
                    .h(px(top_inset + 88.0))
                    .pt(px(top_inset))
                    .flex()
                    .flex_row()
                    .items_center()
                    .px(px(16.0))
                    .bg(rgb(theme::BG_PRIMARY))
                    .child(
                        // Logo button (opens/closes drawer)
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
                                        this.drawer_host
                                            .update(cx, |host, cx| host.close(cx));
                                    } else {
                                        this.drawer_host
                                            .update(cx, |host, cx| host.open(cx));
                                    }
                                }),
                            )
                            .child(
                                svg()
                                    .path("icons/logo.svg")
                                    .size(px(24.0))
                                    .text_color(rgb(theme::TEXT_PRIMARY)),
                            ),
                    )
                    .child(
                        // Title + connection status
                        div()
                            .ml_3()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_color(rgb(theme::TEXT_SECONDARY))
                                    .text_sm()
                                    .child(title),
                            )
                            .when(has_terminal && !terminal_connected, |el| {
                                el.child(
                                    div()
                                        .text_color(rgb(theme::TEXT_MUTED))
                                        .text_xs()
                                        .child(terminal_status.clone()),
                                )
                            }),
                    )
                    // Disconnect button (only when terminal is connected)
                    .when(terminal_connected, |el| {
                        el.child(
                            div()
                                .mr_2()
                                .px_2()
                                .py(px(4.0))
                                .rounded(px(4.0))
                                .border_1()
                                .border_color(rgb(theme::ACCENT_RED))
                                .text_color(rgb(theme::ACCENT_RED))
                                .text_xs()
                                .cursor_pointer()
                                .hover(|s| s.bg(gpui::hsla(0.0, 0.6, 0.5, 0.1)))
                                .child("Disconnect")
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|_this, _event, _window, cx| {
                                        cx.emit(DisconnectEvent);
                                    }),
                                ),
                        )
                    }),
            )
            // Separator
            .child(div().h(px(1.0)).bg(rgb(theme::BORDER_SUBTLE)))
            // Main content (stack navigator)
            .child(
                div()
                    .flex_1()
                    .child(self.editor_stack.clone()),
            )
    }
}

// ---------------------------------------------------------------------------
// ZedraApp — screen-based navigation (Home → Editor)
// ---------------------------------------------------------------------------

pub struct ZedraApp {
    screen: AppScreen,
    home_view: Entity<HomeView>,
    connect_view: Option<Entity<ConnectView>>,
    drawer_host: Entity<DrawerHost>,
    editor_stack: Entity<StackNavigator>,
    editor_content: Entity<EditorContent>,
    _app_drawer: Entity<AppDrawer>,
    terminal_view: Option<Entity<TerminalView>>,
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
            |this: &mut Self, _emitter, event: &HomeEvent, window, cx| {
                match event {
                    HomeEvent::ConnectTapped => {
                        log::info!("Home: Connect tapped");
                        this.show_connect_form(window, cx);
                    }
                    HomeEvent::ScanQrTapped => {
                        log::info!("Home: Scan QR tapped");
                        crate::android_jni::launch_qr_scanner();
                    }
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
            let preview_list = cx.new(|cx| FilePreviewList::new(cx));
            stack.push(preview_list.into(), "Code Samples", cx);
            stack
        });

        // --- PreviewSelected subscription ---
        let preview_list = cx.new(|cx| FilePreviewList::new(cx));
        let editor_stack_for_preview = editor_stack.clone();
        let sub = cx.subscribe_in(
            &preview_list,
            window,
            move |_this: &mut ZedraApp,
                  _emitter: &Entity<FilePreviewList>,
                  event: &PreviewSelected,
                  _window: &mut Window,
                  cx: &mut Context<ZedraApp>| {
                if let Some(sample) = SAMPLE_FILES.get(event.index) {
                    let editor_view = cx.new(|cx| EditorView::new(sample.content.to_string(), cx));
                    editor_stack_for_preview.update(cx, |stack, cx| {
                        stack.push(editor_view.into(), sample.filename, cx);
                    });
                }
            },
        );
        subscriptions.push(sub);
        editor_stack.update(cx, |stack, cx| {
            stack.replace(preview_list.into(), "Code Samples", cx);
        });

        // --- DrawerHost (initially wraps editor_stack, we'll replace content below) ---
        let editor_stack_clone = editor_stack.clone();
        let drawer_host = cx.new(|cx| {
            let mut host = DrawerHost::new(editor_stack_clone.into(), cx);
            host.set_edge_zone_width(px(180.0));
            host
        });

        // --- EditorContent (header + separator + stack) ---
        let drawer_host_for_content = drawer_host.clone();
        let editor_stack_for_content = editor_stack.clone();
        let editor_content = cx.new(|cx| {
            EditorContent::new(editor_stack_for_content, drawer_host_for_content, None, cx)
        });

        // Subscribe to disconnect events from EditorContent
        let sub = cx.subscribe(
            &editor_content,
            |this: &mut Self, _emitter, _event: &DisconnectEvent, cx| {
                log::info!("Disconnect requested from EditorContent header");
                zedra_session::clear_active_session();
                this.session = None;
                this.terminal_view = None;
                this.editor_content
                    .update(cx, |ec, cx| ec.set_terminal_view(None, cx));
                this.editor_showing_project = false;
                this.screen = AppScreen::Home;
                let preview = cx.new(|cx| FilePreviewList::new(cx));
                this.editor_stack.update(cx, |stack, cx| {
                    stack.replace(preview.into(), "Code Samples", cx);
                });
                cx.notify();
            },
        );
        subscriptions.push(sub);

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
        let sub = cx.subscribe_in(
            &app_drawer,
            window,
            move |_this: &mut ZedraApp,
                  _emitter: &Entity<AppDrawer>,
                  event: &AppDrawerEvent,
                  _window: &mut Window,
                  cx: &mut Context<ZedraApp>| {
                match event {
                    AppDrawerEvent::CloseRequested => {
                        drawer_host_for_sub.update(cx, |host, cx| host.close(cx));
                    }
                    AppDrawerEvent::FileSelected(path) => {
                        drawer_host_for_sub.update(cx, |host, cx| host.close(cx));
                        if !path.is_empty() {
                            log::info!("File selected from drawer: {}", path);
                            let path = path.clone();
                            let filename =
                                path.rsplit('/').next().unwrap_or(&path).to_string();

                            if let Some(session) = zedra_session::active_session() {
                                let filename_clone = filename.clone();
                                zedra_session::session_runtime().spawn(async move {
                                    match session.fs_read(&path).await {
                                        Ok(content) => {
                                            set_pending_file_content(
                                                filename_clone,
                                                content,
                                            );
                                            zedra_session::signal_terminal_data();
                                        }
                                        Err(e) => {
                                            log::error!(
                                                "fs/read failed for {}: {}",
                                                path,
                                                e
                                            );
                                        }
                                    }
                                });
                            } else if let Some(sample) =
                                SAMPLE_FILES.iter().find(|s| s.filename == filename)
                            {
                                let editor_view = cx.new(|cx| {
                                    EditorView::new(sample.content.to_string(), cx)
                                });
                                editor_stack_for_sub.update(cx, |stack, cx| {
                                    stack.push(editor_view.into(), sample.filename, cx);
                                });
                            }
                        }
                    }
                    AppDrawerEvent::GitFileSelected(path) => {
                        drawer_host_for_sub.update(cx, |host, cx| host.close(cx));
                        log::info!("Git file selected: {}", path);
                        // Find the diff for this file and push a GitDiffView
                        let diffs = zedra_editor::GitStack::sample_diffs_public();
                        if let Some(diff) = diffs.into_iter().find(|d| d.new_path == *path) {
                            let filename =
                                path.rsplit('/').next().unwrap_or(path).to_string();
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
            },
        );
        subscriptions.push(sub);

        Self {
            screen: AppScreen::Home,
            home_view,
            connect_view: None,
            drawer_host,
            editor_stack,
            editor_content,
            _app_drawer: app_drawer,
            terminal_view: None,
            session: None,
            editor_showing_project: false,
            render_count: 0,
            _subscriptions: subscriptions,
        }
    }

    fn show_connect_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.screen = AppScreen::Connect;
        let connect_view = cx.new(|cx| ConnectView::new(cx));
        let sub = cx.subscribe_in(
            &connect_view,
            window,
            |this: &mut Self, _emitter, event: &ConnectRequested, window, cx| {
                log::info!("ConnectRequested: {}:{}", event.host, event.port);
                this.start_connection(&event.host, event.port, window, cx);
            },
        );
        self._subscriptions.push(sub);
        self.connect_view = Some(connect_view);
        cx.notify();
    }

    fn start_connection(
        &mut self,
        host: &str,
        port: u16,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.screen = AppScreen::Editor;
        self.connect_view = None;

        let host = host.to_string();
        let (cols, rows) = self.create_terminal_view(&host, window, cx);

        zedra_session::session_runtime().spawn(async move {
            log::info!("RemoteSession: connecting to {}:{}...", host, port);
            match RemoteSession::connect(&host, port).await {
                Ok(session) => {
                    log::info!("RemoteSession: connected!");
                    match session.terminal_create(cols, rows).await {
                        Ok(term_id) => log::info!("Remote terminal created: {}", term_id),
                        Err(e) => log::error!("Failed to create remote terminal: {}", e),
                    }
                    zedra_session::set_active_session(session);
                    zedra_session::signal_terminal_data();
                }
                Err(e) => {
                    log::error!("RemoteSession connect failed: {}", e);
                }
            }
        });

        cx.notify();
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
        // Vertical overhead: header (88px) + separator (1px)
        let available_height = viewport.height - px(89.0);

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
                this.terminal_view = None;
                this.editor_content
                    .update(cx, |ec, cx| ec.set_terminal_view(None, cx));
                this.editor_showing_project = false;
                this.screen = AppScreen::Home;
                let preview = cx.new(|cx| FilePreviewList::new(cx));
                this.editor_stack.update(cx, |stack, cx| {
                    stack.replace(preview.into(), "Code Samples", cx);
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

        self.terminal_view = Some(terminal_view.clone());
        self.editor_content.update(cx, |ec, cx| {
            ec.set_terminal_view(Some(terminal_view), cx);
        });

        (columns as u16, rows as u16)
    }

    fn connect_with_peer_info(
        &mut self,
        peer_info: PeerInfo,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let hostname = peer_info.hostname.clone();
        log::info!("QR connect: starting connection to {}", hostname);

        self.screen = AppScreen::Editor;
        self.connect_view = None;

        let (cols, rows) = self.create_terminal_view(&hostname, window, cx);

        zedra_session::session_runtime().spawn(async move {
            log::info!("RemoteSession: connecting via QR to {}...", hostname);
            match RemoteSession::connect_with_peer_info(peer_info).await {
                Ok(session) => {
                    log::info!("RemoteSession: connected via QR!");
                    match session.terminal_create(cols, rows).await {
                        Ok(term_id) => log::info!("Remote terminal created: {}", term_id),
                        Err(e) => log::error!("Failed to create remote terminal: {}", e),
                    }
                    zedra_session::set_active_session(session);
                    zedra_session::signal_terminal_data();
                }
                Err(e) => {
                    log::error!("RemoteSession QR connect failed: {}", e);
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
            && self.terminal_view.is_none()
        {
            let project_editor = cx.new(|cx| ProjectEditor::new(cx));
            self.editor_stack.update(cx, |stack, cx| {
                stack.replace(project_editor.into(), "Project", cx);
            });
            self.editor_showing_project = true;
        }

        // Check for pending remote file content
        if self.screen == AppScreen::Editor && !self.editor_showing_project {
            if let Some((filename, content)) = take_pending_file_content() {
                let editor_view = cx.new(|cx| EditorView::new(content, cx));
                let fname = filename.clone();
                self.editor_stack.update(cx, |stack, cx| {
                    stack.push(editor_view.into(), &fname, cx);
                });
            }
        }

        // Check for QR-scanned PeerInfo
        if let Some(peer_info) = take_pending_qr_peer_info() {
            self.connect_with_peer_info(peer_info, window, cx);
        }

        let screen_content: AnyElement = match self.screen {
            AppScreen::Home => {
                div()
                    .size_full()
                    .child(self.home_view.clone())
                    .into_any_element()
            }
            AppScreen::Connect => {
                let mut root = div()
                    .size_full()
                    .bg(rgb(theme::BG_PRIMARY));

                if let Some(connect) = &self.connect_view {
                    root = root.child(connect.clone());
                }

                root.into_any_element()
            }
            AppScreen::Editor => {
                let mut root = div()
                    .size_full()
                    .bg(rgb(theme::BG_PRIMARY))
                    .flex()
                    .flex_col()
                    // DrawerHost (contains EditorContent + drawer overlay)
                    .child(
                        div()
                            .flex_1()
                            .child(self.drawer_host.clone()),
                    );

                // Transport badge (top-right, floating)
                let transport_badge = zedra_session::active_session().and_then(|s| {
                    let latency = s.latency_ms();
                    s.transport_state()
                        .map(|ts| transport_badge_info(&ts, latency))
                });

                if let Some((label, dot_color)) = transport_badge {
                    root = root.child(
                        deferred(
                            div()
                                .absolute()
                                .top(px(30.0))
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
                                        .w(px(6.0))
                                        .h(px(6.0))
                                        .rounded(px(3.0))
                                        .bg(rgb(dot_color)),
                                )
                                .child(
                                    div()
                                        .text_xs()
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
static PENDING_QR_PEER_INFO: Mutex<Option<PeerInfo>> = Mutex::new(None);
pub fn set_pending_qr_peer_info(info: PeerInfo) {
    if let Ok(mut slot) = PENDING_QR_PEER_INFO.lock() {
        *slot = Some(info);
    }
}

fn take_pending_qr_peer_info() -> Option<PeerInfo> {
    if let Ok(mut slot) = PENDING_QR_PEER_INFO.lock() {
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

fn transport_badge_info(state: &TransportState, latency_ms: u64) -> (String, u32) {
    let (base_label, color) = match state {
        TransportState::Connected { transport_name } => {
            if transport_name.contains("lan") || transport_name.contains("tcp") {
                ("LAN".to_string(), theme::ACCENT_GREEN)
            } else if transport_name.contains("tailscale") {
                ("Tailscale".to_string(), theme::ACCENT_BLUE)
            } else if transport_name.contains("relay") {
                ("Relay".to_string(), theme::ACCENT_YELLOW)
            } else {
                (transport_name.clone(), theme::ACCENT_GREEN)
            }
        }
        TransportState::Discovering => ("Discovering...".to_string(), theme::TEXT_MUTED),
        TransportState::Switching { .. } => ("Switching...".to_string(), theme::ACCENT_YELLOW),
        TransportState::Disconnected => ("Disconnected".to_string(), theme::ACCENT_RED),
    };

    let label = if latency_ms > 0 {
        format!("{} \u{00b7} {}ms", base_label, latency_ms)
    } else {
        base_label
    };

    (label, color)
}
