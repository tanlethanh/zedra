// Root application view for Zedra
// Uses zedra-nav TabNavigator + StackNavigator for mobile navigation

use std::sync::Arc;

use gpui::*;

use crate::file_explorer::{FileExplorer, FileSelected};
use crate::file_preview_list::{FilePreviewList, PreviewSelected, SAMPLE_FILES};
use crate::input::{Input, InputChanged};
use crate::project_editor::ProjectEditor;
use zedra_editor::{EditorView, GitStack};
use zedra_nav::{DrawerHost, HeaderConfig, StackNavigator, TabBarConfig, TabNavigator};
use zedra_session::RemoteSession;
use zedra_terminal::view::{DisconnectRequested, TerminalView};
use zedra_transport::{PeerInfo, TransportState};

// ---------------------------------------------------------------------------
// ConnectView — extracted connection form with Input components
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

/// Event emitted when the user taps "Connect".
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
            .bg(rgb(0x1e1e1e))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .p_6()
                    .w(px(300.0))
                    .bg(rgb(0x282c34))
                    .rounded(px(12.0))
                    .border_1()
                    .border_color(rgb(0x3e4451))
                    // Title
                    .child(div().text_color(rgb(0x61afef)).text_xl().child("Zedra"))
                    .child(
                        div()
                            .text_color(rgb(0x5c6370))
                            .text_sm()
                            .child("Connect to zedra-host daemon"),
                    )
                    // Host input
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(div().text_color(rgb(0xabb2bf)).text_sm().child("Host"))
                            .child(self.host_input.clone()),
                    )
                    // Port input
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(div().text_color(rgb(0xabb2bf)).text_sm().child("Port"))
                            .child(self.port_input.clone()),
                    )
                    // Connect button
                    .child(
                        div()
                            .mt_2()
                            .px_4()
                            .py_2()
                            .bg(rgb(0x61afef))
                            .rounded(px(6.0))
                            .text_color(rgb(0x282c34))
                            .cursor_pointer()
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
                            .text_color(rgb(0x5c6370))
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
                            .border_color(rgb(0x61afef))
                            .text_color(rgb(0x61afef))
                            .cursor_pointer()
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
// ZedraApp — wires TabNavigator containing StackNavigators
// ---------------------------------------------------------------------------

pub struct ZedraApp {
    drawer_host: Entity<DrawerHost>,
    _tab_nav: Entity<TabNavigator>,
    _terminal_stack: Entity<StackNavigator>,
    editor_stack: Entity<StackNavigator>,
    session: Option<Arc<RemoteSession>>,
    /// Whether the editor stack is currently showing ProjectEditor (session active)
    editor_showing_project: bool,
    render_count: u64,
    _subscriptions: Vec<Subscription>,
}

impl ZedraApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Create the terminal tab's stack navigator with header enabled (for back button)
        let terminal_stack = cx.new(|cx| {
            let mut stack = StackNavigator::new(
                HeaderConfig {
                    show_header: true,
                    ..Default::default()
                },
                cx,
            );
            let connect_view = cx.new(|cx| ConnectView::new(cx));
            stack.push(connect_view.into(), "Zedra Terminal", cx);
            stack
        });

        // Create the editor tab's stack navigator with header enabled (for back button)
        let editor_stack = cx.new(|cx| {
            let mut stack = StackNavigator::new(
                HeaderConfig {
                    show_header: true,
                    ..Default::default()
                },
                cx,
            );
            let preview_list = cx.new(|cx| FilePreviewList::new(cx));
            stack.push(preview_list.into(), "Code Samples", cx);
            stack
        });

        // Create the git tab's view with gesture-based drawer
        let git_stack = cx.new(|cx| GitStack::new(cx));

        // Create tab navigator
        let terminal_stack_clone = terminal_stack.clone();
        let editor_stack_clone = editor_stack.clone();

        let tab_nav = cx.new(|cx| {
            let mut tabs = TabNavigator::new(TabBarConfig::default(), cx);
            let ts = terminal_stack_clone.clone();
            tabs.add_tab("Terminal", ">_", move |_window, _cx| ts.clone().into());
            let es = editor_stack_clone.clone();
            tabs.add_tab("Editor", "{}", move |_window, _cx| es.clone().into());
            let gs = git_stack.clone();
            tabs.add_tab("Git", "*", move |_window, _cx| gs.clone().into());
            tabs.ensure_active_view(window, cx);
            tabs
        });

        let mut subscriptions = Vec::new();

        // --- ConnectView subscription ---
        let connect_view = cx.new(|cx| ConnectView::new(cx));
        let terminal_stack_for_connect = terminal_stack.clone();
        let sub = cx.subscribe_in(
            &connect_view,
            window,
            move |_this: &mut ZedraApp,
                  _emitter: &Entity<ConnectView>,
                  event: &ConnectRequested,
                  window: &mut Window,
                  cx: &mut Context<ZedraApp>| {
                log::info!(
                    "ConnectRequested event received: {}:{}",
                    event.host,
                    event.port
                );

                // Calculate terminal dimensions based on actual screen size
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
                // Vertical overhead: tab bar (56px) + stack header (44px) + terminal status bar (~24px)
                let available_height = viewport.height - px(124.0);

                // Subtract 1 column to account for subpixel rounding at non-integer scale factors
                let columns = ((available_width / cell_width).floor() as usize).saturating_sub(1);
                let rows = (available_height / line_height).floor() as usize;
                let columns = columns.clamp(20, 200);
                let rows = rows.clamp(5, 100);

                log::info!(
                    "Terminal sizing: viewport={:?}, cell_width={:?}, columns={}, rows={}",
                    viewport,
                    cell_width,
                    columns,
                    rows
                );

                let terminal_view =
                    cx.new(|cx| TerminalView::new(columns, rows, cell_width, line_height, cx));

                terminal_view.update(cx, |view, _cx| {
                    view.set_keyboard_request(Box::new(|show| {
                        if show {
                            crate::android_jni::show_keyboard();
                        } else {
                            crate::android_jni::hide_keyboard();
                        }
                    }));
                });

                // Subscribe to disconnect event (stored to tie lifetime to ZedraApp)
                let stack_for_disconnect = terminal_stack_for_connect.clone();
                let editor_stack_for_disconnect = _this.editor_stack.clone();
                let disconnect_sub = cx.subscribe(
                    &terminal_view,
                    move |this, _terminal, _event: &DisconnectRequested, cx| {
                        log::info!("DisconnectRequested received, popping terminal view");
                        zedra_session::clear_active_session();
                        this.session = None;
                        stack_for_disconnect.update(cx, |stack, cx| {
                            stack.pop(cx);
                        });
                        // Swap editor stack back to FilePreviewList
                        if this.editor_showing_project {
                            let preview = cx.new(|cx| FilePreviewList::new(cx));
                            editor_stack_for_disconnect.update(cx, |stack, cx| {
                                stack.replace(preview.into(), "Code Samples", cx);
                            });
                            this.editor_showing_project = false;
                        }
                    },
                );
                _this._subscriptions.push(disconnect_sub);

                log::info!("Pushing terminal view onto stack");
                terminal_stack_for_connect.update(cx, |stack, cx| {
                    stack.push(terminal_view.clone().into(), "Terminal", cx);
                });

                // Set initial status
                terminal_view.update(cx, |view, _cx| {
                    view.set_status(format!("Connecting to {}:{}...", event.host, event.port));
                });

                // Connect via RPC session (async on the session runtime)
                // Terminal view will automatically pick up output data from the
                // active session's buffer in process_output(), and update its
                // connected status when data arrives.
                let host = event.host.clone();
                let port = event.port;
                let cols = columns as u16;
                let term_rows = rows as u16;

                zedra_session::session_runtime().spawn(async move {
                    log::info!("RemoteSession: connecting to {}:{}...", host, port);
                    match RemoteSession::connect(&host, port).await {
                        Ok(session) => {
                            log::info!("RemoteSession: connected!");

                            // Create terminal on the remote host
                            match session.terminal_create(cols, term_rows).await {
                                Ok(term_id) => {
                                    log::info!("Remote terminal created: {}", term_id);
                                }
                                Err(e) => {
                                    log::error!("Failed to create remote terminal: {}", e);
                                }
                            }

                            // Store as active session — terminal view will pick up
                            // output via zedra_session::active_session().output_buffer()
                            zedra_session::set_active_session(session);
                            // Trigger re-render so ProjectEditor swap is picked up
                            zedra_session::signal_terminal_data();
                        }
                        Err(e) => {
                            log::error!("RemoteSession connect failed: {}", e);
                        }
                    }
                });
            },
        );
        subscriptions.push(sub);

        // Replace terminal stack root with the subscribed connect_view
        terminal_stack.update(cx, |stack, cx| {
            stack.replace(connect_view.into(), "Zedra Terminal", cx);
        });

        // --- PreviewSelected subscription ---
        // Get the FilePreviewList entity from the editor stack root.
        // We need to create it outside and subscribe, then replace.
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

        // Replace editor stack root with the subscribed preview_list
        editor_stack.update(cx, |stack, cx| {
            stack.replace(preview_list.into(), "Code Samples", cx);
        });

        // Wrap tab_nav in a DrawerHost
        let tab_nav_clone = tab_nav.clone();
        let drawer_host = cx.new(|cx| DrawerHost::new(tab_nav_clone.into(), cx));

        Self {
            drawer_host,
            _tab_nav: tab_nav,
            _terminal_stack: terminal_stack,
            editor_stack,
            session: None,
            editor_showing_project: false,
            render_count: 0,
            _subscriptions: subscriptions,
        }
    }

    fn open_file_drawer(&mut self, cx: &mut Context<Self>) {
        let file_explorer = cx.new(|cx| FileExplorer::new(cx));

        let drawer_host = self.drawer_host.clone();
        let editor_stack = self.editor_stack.clone();
        let sub = cx.subscribe(
            &file_explorer,
            move |_this: &mut ZedraApp,
                  _emitter: Entity<FileExplorer>,
                  event: &FileSelected,
                  cx: &mut Context<ZedraApp>| {
                drawer_host.update(cx, |host, cx| {
                    host.close(cx);
                });
                if !event.path.is_empty() {
                    log::info!("File selected from drawer: {}", event.path);
                    let path = event.path.clone();
                    let filename = path.rsplit('/').next().unwrap_or(&path).to_string();

                    // Try remote fs/read first, fall back to sample files
                    if let Some(session) = zedra_session::active_session() {
                        let _editor_stack = editor_stack.clone();
                        let filename_clone = filename.clone();
                        zedra_session::session_runtime().spawn(async move {
                            match session.fs_read(&path).await {
                                Ok(content) => {
                                    // Store content for main thread to pick up
                                    set_pending_file_content(filename_clone, content);
                                    zedra_session::signal_terminal_data();
                                }
                                Err(e) => {
                                    log::error!("fs/read failed for {}: {}", path, e);
                                }
                            }
                        });
                    } else {
                        // Fallback to sample files
                        if let Some(sample) = SAMPLE_FILES.iter().find(|s| s.filename == filename) {
                            let editor_view =
                                cx.new(|cx| EditorView::new(sample.content.to_string(), cx));
                            editor_stack.update(cx, |stack, cx| {
                                stack.push(editor_view.into(), sample.filename, cx);
                            });
                        }
                    }
                }
            },
        );
        self._subscriptions.push(sub);

        self.drawer_host.update(cx, |host, cx| {
            host.open(file_explorer.into(), cx);
        });
    }

    /// Initiate a connection via TransportManager using PeerInfo from QR scan.
    fn connect_with_peer_info(
        &mut self,
        peer_info: PeerInfo,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let hostname = peer_info.hostname.clone();
        log::info!("QR connect: starting connection to {}", hostname);

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
        let available_height = viewport.height - px(124.0);

        let columns = ((available_width / cell_width).floor() as usize).saturating_sub(1);
        let rows = (available_height / line_height).floor() as usize;
        let columns = columns.clamp(20, 200);
        let rows = rows.clamp(5, 100);

        let terminal_view =
            cx.new(|cx| TerminalView::new(columns, rows, cell_width, line_height, cx));

        terminal_view.update(cx, |view, _cx| {
            view.set_keyboard_request(Box::new(|show| {
                if show {
                    crate::android_jni::show_keyboard();
                } else {
                    crate::android_jni::hide_keyboard();
                }
            }));
        });

        // Subscribe to disconnect event
        let terminal_stack = self._terminal_stack.clone();
        let editor_stack = self.editor_stack.clone();
        let disconnect_sub = cx.subscribe(
            &terminal_view,
            move |this, _terminal, _event: &DisconnectRequested, cx| {
                log::info!("DisconnectRequested received (QR), popping terminal view");
                zedra_session::clear_active_session();
                this.session = None;
                terminal_stack.update(cx, |stack, cx| {
                    stack.pop(cx);
                });
                if this.editor_showing_project {
                    let preview = cx.new(|cx| FilePreviewList::new(cx));
                    editor_stack.update(cx, |stack, cx| {
                        stack.replace(preview.into(), "Code Samples", cx);
                    });
                    this.editor_showing_project = false;
                }
            },
        );
        self._subscriptions.push(disconnect_sub);

        // Push terminal view onto the stack
        let terminal_stack = self._terminal_stack.clone();
        terminal_stack.update(cx, |stack, cx| {
            stack.push(terminal_view.clone().into(), "Terminal", cx);
        });

        terminal_view.update(cx, |view, _cx| {
            view.set_status(format!("Connecting to {}...", hostname));
        });

        // Spawn async connection via TransportManager
        let cols = columns as u16;
        let term_rows = rows as u16;

        zedra_session::session_runtime().spawn(async move {
            log::info!("RemoteSession: connecting via QR to {}...", hostname);
            match RemoteSession::connect_with_peer_info(peer_info).await {
                Ok(session) => {
                    log::info!("RemoteSession: connected via QR!");
                    match session.terminal_create(cols, term_rows).await {
                        Ok(term_id) => {
                            log::info!("Remote terminal created: {}", term_id);
                        }
                        Err(e) => {
                            log::error!("Failed to create remote terminal: {}", e);
                        }
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
                "ZedraApp::render #{}, editor_project={}",
                self.render_count,
                self.editor_showing_project
            );
        }

        // Swap editor stack to ProjectEditor when session becomes active
        if zedra_session::active_session().is_some() && !self.editor_showing_project {
            let project_editor = cx.new(|cx| ProjectEditor::new(cx));
            self.editor_stack.update(cx, |stack, cx| {
                stack.replace(project_editor.into(), "Project", cx);
            });
            self.editor_showing_project = true;
        }

        // Check for pending remote file content and open in editor
        // (only when not showing ProjectEditor — it handles its own file loading)
        if !self.editor_showing_project {
            if let Some((filename, content)) = take_pending_file_content() {
                let editor_view = cx.new(|cx| EditorView::new(content, cx));
                let fname = filename.clone();
                self.editor_stack.update(cx, |stack, cx| {
                    stack.push(editor_view.into(), &fname, cx);
                });
            }
        }

        // Check for QR-scanned PeerInfo and initiate connection
        if let Some(peer_info) = take_pending_qr_peer_info() {
            self.connect_with_peer_info(peer_info, window, cx);
        }

        // Compute transport badge info (label + color) with latency
        let transport_badge = zedra_session::active_session().and_then(|s| {
            let latency = s.latency_ms();
            s.transport_state()
                .map(|ts| transport_badge_info(&ts, latency))
        });

        let mut root = div()
            .size_full()
            .child(self.drawer_host.clone())
            // Hamburger menu button (top-left, floating)
            .child(
                deferred(
                    div()
                        .absolute()
                        .top(px(8.0))
                        .left(px(8.0))
                        .w(px(36.0))
                        .h(px(36.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(6.0))
                        .bg(hsla(220.0 / 360.0, 0.13, 0.14, 0.8))
                        .cursor_pointer()
                        .hover(|s| s.bg(rgb(0x2c313a)))
                        .text_color(rgb(0xabb2bf))
                        .text_lg()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _event, _window, cx| {
                                if this.drawer_host.read(cx).is_open() {
                                    this.drawer_host.update(cx, |host, cx| host.close(cx));
                                } else {
                                    this.open_file_drawer(cx);
                                }
                            }),
                        )
                        .child("☰"),
                )
                .with_priority(100),
            );

        // Show transport state badge (top-right) with colored dot + latency
        if let Some((label, dot_color)) = transport_badge {
            root = root.child(
                deferred(
                    div()
                        .absolute()
                        .top(px(10.0))
                        .right(px(8.0))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(4.0))
                        .px_2()
                        .py(px(2.0))
                        .rounded(px(4.0))
                        .bg(hsla(220.0 / 360.0, 0.13, 0.14, 0.8))
                        // Colored dot
                        .child(
                            div()
                                .w(px(6.0))
                                .h(px(6.0))
                                .rounded(px(3.0))
                                .bg(rgb(dot_color)),
                        )
                        // Label text
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

        root
    }
}

// ---------------------------------------------------------------------------
// Global pending state for async file content → main thread
// ---------------------------------------------------------------------------

use std::sync::Mutex;

static PENDING_FILE_CONTENT: Mutex<Option<(String, String)>> = Mutex::new(None);

// ---------------------------------------------------------------------------
// Global pending state for QR-scanned PeerInfo → main thread
// ---------------------------------------------------------------------------

static PENDING_QR_PEER_INFO: Mutex<Option<PeerInfo>> = Mutex::new(None);

/// Store a PeerInfo parsed from a QR code for the main thread to pick up.
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

/// Return (label, dot_color) for a transport state + latency.
fn transport_badge_info(state: &TransportState, latency_ms: u64) -> (String, u32) {
    let (base_label, color) = match state {
        TransportState::Connected { transport_name } => {
            if transport_name.contains("lan") || transport_name.contains("tcp") {
                ("LAN".to_string(), 0x98c379u32) // green
            } else if transport_name.contains("tailscale") {
                ("Tailscale".to_string(), 0x61afefu32) // blue
            } else if transport_name.contains("relay") {
                ("Relay".to_string(), 0xe5c07bu32) // yellow
            } else {
                (transport_name.clone(), 0x98c379u32)
            }
        }
        TransportState::Discovering => ("Discovering...".to_string(), 0x5c6370u32), // gray
        TransportState::Switching { .. } => ("Switching...".to_string(), 0xe5c07bu32), // yellow
        TransportState::Disconnected => ("Disconnected".to_string(), 0xe06c75u32), // red
    };

    let label = if latency_ms > 0 {
        format!("{} \u{00b7} {}ms", base_label, latency_ms)
    } else {
        base_label
    };

    (label, color)
}
