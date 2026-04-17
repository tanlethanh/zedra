use gpui::*;
use tracing::*;
use zedra_session::SessionHandle;
use zedra_terminal::view::TerminalView;

use crate::{
    fonts, theme,
    workspace_state::{WorkspaceState, WorkspaceStateEvent},
};

pub struct WorkspaceTerminal {
    terminal_id: String,
    workspace_state: Entity<WorkspaceState>,
    #[allow(dead_code)]
    session_handle: SessionHandle,
    content: Option<Entity<TerminalView>>,
    _subscriptions: Vec<Subscription>,
}

impl WorkspaceTerminal {
    pub fn terminal_id(&self) -> &str {
        &self.terminal_id
    }

    pub fn new(
        terminal_id: String,
        workspace_state: Entity<WorkspaceState>,
        session_handle: SessionHandle,
        cx: &mut Context<Self>,
    ) -> Self {
        // Attach the input/ouput channel whenever receiving SyncComplete event.
        // This mostly happens when the session reconnects.
        let attach_sub = cx.subscribe(&workspace_state, |this, _ws, event, cx| {
            if matches!(event, WorkspaceStateEvent::SyncComplete) {
                info!("received SyncComplete event, attempt to attach input/output channel");
                this.attach_channel(cx);
            }
        });

        Self {
            terminal_id,
            workspace_state,
            session_handle,
            content: None,
            _subscriptions: vec![attach_sub],
        }
    }

    pub fn attach_channel(&mut self, cx: &mut Context<Self>) {
        if let Some(terminal) = self
            .workspace_state
            .read(cx)
            .remote_terminals
            .iter()
            .find(|t| t.id() == self.terminal_id)
        {
            if let Some(terminal_view) = &self.content {
                match terminal.take_chanel() {
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
            } else {
                error!("no terminal view found to attach input/output channel");
            }
        } else {
            warn!(
                "no terminal found with id: {} to attach input/output channel",
                self.terminal_id
            );
        }
    }

    /// Returns the terminal view, creating it if it doesn't exist.
    fn terminal_view(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TerminalView> {
        if let Some(content) = &self.content {
            content.clone()
        } else {
            let (columns, rows, cell_width, line_height) = compute_terminal_dimensions(window);
            let tview = cx.new(|cx| TerminalView::new(columns, rows, cell_width, line_height, cx));
            self.content = Some(tview.clone());

            info!("attach channel to terminal on creation");
            self.attach_channel(cx);

            tview
        }
    }
}

impl Render for WorkspaceTerminal {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = self.terminal_view(window, cx);
        div().flex_1().h_full().w_full().child(content)
    }
}

/// Compute terminal grid dimensions from the current viewport.
/// Returns `(columns, rows, cell_width, line_height)`.
///
/// Calls `load_fonts()` to ensure the monospace font is registered with the
/// GPUI text system before measuring glyph advance width. This is a no-op
/// after the first call — `load_fonts` is idempotent.
pub fn compute_terminal_dimensions(window: &mut Window) -> (usize, usize, Pixels, Pixels) {
    let viewport = window.viewport_size();
    let line_height = px(16.0);

    fonts::load_fonts(window);

    let font = gpui::Font {
        family: fonts::MONO_FONT_FAMILY.into(),
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

    // Subtract chrome (status bar, header, home indicator) so the PTY row count
    // matches what's actually visible, preventing TUI overflow.
    let top_reserved = crate::platform_bridge::status_bar_inset() + theme::HEADER_HEIGHT;
    let bottom_reserved = crate::platform_bridge::home_indicator_inset();
    let terminal_height = viewport.height - px(top_reserved + bottom_reserved);

    let columns = ((viewport.width / cell_width).floor() as usize)
        .saturating_sub(1)
        .clamp(20, 200);
    let rows = ((terminal_height / line_height).floor() as usize)
        .saturating_sub(1)
        .clamp(5, 200);

    (columns, rows, cell_width, line_height)
}
