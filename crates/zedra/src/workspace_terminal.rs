use std::time::Duration;

use gpui::{prelude::FluentBuilder as _, *};
use tracing::*;
use zedra_osc::OscEvent;
use zedra_session::SessionHandle;
use zedra_terminal::terminal::{TerminalEvent, TerminalHyperlinkTarget};
use zedra_terminal::view::TerminalView;

use crate::active_terminal;
use crate::button::{
    NativeFloatingButtonId, hide_native_floating_button, native_floating_button,
    native_floating_button_id,
};
use crate::platform_bridge::{
    self, CustomSheetDetent, CustomSheetOptions, NativeDictationPreviewOptions,
};
use crate::telemetry::view_telemetry;
use crate::terminal_preview_view::TerminalPreviewView;
use crate::terminal_state::TerminalState;
use crate::workspace_state::{WorkspaceState, WorkspaceStateEvent};

pub const TERMINAL_PENDING_ID: &str = "___PENDING___";
const SCROLL_TO_BOTTOM_BUTTON_THRESHOLD_LINES: usize = 50;
const SCROLL_TO_BOTTOM_BUTTON_DISMISS_DELAY: Duration = Duration::from_millis(160);

pub struct WorkspaceTerminal {
    terminal_id: String,
    #[allow(dead_code)]
    workspace_state: Entity<WorkspaceState>,
    terminal_state: Entity<TerminalState>,
    session_handle: SessionHandle,
    terminal_view: Entity<TerminalView>,
    preview: Entity<TerminalPreviewView>,
    /// Tracks whether the active terminal is in alt-screen mode (vim, opencode, etc.).
    /// Updated via AltScreenChanged event — never read via terminal_view.read(cx) in render
    /// to avoid creating a GPUI dependency that causes re-render cascades.
    is_alt_screen: bool,
    /// Last keyboard inset pushed to TerminalView. Compared against current platform value
    /// without reading terminal_view in render, breaking the GPUI observer dependency.
    last_synced_keyboard_inset: Pixels,
    scroll_to_bottom_button_id: NativeFloatingButtonId,
    dictation_preview_id: u32,
    scroll_to_bottom_button_visible: bool,
    scroll_to_bottom_button_hide_pending: bool,
    scroll_to_bottom_button_hide_generation: u64,
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

    fn scroll_button_bottom_offset() -> f32 {
        let keyboard_inset = (Self::keyboard_inset() / px(1.0)) as f32;
        platform_bridge::home_indicator_inset().max(keyboard_inset)
    }

    fn dictation_preview_bottom_offset() -> f32 {
        24.0 + Self::scroll_button_bottom_offset()
    }

    fn should_show_scroll_to_bottom_button(display_offset: usize) -> bool {
        display_offset > SCROLL_TO_BOTTOM_BUTTON_THRESHOLD_LINES
    }

    fn set_scroll_to_bottom_button_visible(
        &mut self,
        visible: bool,
        force: bool,
        cx: &mut Context<Self>,
    ) {
        if !force && self.scroll_to_bottom_button_visible == visible {
            return;
        }

        self.scroll_to_bottom_button_visible = visible;
        cx.notify();
    }

    fn refresh_scroll_to_bottom_button(&mut self, cx: &mut Context<Self>, force: bool) {
        let display_offset = self.terminal_view.read(cx).display_offset(cx);
        self.set_scroll_to_bottom_button_visible(
            Self::should_show_scroll_to_bottom_button(display_offset),
            force,
            cx,
        );
    }

    pub fn deactivate(&mut self, cx: &mut Context<Self>) {
        self.scroll_to_bottom_button_hide_pending = false;
        self.scroll_to_bottom_button_hide_generation =
            self.scroll_to_bottom_button_hide_generation.wrapping_add(1);
        hide_native_floating_button(self.scroll_to_bottom_button_id);
        platform_bridge::hide_native_dictation_preview(self.dictation_preview_id);
        self.set_scroll_to_bottom_button_visible(false, false, cx);
    }

    fn scroll_to_bottom(&mut self, cx: &mut Context<Self>) {
        self.terminal_view.update(cx, |terminal_view, cx| {
            terminal_view.scroll_to_bottom(cx);
        });
        self.schedule_scroll_to_bottom_button_hide(cx);
    }

    fn schedule_scroll_to_bottom_button_hide(&mut self, cx: &mut Context<Self>) {
        self.scroll_to_bottom_button_hide_pending = true;
        self.scroll_to_bottom_button_hide_generation =
            self.scroll_to_bottom_button_hide_generation.wrapping_add(1);
        let generation = self.scroll_to_bottom_button_hide_generation;

        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(SCROLL_TO_BOTTOM_BUTTON_DISMISS_DELAY)
                .await;
            let _ = this.update(cx, |this, cx| {
                if !this.scroll_to_bottom_button_hide_pending
                    || this.scroll_to_bottom_button_hide_generation != generation
                {
                    return;
                }

                this.scroll_to_bottom_button_hide_pending = false;
                this.refresh_scroll_to_bottom_button(cx, true);
            });
        })
        .detach();
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
                } else {
                    this.deactivate(cx);
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
                            OscEvent::IconName(name) => ts.set_icon_name(&id, name.clone()),
                            OscEvent::Cwd(cwd) => ts.set_cwd(&id, cwd.clone()),
                            OscEvent::CommandLine(cmd) => ts.set_current_command(&id, cmd.clone()),
                            OscEvent::CommandStart => ts.set_shell_running(&id),
                            OscEvent::CommandEnd { exit_code } => {
                                ts.set_shell_idle(&id, Some(*exit_code))
                            }
                            OscEvent::PromptReady => ts.set_prompt_ready(&id),
                            _ => return,
                        }
                        cx.notify();
                    });
                }
                TerminalEvent::OpenHyperlink(hyperlink) => match &hyperlink.target {
                    TerminalHyperlinkTarget::Url { url } => {
                        platform_bridge::bridge().open_url(url);
                    }
                    TerminalHyperlinkTarget::File { path, .. } => {
                        this.preview.update(cx, |preview, cx| {
                            preview.open_hyperlink(hyperlink.clone(), cx);
                        });
                        view_telemetry::record(view_telemetry::custom_sheet_file(path));
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
                TerminalEvent::AltScreenChanged(is_alt) => {
                    this.is_alt_screen = *is_alt;
                    cx.notify();
                }
                TerminalEvent::DictationPreviewChanged(text) => {
                    let active_terminal_id =
                        this.workspace_state.read(cx).active_terminal_id.clone();
                    let is_active =
                        active_terminal_id.as_deref() == Some(this.terminal_id.as_str());
                    if !is_active {
                        return;
                    }

                    match text {
                        Some(text) => platform_bridge::update_native_dictation_preview(
                            this.dictation_preview_id,
                            NativeDictationPreviewOptions {
                                text: text.clone(),
                                bottom_offset_pts: Self::dictation_preview_bottom_offset(),
                            },
                        ),
                        None => platform_bridge::hide_native_dictation_preview(
                            this.dictation_preview_id,
                        ),
                    }
                }
                TerminalEvent::ScrollbackPositionChanged { display_offset, .. } => {
                    let active_terminal_id =
                        this.workspace_state.read(cx).active_terminal_id.clone();
                    let is_active =
                        active_terminal_id.as_deref() == Some(this.terminal_id.as_str());
                    if !is_active {
                        this.deactivate(cx);
                        return;
                    }

                    let should_show = Self::should_show_scroll_to_bottom_button(*display_offset);
                    if should_show {
                        this.scroll_to_bottom_button_hide_pending = false;
                        this.scroll_to_bottom_button_hide_generation =
                            this.scroll_to_bottom_button_hide_generation.wrapping_add(1);
                    } else if this.scroll_to_bottom_button_hide_pending {
                        return;
                    }
                    this.set_scroll_to_bottom_button_visible(should_show, false, cx);
                }
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
            is_alt_screen: false,
            last_synced_keyboard_inset: px(0.0),
            scroll_to_bottom_button_id: native_floating_button_id(),
            dictation_preview_id: platform_bridge::allocate_native_dictation_preview_id(),
            scroll_to_bottom_button_visible: false,
            scroll_to_bottom_button_hide_pending: false,
            scroll_to_bottom_button_hide_generation: 0,
            _subscriptions: vec![attach_sub, terminal_events_sub],
        }
    }

    pub fn register_as_active_input(&mut self, cx: &mut Context<Self>) {
        match self.terminal_view.read(cx).input_sender(cx) {
            Some(sender) => {
                let terminal_id = self.terminal_id.clone();
                active_terminal::set_active_input(Box::new(move |bytes| {
                    if let Err(e) = sender.try_send(bytes) {
                        warn!(terminal_id, "failed to send input: {}", e);
                    }
                }));
            }
            None => {
                warn!(terminal_id = %self.terminal_id, "no input sender, skipping active input registration");
                active_terminal::clear_active_input();
            }
        }

        self.refresh_scroll_to_bottom_button(cx, true);
    }

    pub fn input_sender(&self, cx: &App) -> Option<tokio::sync::mpsc::Sender<Vec<u8>>> {
        self.terminal_view.read(cx).input_sender(cx)
    }

    pub fn set_terminal_id(&mut self, terminal_id: String, cx: &mut Context<Self>) {
        self.terminal_id = terminal_id.clone();
        self.deactivate(cx);
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
                    terminal_view.sync_remote_size_after_attach(cx);
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let keyboard_inset = Self::keyboard_inset();

        // Sync keyboard_inset to TerminalView for cursor-visibility offset in non-alt mode.
        // Uses self.last_synced_keyboard_inset (not terminal_view.read(cx)) to avoid creating
        // a GPUI render dependency on TerminalView, which would cause re-render cascades that
        // make the keyboard dismiss and reopen on every PTY output frame.
        if self.last_synced_keyboard_inset != keyboard_inset {
            self.last_synced_keyboard_inset = keyboard_inset;
            let tv = self.terminal_view.clone();
            window.defer(cx, move |_, cx| {
                tv.update(cx, |tv, cx| {
                    tv.keyboard_inset = keyboard_inset;
                    cx.notify();
                });
            });
        }
        let bottom_offset = Self::scroll_button_bottom_offset();
        let this = cx.weak_entity();

        div()
            .id(("workspace-terminal-surface", cx.entity_id()))
            .relative()
            .size_full()
            // Alt-screen TUIs (vim, OpenCode) need the container to shrink so reconcile fires
            // and SIGWINCH is sent. Non-alt apps (Claude, Codex) keep their grid fixed; the
            // element shifts content up so cursor stays visible above the keyboard instead.
            .when(self.is_alt_screen && keyboard_inset > px(0.0), |div| {
                div.pb(keyboard_inset)
            })
            .child(self.terminal_view.clone())
            .when(self.scroll_to_bottom_button_visible, move |container| {
                container.child(
                    native_floating_button(
                        ("terminal-scroll-to-bottom-button", this.entity_id()),
                        self.scroll_to_bottom_button_id,
                        "arrow.down",
                        "Scroll to bottom",
                        move |cx| {
                            let _ = this.update(cx, |terminal, cx| {
                                terminal.scroll_to_bottom(cx);
                            });
                        },
                    )
                    .absolute()
                    .right(px(24.0))
                    .bottom(px(24.0 + bottom_offset))
                    .w(px(48.0))
                    .h(px(48.0)),
                )
            })
    }
}

impl Drop for WorkspaceTerminal {
    fn drop(&mut self) {
        hide_native_floating_button(self.scroll_to_bottom_button_id);
        platform_bridge::hide_native_dictation_preview(self.dictation_preview_id);
    }
}
