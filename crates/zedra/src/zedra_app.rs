// Root application view for Zedra
// Uses zedra-nav TabNavigator + StackNavigator for mobile navigation

use gpui::*;

use crate::ai_chat::{AiChatView, AiPromptSubmitted};
use crate::file_explorer::{FileExplorer, FileSelected};
use crate::file_preview_list::{FilePreviewList, PreviewSelected, SAMPLE_FILES};
use crate::git_view::{GitCommitRequested, GitView};
use crate::session_list::{NewSessionRequested, SessionList, SessionSelected};
use zedra_editor::EditorView;
use zedra_nav::{DrawerHost, HeaderConfig, StackNavigator, TabBarConfig, TabNavigator};
use zedra_ssh::connection::{AuthMethod, ConnectionManager, ConnectionParams};
use zedra_ssh::pairing::PairingPayload;
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

/// Event emitted when the user taps "Scan QR Code".
#[derive(Clone, Debug)]
pub struct ScanQrRequested;

impl EventEmitter<ConnectRequested> for ConnectView {}
impl EventEmitter<ScanQrRequested> for ConnectView {}

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
                    // "Scan QR Code" button
                    .child(
                        div()
                            .mt_2()
                            .px_4()
                            .py_2()
                            .bg(rgb(0x98c379))
                            .rounded(px(6.0))
                            .text_color(rgb(0x282c34))
                            .cursor_pointer()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|_this, _event, _window, cx| {
                                    cx.emit(ScanQrRequested);
                                }),
                            )
                            .child(div().flex().justify_center().child("Scan QR Code")),
                    )
                    .child(
                        div()
                            .text_color(rgb(0x5c6370))
                            .text_sm()
                            .mt_2()
                            .child("— or connect manually —"),
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
    _git_stack: Entity<StackNavigator>,
    _ai_stack: Entity<StackNavigator>,
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

        // Create the git tab's stack navigator
        let git_stack = cx.new(|cx| {
            let mut stack = StackNavigator::new(Default::default(), cx);
            let git_view = cx.new(|cx| GitView::new(cx));
            stack.push(git_view.into(), "Git", cx);
            stack
        });

        // Create the AI tab's stack navigator
        let ai_stack = cx.new(|cx| {
            let mut stack = StackNavigator::new(Default::default(), cx);
            let ai_chat = cx.new(|cx| AiChatView::new(cx));
            stack.push(ai_chat.into(), "Claude Code", cx);
            stack
        });

        // Create tab navigator
        let terminal_stack_clone = terminal_stack.clone();
        let editor_stack_clone = editor_stack.clone();
        let git_stack_clone = git_stack.clone();
        let ai_stack_clone = ai_stack.clone();

        let tab_nav = cx.new(|cx| {
            let mut tabs = TabNavigator::new(TabBarConfig::default(), cx);
            let ts = terminal_stack_clone.clone();
            tabs.add_tab("Terminal", ">_", move |_window, _cx| ts.clone().into());
            let es = editor_stack_clone.clone();
            tabs.add_tab("Editor", "{}", move |_window, _cx| es.clone().into());
            let gs = git_stack_clone.clone();
            tabs.add_tab("Git", "⎇", move |_window, _cx| gs.clone().into());
            let ai = ai_stack_clone.clone();
            tabs.add_tab("AI", "◆", move |_window, _cx| ai.clone().into());
            tabs.ensure_active_view(window, cx);
            tabs
        });

        let mut subscriptions = Vec::new();

        // --- SessionList subscriptions ---
        let session_list = cx.new(|cx| SessionList::new(cx));

        // SessionSelected → push connect view pre-filled with session info
        let terminal_stack_for_session = terminal_stack.clone();
        let sub = cx.subscribe_in(
            &session_list,
            window,
            move |_this: &mut ZedraApp,
                  _emitter: &Entity<SessionList>,
                  event: &SessionSelected,
                  _window: &mut Window,
                  cx: &mut Context<ZedraApp>| {
                let session = &event.0;
                let cell_width = px(9.0);
                let line_height = px(18.0);
                let terminal_view =
                    cx.new(|_cx| TerminalView::new(80, 24, cell_width, line_height));
                terminal_stack_for_session.update(cx, |stack, cx| {
                    stack.push(terminal_view.clone().into(), "Terminal", cx);
                });
                let params = ConnectionParams {
                    addrs: vec![session.host.clone()],
                    port: session.port,
                    auth: AuthMethod::Password {
                        username: "zedra".into(),
                        password: String::new(),
                    },
                    expected_fingerprint: None,
                };
                ConnectionManager::connect(terminal_view.downgrade(), params, cx);
            },
        );
        subscriptions.push(sub);

        // NewSessionRequested → push ConnectView for manual entry
        let terminal_stack_for_new = terminal_stack.clone();
        let sub = cx.subscribe_in(
            &session_list,
            window,
            move |_this: &mut ZedraApp,
                  _emitter: &Entity<SessionList>,
                  _event: &NewSessionRequested,
                  _window: &mut Window,
                  cx: &mut Context<ZedraApp>| {
                let connect_view = cx.new(|_cx| ConnectView::new());
                terminal_stack_for_new.update(cx, |stack, cx| {
                    stack.push(connect_view.into(), "New Connection", cx);
                });
            },
        );
        subscriptions.push(sub);

        // Replace terminal stack root with SessionList
        terminal_stack.update(cx, |stack, cx| {
            stack.replace(session_list.into(), "Sessions", cx);
        });

        // --- ConnectView subscription (for manual connect from within stack) ---
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
                let terminal_view =
                    cx.new(|_cx| TerminalView::new(80, 24, cell_width, line_height));
                terminal_stack_for_connect.update(cx, |stack, cx| {
                    stack.push(terminal_view.clone().into(), "Terminal", cx);
                });
                let params = ConnectionParams {
                    addrs: vec![event.host.clone()],
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

        // --- ScanQrRequested subscription ---
        let sub = cx.subscribe_in(
            &connect_view,
            window,
            |_this: &mut ZedraApp,
             _emitter: &Entity<ConnectView>,
             _event: &ScanQrRequested,
             _window: &mut Window,
             _cx: &mut Context<ZedraApp>| {
                log::info!("Scan QR requested — launching scanner");
                let sender = crate::android_command_queue::get_command_sender();
                if let Err(e) = sender.send(
                    crate::android_command_queue::AndroidCommand::LaunchQrScanner,
                ) {
                    log::error!("Failed to send LaunchQrScanner command: {:?}", e);
                }
            },
        );
        subscriptions.push(sub);

        // --- AiChatView subscription ---
        let ai_chat = cx.new(|cx| AiChatView::new(cx));
        let ai_chat_entity = ai_chat.clone();
        let sub = cx.subscribe_in(
            &ai_chat,
            window,
            move |_this: &mut ZedraApp,
                  _emitter: &Entity<AiChatView>,
                  event: &AiPromptSubmitted,
                  _window: &mut Window,
                  cx: &mut Context<ZedraApp>| {
                // For now, provide a local echo response.
                // In production, this sends via RPC to the host daemon's ai/prompt handler.
                let prompt = event.prompt.clone();
                let entity = ai_chat_entity.clone();
                cx.spawn(|_, mut cx| async move {
                    // Simulate a brief delay
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(500))
                        .await;
                    let _ = cx.update(|_, cx| {
                        entity.update(cx, |view, cx| {
                            view.push_assistant_message(
                                format!(
                                    "[Connected to host for AI response]\n\nYour prompt: {}",
                                    prompt
                                ),
                                cx,
                            );
                        });
                    });
                })
                .detach();
            },
        );
        subscriptions.push(sub);

        // Replace AI stack root with the subscribed ai_chat
        ai_stack.update(cx, |stack, cx| {
            stack.replace(ai_chat.into(), "Claude Code", cx);
        });

        // --- GitView subscriptions ---
        let git_view = cx.new(|cx| GitView::new(cx));
        let sub = cx.subscribe_in(
            &git_view,
            window,
            |_this: &mut ZedraApp,
             _emitter: &Entity<GitView>,
             event: &GitCommitRequested,
             _window: &mut Window,
             _cx: &mut Context<ZedraApp>| {
                log::info!(
                    "Git commit requested: {} ({} files)",
                    event.message,
                    event.paths.len()
                );
                // In production, this calls RPC: git/commit
            },
        );
        subscriptions.push(sub);

        // Replace git stack root with the subscribed git_view
        git_stack.update(cx, |stack, cx| {
            stack.replace(git_view.into(), "Git", cx);
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
            _git_stack: git_stack,
            _ai_stack: ai_stack,
            _subscriptions: subscriptions,
        }
    }

    /// Handle a scanned QR pairing payload: switch to Terminal tab, create a
    /// TerminalView, push it onto the terminal stack, and auto-connect.
    pub fn handle_qr_scanned(
        &mut self,
        payload: PairingPayload,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        log::info!(
            "QR scanned — connecting to {} ({}:{})",
            payload.name,
            payload.host,
            payload.port
        );

        // Switch to Terminal tab (index 0)
        self._tab_nav.update(cx, |tabs, cx| {
            // Terminal tab at index 0 is always pre-created, so we can
            // use set_active_index which doesn't require &mut Window.
            if tabs.active_index() != 0 {
                tabs.set_active_index(0, cx);
            }
        });

        // Create terminal view
        let cell_width = px(9.0);
        let line_height = px(18.0);
        let columns = 80;
        let rows = 24;
        let terminal_view =
            cx.new(|_cx| TerminalView::new(columns, rows, cell_width, line_height));

        // Push onto the terminal stack
        self._terminal_stack.update(cx, |stack, cx| {
            stack.push(terminal_view.clone().into(), "Terminal", cx);
        });

        // Build connection params from the pairing payload
        let params = ConnectionParams {
            addrs: payload.addresses(),
            port: payload.port,
            auth: AuthMethod::PairingToken {
                token: payload.token,
            },
            expected_fingerprint: Some(payload.fingerprint),
        };

        ConnectionManager::connect(terminal_view.downgrade(), params, cx);
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
