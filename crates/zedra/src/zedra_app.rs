// Root application view for Zedra
// Uses zedra-nav TabNavigator + StackNavigator for mobile navigation

use gpui::*;

use crate::file_explorer::{FileExplorer, FileSelected};
use crate::file_preview_list::{FilePreviewList, PreviewSelected, SAMPLE_FILES};
use crate::input::{Input, InputChanged};
use zedra_editor::{EditorView, GitStack};
use zedra_nav::{DrawerHost, HeaderConfig, StackNavigator, TabBarConfig, TabNavigator};
use zedra_ssh::connection::{AuthMethod, ConnectionManager, ConnectionParams};
use zedra_terminal::view::TerminalView;

// ---------------------------------------------------------------------------
// ConnectView — extracted connection form with Input components
// ---------------------------------------------------------------------------

pub struct ConnectView {
    host_input: Entity<Input>,
    port_input: Entity<Input>,
    username_input: Entity<Input>,
    password_input: Entity<Input>,
    _subscriptions: Vec<Subscription>,
}

impl ConnectView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        // Create input entities
        let host_input = cx.new(|cx| Input::new(cx).placeholder("192.168.1.12").value("192.168.1.12"));
        let port_input = cx.new(|cx| Input::new(cx).placeholder("2222").value("2222"));
        let username_input = cx.new(|cx| Input::new(cx).placeholder("username").value("zedra"));
        let password_input = cx.new(|cx| Input::new(cx).placeholder("password").value("zedra").secure(true));

        let mut subscriptions = Vec::new();

        // Subscribe to input changes for logging/debugging
        let sub = cx.subscribe(&host_input, |_this: &mut Self, _input, event: &InputChanged, cx| {
            log::debug!("Host changed: {}", event.value);
            cx.notify();
        });
        subscriptions.push(sub);

        let sub = cx.subscribe(&port_input, |_this: &mut Self, _input, event: &InputChanged, cx| {
            log::debug!("Port changed: {}", event.value);
            cx.notify();
        });
        subscriptions.push(sub);

        let sub = cx.subscribe(&username_input, |_this: &mut Self, _input, event: &InputChanged, cx| {
            log::debug!("Username changed: {}", event.value);
            cx.notify();
        });
        subscriptions.push(sub);

        let sub = cx.subscribe(&password_input, |_this: &mut Self, _input, _event: &InputChanged, cx| {
            log::debug!("Password changed");
            cx.notify();
        });
        subscriptions.push(sub);

        Self {
            host_input,
            port_input,
            username_input,
            password_input,
            _subscriptions: subscriptions,
        }
    }

    fn get_connection_params(&self, cx: &App) -> (String, u16, String, String) {
        let host = self.host_input.read(cx).get_value().to_string();
        let host = if host.is_empty() {
            "192.168.1.12".to_string()
        } else {
            host
        };

        let port_str = self.port_input.read(cx).get_value();
        let port: u16 = port_str.parse().unwrap_or(2222);

        let username = self.username_input.read(cx).get_value().to_string();
        let username = if username.is_empty() {
            "zedra".to_string()
        } else {
            username
        };

        let password = self.password_input.read(cx).get_value().to_string();

        (host, port, username, password)
    }
}

/// Event emitted when the user taps "Connect".
#[derive(Clone, Debug)]
pub struct ConnectRequested {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
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
                    .child(
                        div()
                            .text_color(rgb(0x61afef))
                            .text_xl()
                            .child("Zedra Terminal"),
                    )
                    .child(
                        div()
                            .text_color(rgb(0x5c6370))
                            .text_sm()
                            .child("Connect to a remote host"),
                    )
                    // Host input
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_color(rgb(0xabb2bf))
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
                                    .text_color(rgb(0xabb2bf))
                                    .text_sm()
                                    .child("Port"),
                            )
                            .child(self.port_input.clone()),
                    )
                    // Username input
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_color(rgb(0xabb2bf))
                                    .text_sm()
                                    .child("Username"),
                            )
                            .child(self.username_input.clone()),
                    )
                    // Password input
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_color(rgb(0xabb2bf))
                                    .text_sm()
                                    .child("Password"),
                            )
                            .child(self.password_input.clone()),
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
                                    let (host, port, username, password) =
                                        this.get_connection_params(cx);
                                    log::info!("Connecting to {}:{} as {}", host, port, username);
                                    cx.emit(ConnectRequested {
                                        host,
                                        port,
                                        username,
                                        password,
                                    });
                                }),
                            )
                            .child(div().flex().justify_center().child("Connect")),
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
    _subscriptions: Vec<Subscription>,
}

impl ZedraApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Create the terminal tab's stack navigator
        let terminal_stack = cx.new(|cx| {
            let mut stack = StackNavigator::new(Default::default(), cx);
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
                log::info!("ConnectRequested event received: {}:{}", event.host, event.port);

                // Calculate terminal dimensions based on actual screen size
                let viewport = window.viewport_size();
                let line_height = px(16.0); // Balanced font size

                // Ensure terminal font is loaded
                zedra_terminal::load_terminal_font(window);

                // Measure actual cell width from font metrics
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

                // Calculate available space for terminal
                // Subtract: tab bar (~50px), header bar (~44px), terminal status bar (~30px)
                let available_width = viewport.width - px(16.0); // Some padding
                let available_height = viewport.height - px(50.0 + 44.0 + 30.0);

                let columns = (available_width / cell_width).floor() as usize;
                let rows = (available_height / line_height).floor() as usize;

                // Clamp to reasonable values
                let columns = columns.clamp(20, 200);
                let rows = rows.clamp(5, 100);

                log::info!(
                    "Terminal sizing: viewport={:?}, cell_width={:?}, columns={}, rows={}",
                    viewport, cell_width, columns, rows
                );

                let terminal_view =
                    cx.new(|cx| TerminalView::new(columns, rows, cell_width, line_height, cx));

                // Set keyboard request callback to show/hide soft keyboard
                terminal_view.update(cx, |view, _cx| {
                    view.set_keyboard_request(Box::new(|show| {
                        if show {
                            crate::android_jni::show_keyboard();
                        } else {
                            crate::android_jni::hide_keyboard();
                        }
                    }));
                });

                log::info!("Pushing terminal view onto stack");
                terminal_stack_for_connect.update(cx, |stack, cx| {
                    stack.push(terminal_view.clone().into(), "Terminal", cx);
                });

                let params = ConnectionParams {
                    host: event.host.clone(),
                    port: event.port,
                    auth: AuthMethod::Password {
                        username: event.username.clone(),
                        password: event.password.clone(),
                    },
                    expected_fingerprint: None,
                };
                log::info!("Starting SSH connection...");
                ConnectionManager::connect(terminal_view.downgrade(), params, cx);
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
                    let editor_view =
                        cx.new(|cx| EditorView::new(sample.content.to_string(), cx));
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
                    // Try to match the filename to a sample file
                    let filename = event.path.rsplit('/').next().unwrap_or(&event.path);
                    if let Some(sample) = SAMPLE_FILES.iter().find(|s| s.filename == filename) {
                        let editor_view =
                            cx.new(|cx| EditorView::new(sample.content.to_string(), cx));
                        editor_stack.update(cx, |stack, cx| {
                            stack.push(editor_view.into(), sample.filename, cx);
                        });
                    }
                }
            },
        );
        self._subscriptions.push(sub);

        self.drawer_host.update(cx, |host, cx| {
            host.open(file_explorer.into(), cx);
        });
    }
}

impl Render for ZedraApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
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
            )
    }
}
