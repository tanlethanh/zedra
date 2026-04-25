use gpui::{prelude::FluentBuilder as _, *};
use tracing::*;
use zedra_osc::OscEvent;
use zedra_session::SessionHandle;
use zedra_terminal::terminal::{TerminalEvent, TerminalHyperlinkTarget};
use zedra_terminal::view::TerminalView;

use crate::active_terminal;
use crate::platform_bridge::{self, CustomSheetDetent, CustomSheetOptions};
use crate::terminal_preview_view::TerminalPreviewView;
use crate::terminal_state::TerminalState;
use crate::workspace_state::{WorkspaceState, WorkspaceStateEvent};

pub const TERMINAL_PENDING_ID: &str = "___PENDING___";

pub struct WorkspaceTerminal {
    terminal_id: String,
    #[allow(dead_code)]
    workspace_state: Entity<WorkspaceState>,
    terminal_state: Entity<TerminalState>,
    session_handle: SessionHandle,
    terminal_view: Entity<TerminalView>,
    preview: Entity<TerminalPreviewView>,
    _subscriptions: Vec<Subscription>,
}

impl WorkspaceTerminal {
    fn keyboard_inset() -> Pixels {
        let bridge = platform_bridge::bridge();
        let density = bridge.density();
        if density > 0.0 {
            px(bridge.keyboard_height() as f32 / density)
        } else {
            px(0.0)
        }
    }

    fn viewport_without_keyboard(viewport: Size<Pixels>) -> Size<Pixels> {
        Size {
            width: viewport.width.max(px(0.0)),
            height: (viewport.height - Self::keyboard_inset()).max(px(0.0)),
        }
    }

    pub fn terminal_id(&self) -> &str {
        &self.terminal_id
    }

    pub fn new(
        terminal_id: String,
        workspace_state: Entity<WorkspaceState>,
        terminal_state: Entity<TerminalState>,
        session_handle: SessionHandle,
        window: &mut Window,
        initial_viewport: Size<Pixels>,
        cx: &mut Context<Self>,
    ) -> Self {
        let attach_sub = cx.subscribe(&workspace_state, |this, _ws, event, cx| match event {
            WorkspaceStateEvent::SyncComplete => {
                info!("received SyncComplete event, attempt to attach input/output channel");
                Self::attach_channel_to_terminal_view(
                    this.session_handle.clone(),
                    this.terminal_id.clone(),
                    this.terminal_view.clone(),
                    cx,
                );
            }
            WorkspaceStateEvent::TerminalCreated { id } => {
                if this.terminal_id == *id {
                    info!("received TerminalCreated event, attempt to attach input/output channel");
                    Self::attach_channel_to_terminal_view(
                        this.session_handle.clone(),
                        this.terminal_id.clone(),
                        this.terminal_view.clone(),
                        cx,
                    );
                }
            }
            WorkspaceStateEvent::TerminalOpened { id } => {
                if this.terminal_id == *id {
                    info!("received TerminalOpened event, registering as active input");
                    this.register_as_active_input(cx);
                }
            }
            _ => {}
        });

        let initial_viewport = Self::viewport_without_keyboard(initial_viewport);
        let terminal_view =
            cx.new(|cx| TerminalView::new(terminal_id.clone(), window, initial_viewport, cx));
        let workdir = workspace_state.read(cx).workdir.clone();
        terminal_view.update(cx, |terminal_view, _cx| {
            terminal_view.set_workdir(Some(workdir.clone()));
        });
        let preview = cx.new(|cx| {
            TerminalPreviewView::new(session_handle.clone(), workspace_state.clone(), cx)
        });

        let terminal_events_sub =
            cx.subscribe(&terminal_view, |this, _terminal, event, cx| match event {
                TerminalEvent::RequestResize { cols, rows } => {
                    Self::resize_remote_terminal(
                        this.session_handle.clone(),
                        this.terminal_id.clone(),
                        *cols,
                        *rows,
                        cx,
                    );
                }
                TerminalEvent::TitleChanged(title) => {
                    let id = this.terminal_id.clone();
                    let title = title.clone();
                    this.terminal_state.update(cx, |ts, cx| {
                        ts.set_title(&id, title);
                        cx.notify();
                    });
                }
                TerminalEvent::OscEvent(event) => {
                    let id = this.terminal_id.clone();
                    this.terminal_state.update(cx, |ts, cx| {
                        match event {
                            OscEvent::Title(title) => ts.set_title(&id, Some(title.clone())),
                            OscEvent::ResetTitle => ts.set_title(&id, None),
                            OscEvent::Cwd(cwd) => ts.set_cwd(&id, cwd.clone()),
                            OscEvent::CommandLine(cmd) => ts.set_current_command(&id, cmd.clone()),
                            OscEvent::CommandStart => ts.set_shell_running(&id),
                            OscEvent::CommandEnd { exit_code } => {
                                ts.set_shell_idle(&id, Some(*exit_code))
                            }
                            OscEvent::PromptReady => ts.set_shell_idle(&id, None),
                            _ => return,
                        }
                        cx.notify();
                    });
                }
                TerminalEvent::OpenHyperlink(hyperlink) => match &hyperlink.target {
                    TerminalHyperlinkTarget::Url { url } => {
                        platform_bridge::bridge().open_url(url);
                    }
                    TerminalHyperlinkTarget::File { .. } => {
                        this.preview.update(cx, |preview, cx| {
                            preview.open_hyperlink(hyperlink.clone(), cx);
                        });
                        platform_bridge::show_custom_sheet(
                            CustomSheetOptions {
                                detents: vec![CustomSheetDetent::Large],
                                initial_detent: CustomSheetDetent::Large,
                                shows_grabber: true,
                                expands_on_scroll_edge: true,
                                edge_attached_in_compact_height: false,
                                width_follows_preferred_content_size_when_edge_attached: false,
                                corner_radius: None,
                                modal_in_presentation: false,
                            },
                            this.preview.clone(),
                        );
                    }
                },
            });

        if terminal_id != TERMINAL_PENDING_ID {
            Self::attach_channel_to_terminal_view(
                session_handle.clone(),
                terminal_id.clone(),
                terminal_view.clone(),
                cx,
            );
        }

        Self {
            terminal_id,
            workspace_state,
            terminal_state,
            session_handle,
            terminal_view,
            preview,
            _subscriptions: vec![attach_sub, terminal_events_sub],
        }
    }

    pub fn register_as_active_input(&self, cx: &App) {
        let Some(sender) = self.terminal_view.read(cx).input_sender(cx) else {
            warn!(terminal_id = %self.terminal_id, "no input sender, skipping active input registration");
            return;
        };
        let terminal_id = self.terminal_id.clone();
        active_terminal::set_active_input(Box::new(move |bytes| {
            if let Err(e) = sender.try_send(bytes) {
                warn!(terminal_id, "failed to send input: {}", e);
            }
        }));
    }

    pub fn set_terminal_id(&mut self, terminal_id: String, cx: &mut Context<Self>) {
        self.terminal_id = terminal_id.clone();
        self.terminal_view.update(cx, |terminal_view, _cx| {
            terminal_view.set_terminal_id(terminal_id);
        });

        let (cols, rows) = self.terminal_view.read(cx).remote_size(cx);
        Self::resize_remote_terminal(
            self.session_handle.clone(),
            self.terminal_id.clone(),
            cols,
            rows,
            cx,
        );
    }

    fn attach_channel_to_terminal_view(
        session_handle: SessionHandle,
        terminal_id: String,
        terminal_view: Entity<TerminalView>,
        cx: &mut Context<Self>,
    ) {
        let Some(remote_terminal) = session_handle.terminal(&terminal_id) else {
            warn!("no remote terminal found with id: {}", terminal_id);
            return;
        };
        match remote_terminal.take_chanel() {
            Ok((input_tx, output_rx)) => {
                terminal_view.update(cx, |terminal_view, cx| {
                    terminal_view.attach_channel(input_tx, output_rx, cx);
                    info!("attached channel to terminal");
                });
            }
            Err(e) => {
                warn!("failed to attach input/output channel: {}", e);
            }
        }
    }

    fn resize_remote_terminal(
        session_handle: SessionHandle,
        terminal_id: String,
        cols: u16,
        rows: u16,
        cx: &mut Context<Self>,
    ) {
        if terminal_id == TERMINAL_PENDING_ID {
            return;
        }

        cx.spawn(async move |this, cx| {
            match session_handle
                .terminal_resize(&terminal_id, cols, rows)
                .await
            {
                Ok(_) => {
                    info!(terminal_id, cols, rows, "resized remote terminal");
                    if let Err(e) = this.update(cx, |_, cx| cx.notify()) {
                        warn!("failed to notify from terminal resize task: {}", e);
                    }
                }
                Err(error) => {
                    warn!(
                        terminal_id,
                        cols, rows, "failed to resize remote terminal: {}", error
                    );
                }
            }
        })
        .detach();
    }
}

impl Render for WorkspaceTerminal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let keyboard_inset = Self::keyboard_inset();

        div()
            .id(("workspace-terminal-surface", cx.entity_id()))
            .size_full()
            .when(keyboard_inset > px(0.0), |div| div.pb(keyboard_inset))
            .child(self.terminal_view.clone())
    }
}
