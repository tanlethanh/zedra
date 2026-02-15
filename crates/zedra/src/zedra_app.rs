// Root application view for Zedra
// Screen-based navigation: Home → Editor
// Drawer-based navigation with footer icons replaces the bottom tab bar.

use std::sync::Arc;

use gpui::*;

use crate::app_drawer::{AppDrawer, AppDrawerEvent, DrawerSection};
use crate::file_preview_list::{FilePreviewList, PreviewSelected, SAMPLE_FILES};
use crate::input::{Input, InputChanged};
use crate::project_editor::ProjectEditor;
use crate::home_view::{HomeEvent, HomeView};
use crate::theme;
use zedra_editor::{EditorView, GitStack};
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
                                    crate::platform_bridge::launch_qr_scanner();
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
// ZedraApp — screen-based navigation (Home → Editor)
// ---------------------------------------------------------------------------

pub struct ZedraApp {
    screen: AppScreen,
    home_view: Entity<HomeView>,
    connect_view: Option<Entity<ConnectView>>,
    drawer_host: Entity<DrawerHost>,
    editor_stack: Entity<StackNavigator>,
    terminal_view: Option<Entity<TerminalView>>,
    session: Option<Arc<RemoteSession>>,
    editor_showing_project: bool,
    active_section: DrawerSection,
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
            |this: &mut Self, _emitter, event: &HomeEvent, _window, cx| {
                match event {
                    HomeEvent::ConnectTapped => {
                        log::info!("Home: Connect tapped");
                        this.show_connect_form(cx);
                    }
                    HomeEvent::ScanQrTapped => {
                        log::info!("Home: Scan QR tapped");
                        crate::platform_bridge::launch_qr_scanner();
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

        // --- DrawerHost (wraps editor content, drawer = AppDrawer) ---
        let editor_stack_clone = editor_stack.clone();
        let drawer_host = cx.new(|cx| DrawerHost::new(editor_stack_clone.into(), cx));

        Self {
            screen: AppScreen::Home,
            home_view,
            connect_view: None,
            drawer_host,
            editor_stack,
            terminal_view: None,
            session: None,
            editor_showing_project: false,
            active_section: DrawerSection::Files,
            render_count: 0,
            _subscriptions: subscriptions,
        }
    }

    fn show_connect_form(&mut self, cx: &mut Context<Self>) {
        self.screen = AppScreen::Connect;
        let connect_view = cx.new(|cx| ConnectView::new(cx));
        let sub = cx.subscribe(
            &connect_view,
            |this: &mut Self, _emitter, event: &ConnectRequested, cx| {
                log::info!("ConnectRequested: {}:{}", event.host, event.port);
                this.start_connection(&event.host, event.port, cx);
            },
        );
        self._subscriptions.push(sub);
        self.connect_view = Some(connect_view);
        cx.notify();
    }

    fn start_connection(&mut self, host: &str, port: u16, cx: &mut Context<Self>) {
        // Transition to Editor screen immediately
        self.screen = AppScreen::Editor;
        self.connect_view = None;

        let host = host.to_string();

        zedra_session::session_runtime().spawn(async move {
            log::info!("RemoteSession: connecting to {}:{}...", host, port);
            match RemoteSession::connect(&host, port).await {
                Ok(session) => {
                    log::info!("RemoteSession: connected!");
                    match session.terminal_create(80, 24).await {
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

    fn open_app_drawer(&mut self, cx: &mut Context<Self>) {
        let app_drawer = cx.new(|cx| AppDrawer::new(cx));

        let drawer_host = self.drawer_host.clone();
        let editor_stack = self.editor_stack.clone();
        let sub = cx.subscribe(
            &app_drawer,
            move |this: &mut ZedraApp,
                  _emitter: Entity<AppDrawer>,
                  event: &AppDrawerEvent,
                  cx: &mut Context<ZedraApp>| {
                match event {
                    AppDrawerEvent::FileSelected(path) => {
                        drawer_host.update(cx, |host, cx| host.close(cx));
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
                                            set_pending_file_content(filename_clone, content);
                                            zedra_session::signal_terminal_data();
                                        }
                                        Err(e) => {
                                            log::error!("fs/read failed for {}: {}", path, e);
                                        }
                                    }
                                });
                            } else if let Some(sample) =
                                SAMPLE_FILES.iter().find(|s| s.filename == filename)
                            {
                                let editor_view = cx
                                    .new(|cx| EditorView::new(sample.content.to_string(), cx));
                                editor_stack.update(cx, |stack, cx| {
                                    stack.push(editor_view.into(), sample.filename, cx);
                                });
                            }
                        }
                    }
                    AppDrawerEvent::SectionChanged(section) => {
                        this.active_section = *section;
                        drawer_host.update(cx, |host, cx| host.close(cx));
                        this.switch_section(*section, cx);
                    }
                }
            },
        );
        self._subscriptions.push(sub);

        self.drawer_host.update(cx, |host, cx| {
            host.open(app_drawer.into(), cx);
        });
    }

    fn switch_section(&mut self, section: DrawerSection, cx: &mut Context<Self>) {
        match section {
            DrawerSection::Files => {
                // Already showing editor stack — swap to preview list if not project
                if !self.editor_showing_project {
                    let preview = cx.new(|cx| FilePreviewList::new(cx));
                    self.editor_stack.update(cx, |stack, cx| {
                        stack.replace(preview.into(), "Code Samples", cx);
                    });
                }
            }
            DrawerSection::Terminal => {
                if let Some(terminal) = &self.terminal_view {
                    let terminal = terminal.clone();
                    self.editor_stack.update(cx, |stack, cx| {
                        stack.replace(terminal.into(), "Terminal", cx);
                    });
                }
            }
            DrawerSection::Git => {
                let git_stack = cx.new(|cx| GitStack::new(cx));
                self.editor_stack.update(cx, |stack, cx| {
                    stack.replace(git_stack.into(), "Git", cx);
                });
            }
            DrawerSection::Packages => {
                // Placeholder
            }
        }
        cx.notify();
    }

    fn connect_with_peer_info(
        &mut self,
        peer_info: PeerInfo,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let hostname = peer_info.hostname.clone();
        log::info!("QR connect: starting connection to {}", hostname);

        // Transition to Editor screen
        self.screen = AppScreen::Editor;
        self.connect_view = None;

        // Calculate terminal dimensions
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
        // Vertical overhead: custom header (88px) + terminal status bar (~24px)
        let available_height = viewport.height - px(112.0);

        let columns = ((available_width / cell_width).floor() as usize).saturating_sub(1);
        let rows = (available_height / line_height).floor() as usize;
        let columns = columns.clamp(20, 200);
        let rows = rows.clamp(5, 100);

        let terminal_view =
            cx.new(|cx| TerminalView::new(columns, rows, cell_width, line_height, cx));

        terminal_view.update(cx, |view, _cx| {
            view.set_keyboard_request(Box::new(|show| {
                if show {
                    crate::platform_bridge::show_keyboard();
                } else {
                    crate::platform_bridge::hide_keyboard();
                }
            }));
        });

        // Subscribe to disconnect event
        let disconnect_sub = cx.subscribe(
            &terminal_view,
            |this, _terminal, _event: &DisconnectRequested, cx| {
                log::info!("DisconnectRequested received, returning to Home");
                zedra_session::clear_active_session();
                this.session = None;
                this.terminal_view = None;
                this.editor_showing_project = false;
                this.screen = AppScreen::Home;
                // Reset editor stack
                let preview = cx.new(|cx| FilePreviewList::new(cx));
                this.editor_stack.update(cx, |stack, cx| {
                    stack.replace(preview.into(), "Code Samples", cx);
                });
                cx.notify();
            },
        );
        self._subscriptions.push(disconnect_sub);

        // Show terminal in editor stack
        self.editor_stack.update(cx, |stack, cx| {
            stack.replace(terminal_view.clone().into(), "Terminal", cx);
        });

        terminal_view.update(cx, |view, _cx| {
            view.set_status(format!("Connecting to {}...", hostname));
        });

        self.terminal_view = Some(terminal_view);

        let cols = columns as u16;
        let term_rows = rows as u16;

        zedra_session::session_runtime().spawn(async move {
            log::info!("RemoteSession: connecting via QR to {}...", hostname);
            match RemoteSession::connect_with_peer_info(peer_info).await {
                Ok(session) => {
                    log::info!("RemoteSession: connected via QR!");
                    match session.terminal_create(cols, term_rows).await {
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

    fn render_editor_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let title = self
            .editor_stack
            .read(cx)
            .current_title()
            .cloned()
            .unwrap_or_default();

        div()
            .h(px(88.0))
            .flex()
            .flex_row()
            .items_center()
            .px(px(16.0))
            .bg(rgb(theme::BG_PRIMARY))
            .child(
                // Logo button (opens drawer)
                div()
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
                                this.open_app_drawer(cx);
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
                div()
                    .ml_3()
                    .flex_1()
                    .text_color(rgb(theme::TEXT_SECONDARY))
                    .text_sm()
                    .child(title),
            )
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

        // Swap editor stack to ProjectEditor when session becomes active
        if self.screen == AppScreen::Editor
            && zedra_session::active_session().is_some()
            && !self.editor_showing_project
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
                    // Custom 88px header
                    .child(self.render_editor_header(cx))
                    // Separator
                    .child(div().h(px(1.0)).bg(rgb(theme::BORDER_SUBTLE)))
                    // Main content (DrawerHost wrapping editor stack)
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
