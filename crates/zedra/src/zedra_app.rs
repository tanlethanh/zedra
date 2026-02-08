// Root application view for Zedra
// Uses zedra-nav TabNavigator + StackNavigator for mobile navigation

use gpui::*;

use crate::file_explorer::{FileExplorer, FileSelected};
use crate::file_preview_list::{FilePreviewList, PreviewSelected, SAMPLE_FILES};
use zedra_editor::EditorView;
use zedra_nav::{DrawerHost, HeaderConfig, StackNavigator, TabBarConfig, TabNavigator};
use zedra_ssh::connection::{AuthMethod, ConnectionManager, ConnectionParams};
use zedra_terminal::view::TerminalView;

// ---------------------------------------------------------------------------
// ConnectView — extracted connection form
// ---------------------------------------------------------------------------

pub struct ConnectView {
    host: String,
    port: String,
    username: String,
    password: String,
}

impl ConnectView {
    pub fn new() -> Self {
        Self {
            host: String::new(),
            port: "2222".to_string(),
            username: "zedra".to_string(),
            password: String::new(),
        }
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
                            .child(
                                div()
                                    .px_2()
                                    .py_1()
                                    .bg(rgb(0x1e1e1e))
                                    .rounded(px(4.0))
                                    .text_color(rgb(0x5c6370))
                                    .child(if self.host.is_empty() {
                                        "192.168.1.100".to_string()
                                    } else {
                                        self.host.clone()
                                    }),
                            ),
                    )
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
                            .child(
                                div()
                                    .px_2()
                                    .py_1()
                                    .bg(rgb(0x1e1e1e))
                                    .rounded(px(4.0))
                                    .text_color(rgb(0xabb2bf))
                                    .child(self.port.clone()),
                            ),
                    )
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
                                    let host = if this.host.is_empty() {
                                        "192.168.1.100".to_string()
                                    } else {
                                        this.host.clone()
                                    };
                                    let port: u16 = this.port.parse().unwrap_or(2222);
                                    let username = if this.username.is_empty() {
                                        "zedra".to_string()
                                    } else {
                                        this.username.clone()
                                    };
                                    cx.emit(ConnectRequested {
                                        host,
                                        port,
                                        username,
                                        password: this.password.clone(),
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
            let connect_view = cx.new(|_cx| ConnectView::new());
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

        // Create tab navigator
        let terminal_stack_clone = terminal_stack.clone();
        let editor_stack_clone = editor_stack.clone();

        let tab_nav = cx.new(|cx| {
            let mut tabs = TabNavigator::new(TabBarConfig::default(), cx);
            let ts = terminal_stack_clone.clone();
            tabs.add_tab("Terminal", ">_", move |_window, _cx| ts.clone().into());
            let es = editor_stack_clone.clone();
            tabs.add_tab("Editor", "{}", move |_window, _cx| es.clone().into());
            tabs.ensure_active_view(window, cx);
            tabs
        });

        let mut subscriptions = Vec::new();

        // --- ConnectView subscription ---
        let connect_view = cx.new(|_cx| ConnectView::new());
        let terminal_stack_for_connect = terminal_stack.clone();
        let sub = cx.subscribe_in(
            &connect_view,
            window,
            move |_this: &mut ZedraApp,
                  _emitter: &Entity<ConnectView>,
                  event: &ConnectRequested,
                  _window: &mut Window,
                  cx: &mut Context<ZedraApp>| {
                let cell_width = px(9.0);
                let line_height = px(18.0);
                let columns = 80;
                let rows = 24;

                let terminal_view =
                    cx.new(|_cx| TerminalView::new(columns, rows, cell_width, line_height));

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
