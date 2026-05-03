// Terminal view - GPUI Render implementation for the terminal
// Manages terminal state, viewport-driven sizing, and rendering of the terminal grid

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use alacritty_terminal::term::TermMode;
use gpui::*;
use tokio::sync::mpsc;
use tracing::*;
use zedra_osc::OscEvent;

use crate::element::TerminalElement;
use crate::terminal::{Terminal, TerminalContent, TerminalEvent};

const FALLBACK_CELL_WIDTH: f32 = 9.0;
const TERMINAL_LINE_HEIGHT: f32 = 16.0;
const TOUCH_SCROLL_SUPPRESSION_AFTER_SCROLL_TO_BOTTOM: Duration = Duration::from_millis(1000);

/// Thread-safe buffer for receiving PTY output.
pub type OutputBuffer = Arc<Mutex<VecDeque<Vec<u8>>>>;

#[derive(Clone, Copy, Debug)]
pub struct TerminalGridSize {
    pub columns: usize,
    pub rows: usize,
    pub cell_width: Pixels,
    pub line_height: Pixels,
}

impl TerminalGridSize {
    fn remote_size(self) -> (u16, u16) {
        (self.columns as u16, self.rows as u16)
    }
}

trait IntoRemoteSize {
    fn into_remote_size(self) -> (u16, u16);
}

impl IntoRemoteSize for crate::terminal::TerminalSize {
    fn into_remote_size(self) -> (u16, u16) {
        (self.columns as u16, self.rows as u16)
    }
}

/// Scans viewport cells to find the last non-blank row, then computes how many
/// pixels to shift the grid upward so that content stays visible above the keyboard.
/// Returns 0.0 when the keyboard is hidden, when the user is in scrollback, or when
/// all content fits above the keyboard edge.
pub fn keyboard_content_offset_px(
    content: &TerminalContent,
    bounds_height: Pixels,
    line_height: Pixels,
    keyboard_inset: Pixels,
) -> f32 {
    if content.mode.contains(TermMode::ALT_SCREEN)
        || keyboard_inset <= px(0.0)
        || content.display_offset != 0
    {
        return 0.0;
    }

    let line_height_f = (line_height / px(1.0)) as f32;
    let keyboard_f = (keyboard_inset / px(1.0)) as f32;
    if line_height_f <= 0.0 || keyboard_f <= 0.0 {
        return 0.0;
    }

    let display_offset = content.display_offset as i32;
    let grid_rows = content.grid_rows as i32;
    let last_nonblank_row = content
        .cells
        .iter()
        .filter_map(|cell| {
            if crate::terminal::is_blank(cell) {
                return None;
            }
            let line = cell.point.line.0 + display_offset;
            (line >= 0 && line < grid_rows).then_some(line as usize)
        })
        .max();

    let Some(bottom_row) = last_nonblank_row else {
        return 0.0;
    };

    // Two buffer rows give visual breathing room and absorb cases where the last
    // row is a plain-space line (indistinguishable from empty via is_blank).
    let content_bottom = (bottom_row as f32 + 1.0 + 2.0) * line_height_f;
    let visible_height = ((bounds_height - keyboard_inset).max(px(0.0)) / px(1.0)) as f32;
    (content_bottom - visible_height).clamp(0.0, keyboard_f)
}

pub struct TerminalView {
    terminal_id: String,
    terminal: Entity<Terminal>,
    focus_handle: FocusHandle,
    scroll_offset_px: f32,
    keyboard_top_reveal_px: f32,
    last_remote_size: Option<(u16, u16)>,
    /// Top-left origin of the painted terminal grid within the window.
    /// Used to turn touch scroll positions into terminal cell coordinates.
    grid_origin: Option<Point<Pixels>>,
    workdir: Option<String>,
    /// Keyboard height in logical pixels. Updated by WorkspaceTerminal via deferred sync.
    /// Used to suppress PTY resize (SIGWINCH) while the keyboard masks the bottom rows.
    pub keyboard_inset: Pixels,
    /// Pre-computed upward shift in pixels. Set by WorkspaceTerminal on keyboard appear;
    /// reset to zero on keyboard dismiss. Applied by TerminalElement during paint.
    pub keyboard_content_offset: Pixels,
    suppress_touch_scroll_until: Option<Instant>,
    /// Cached from terminal mode; updated each render so parent views can read without
    /// creating a GPUI dependency on the inner terminal entity.
    pub is_alt_screen: bool,
    _event_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl TerminalView {
    pub fn new(
        terminal_id: String,
        window: &mut Window,
        viewport: Size<Pixels>,
        cx: &mut Context<Self>,
    ) -> Self {
        let initial_grid_size = TerminalView::compute_grid_size(window, viewport);

        let terminal = cx.new(|_cx| {
            Terminal::new(
                initial_grid_size.columns,
                initial_grid_size.rows,
                initial_grid_size.cell_width,
                initial_grid_size.line_height,
            )
        });

        // Subscribe to terminal events via tokio broadcast channel and emit them to the app
        let mut event_rx = terminal.read(cx).subscribe_events();
        let event_task = cx.spawn(async move |this, cx| {
            while let Ok(event) = event_rx.recv().await {
                if let Err(e) = this.update(cx, |this, cx| {
                    if let TerminalEvent::AltScreenChanged(is_alt) = &event {
                        this.is_alt_screen = *is_alt;
                    }
                    if let TerminalEvent::OscEvent(OscEvent::Cwd(cwd)) = &event {
                        this.workdir = Some(cwd.clone());
                    }
                    cx.emit(event);
                    cx.notify();
                }) {
                    error!("failed to emit terminal event: {:?}", e);
                }
            }
        });

        Self {
            terminal,
            terminal_id,
            focus_handle: cx.focus_handle(),
            scroll_offset_px: 0.0,
            keyboard_top_reveal_px: 0.0,
            last_remote_size: None,
            grid_origin: None,
            workdir: None,
            keyboard_inset: px(0.0),
            keyboard_content_offset: px(0.0),
            suppress_touch_scroll_until: None,
            is_alt_screen: false,
            _event_task: event_task,
            _subscriptions: vec![],
        }
    }

    pub fn set_terminal_id(&mut self, terminal_id: String) {
        self.terminal_id = terminal_id;
    }

    /// Computes the upward pixel shift needed to keep active bottom content visible above
    /// the keyboard. Returns `None` while the user is in scrollback because the renderable
    /// cells then describe the scrollback viewport, not the live bottom viewport.
    pub fn compute_keyboard_content_offset(
        &self,
        keyboard_inset: Pixels,
        cx: &App,
    ) -> Option<Pixels> {
        let terminal = self.terminal.read(cx);
        let content = terminal.content();
        if content.mode.contains(TermMode::ALT_SCREEN) {
            return Some(px(0.0));
        }
        if content.display_offset != 0 {
            return None;
        }
        let size = terminal.size();
        let bounds_height = size.line_height * content.grid_rows;
        Some(px(keyboard_content_offset_px(
            &content,
            bounds_height,
            size.line_height,
            keyboard_inset,
        )))
    }

    pub fn compute_grid_size(window: &mut Window, viewport: Size<Pixels>) -> TerminalGridSize {
        let line_height = px(TERMINAL_LINE_HEIGHT);
        let cell_width = Self::measure_cell_width(window, line_height);
        Self::compute_grid_size_with_metrics(viewport, cell_width, line_height)
    }

    fn measure_cell_width(window: &mut Window, line_height: Pixels) -> Pixels {
        let font = Font {
            family: crate::MONO_FONT_FAMILY.into(),
            features: FontFeatures::default(),
            fallbacks: None,
            weight: FontWeight::NORMAL,
            style: FontStyle::Normal,
        };
        let font_size = line_height * 0.75;
        let text_system = window.text_system();
        let font_id = text_system.resolve_font(&font);
        text_system
            .advance(font_id, font_size, 'm')
            .map(|size| size.width)
            .unwrap_or(px(FALLBACK_CELL_WIDTH))
    }

    fn compute_grid_size_with_metrics(
        viewport: Size<Pixels>,
        cell_width: Pixels,
        line_height: Pixels,
    ) -> TerminalGridSize {
        let width = viewport.width.max(px(0.0));
        let height = viewport.height.max(px(0.0));
        let columns = (width / cell_width).floor() as usize;
        let rows = (height / line_height).floor() as usize;

        TerminalGridSize {
            columns,
            rows,
            cell_width,
            line_height,
        }
    }

    pub fn is_channel_attached(&self, cx: &mut Context<Self>) -> bool {
        self.terminal.read(cx).is_channel_attached()
    }

    pub fn input_sender(&self, cx: &App) -> Option<tokio::sync::mpsc::Sender<Vec<u8>>> {
        self.terminal.read(cx).input_sender()
    }

    pub fn attach_channel(
        &mut self,
        input_tx: mpsc::Sender<Vec<u8>>,
        output_rx: mpsc::Receiver<Vec<u8>>,
        cx: &mut Context<Self>,
    ) {
        self.terminal.update(cx, |terminal, cx| {
            terminal.attach_channel(input_tx, output_rx, cx);
        });
    }

    pub fn remote_size(&self, cx: &App) -> (u16, u16) {
        self.terminal.read(cx).size().into_remote_size()
    }

    /// Reattach may not produce a paint mismatch, so sync the host PTY once.
    pub fn sync_remote_size_after_attach(&mut self, cx: &mut Context<Self>) {
        let remote_size = self.terminal.read(cx).size().into_remote_size();
        self.last_remote_size = Some(remote_size);
        cx.emit(TerminalEvent::RequestResize {
            cols: remote_size.0,
            rows: remote_size.1,
        });
    }

    /// This is called by TerminalElement when the actual bounds of the terminal
    /// do not match the expected bounds.
    pub(crate) fn reconcile_bounds_fallback(
        &mut self,
        actual_bounds: Size<Pixels>,
        cell_width: Pixels,
        line_height: Pixels,
        cx: &mut Context<Self>,
    ) {
        let next = Self::compute_grid_size_with_metrics(actual_bounds, cell_width, line_height);
        self.apply_grid_size(next, cx);
    }

    fn apply_grid_size(&mut self, next: TerminalGridSize, cx: &mut Context<Self>) {
        let size = self.terminal.read(cx).size();
        let changed = size.columns != next.columns
            || size.rows != next.rows
            || size.cell_width != next.cell_width
            || size.line_height != next.line_height;

        if changed {
            let terminal_id = self.terminal_id.clone();
            self.terminal.update(cx, |terminal, _cx| {
                info!(
                    terminal_id,
                    columns = next.columns,
                    rows = next.rows,
                    "terminal grid resized"
                );
                terminal.resize(next.columns, next.rows, next.cell_width, next.line_height);
            });
            cx.notify();
        }

        if !changed && self.last_remote_size == Some(next.remote_size()) {
            return;
        }

        self.resize_remote_pty(next, cx);
    }

    fn resize_remote_pty(&mut self, next: TerminalGridSize, cx: &mut Context<Self>) {
        let remote_size = next.remote_size();
        if self.last_remote_size == Some(remote_size) {
            return;
        }

        self.last_remote_size = Some(remote_size);
        cx.emit(TerminalEvent::RequestResize {
            cols: remote_size.0,
            rows: remote_size.1,
        });
    }

    /// Scroll the terminal by line count (positive = up).
    pub fn scroll(&mut self, cx: &mut Context<Self>, lines: i32) {
        let previous_display_offset = self.display_offset(cx);
        self.terminal.update(cx, |terminal, _| {
            terminal.scroll(lines);
        });
        self.reset_keyboard_top_reveal_if_not_at_top(cx);
        self.emit_scrollback_position_if_changed(previous_display_offset, cx);
        cx.notify();
    }

    pub fn scroll_to_bottom(&mut self, cx: &mut Context<Self>) {
        let previous_display_offset = self.display_offset(cx);
        self.scroll_offset_px = 0.0;
        self.keyboard_top_reveal_px = 0.0;
        self.suppress_touch_scroll_until =
            Some(Instant::now() + TOUCH_SCROLL_SUPPRESSION_AFTER_SCROLL_TO_BOTTOM);
        self.terminal.update(cx, |terminal, _| {
            terminal.scroll_to_bottom();
        });
        self.emit_scrollback_position_if_changed(previous_display_offset, cx);
        cx.notify();
    }

    fn keyboard_top_reveal_limit_px(&self) -> f32 {
        if self.keyboard_content_offset <= px(0.0) || self.is_alt_screen {
            return 0.0;
        }

        ((self.keyboard_content_offset / px(1.0)) as f32).max(0.0)
    }

    fn effective_keyboard_top_reveal_px(
        &self,
        content: &TerminalContent,
        history_size: usize,
    ) -> f32 {
        if content.display_offset == 0
            || content.display_offset < history_size
            || content.mode.contains(TermMode::ALT_SCREEN)
        {
            return 0.0;
        }

        self.keyboard_top_reveal_px
            .clamp(0.0, self.keyboard_top_reveal_limit_px())
    }

    fn add_keyboard_top_reveal_px(&mut self, delta_px: f32) {
        let limit = self.keyboard_top_reveal_limit_px();
        if delta_px <= 0.0 || limit <= 0.0 {
            return;
        }

        self.keyboard_top_reveal_px =
            (self.keyboard_top_reveal_px.min(limit) + delta_px).min(limit);
    }

    fn consume_keyboard_top_reveal_px(&mut self, delta_px: f32) -> f32 {
        let limit = self.keyboard_top_reveal_limit_px();
        if delta_px <= 0.0 || limit <= 0.0 {
            self.keyboard_top_reveal_px = 0.0;
            return delta_px.max(0.0);
        }

        self.keyboard_top_reveal_px = self.keyboard_top_reveal_px.min(limit);
        let consumed = self.keyboard_top_reveal_px.min(delta_px);
        self.keyboard_top_reveal_px -= consumed;
        delta_px - consumed
    }

    fn reset_keyboard_top_reveal_if_not_at_top(&mut self, cx: &App) {
        let terminal = self.terminal.read(cx);
        let display_offset = terminal.display_offset();
        let history_size = terminal.history_size();
        if display_offset == 0 || display_offset < history_size {
            self.keyboard_top_reveal_px = 0.0;
        }
    }

    fn emit_scrollback_position_if_changed(
        &self,
        previous_display_offset: usize,
        cx: &mut Context<Self>,
    ) {
        let terminal = self.terminal.read(cx);
        let display_offset = terminal.display_offset();
        if display_offset == previous_display_offset {
            return;
        }

        let history_size = terminal.history_size();
        cx.emit(TerminalEvent::ScrollbackPositionChanged {
            display_offset,
            history_size,
        });
    }

    fn should_ignore_touch_scroll(&mut self, event: &ScrollWheelEvent) -> bool {
        if !matches!(event.delta, ScrollDelta::Pixels(_)) {
            return false;
        }

        if matches!(event.touch_phase, TouchPhase::Started) {
            self.suppress_touch_scroll_until = None;
            return false;
        }

        let Some(until) = self.suppress_touch_scroll_until else {
            return false;
        };

        if Instant::now() >= until {
            self.suppress_touch_scroll_until = None;
            return false;
        }

        self.scroll_offset_px = 0.0;
        true
    }

    pub fn display_offset(&self, cx: &App) -> usize {
        self.terminal.read(cx).display_offset()
    }

    pub fn set_grid_origin(&mut self, origin: Point<Pixels>) {
        self.grid_origin = Some(origin);
    }

    pub fn set_workdir(&mut self, workdir: Option<String>) {
        self.workdir = workdir;
    }

    fn handle_terminal_press(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let hyperlink = self.terminal.read(cx).hyperlink_at(
            position,
            self.grid_origin,
            self.workdir.as_deref(),
        );
        if let Some(hyperlink) = hyperlink {
            cx.emit(TerminalEvent::OpenHyperlink(hyperlink));
            return;
        }

        let is_focused = self.focus_handle.is_focused(window);
        let keyboard_visible = window.is_soft_keyboard_visible();

        window.prevent_default();

        if is_focused && keyboard_visible {
            // Must explicitly hide the keyboard, window.blur only blurs the focus, not the keyboard.
            window.hide_soft_keyboard();
            window.blur();
            cx.notify();
        } else if is_focused {
            // Keyboard might not be visible when already focused.
            // It does nothing if the keyboard is already visible.
            window.show_soft_keyboard();
            cx.notify();
        } else if !is_focused {
            self.focus_handle.focus(window, cx);
            window.show_soft_keyboard();
            cx.notify();
        }
    }
}

impl EventEmitter<TerminalEvent> for TerminalView {}

impl Focusable for TerminalView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let terminal = self.terminal.read(cx);
        let content = terminal.content();
        let size = terminal.size();
        let history_size = terminal.history_size();
        let focus_handle = self.focus_handle.clone();
        let visual_scroll_offset_px =
            self.scroll_offset_px + self.effective_keyboard_top_reveal_px(&content, history_size);

        div()
            .id("terminal-view")
            .key_context("Terminal")
            .size_full()
            .overflow_hidden()
            .bg(rgb(0x0e0c0c))
            .track_focus(&focus_handle)
            .manual_focus()
            .on_press(cx.listener(|this, event: &PressEvent, window, cx| {
                this.handle_terminal_press(event.position(), window, cx);
            }))
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _window, cx| {
                let previous_display_offset = this.display_offset(cx);
                if this.should_ignore_touch_scroll(event) {
                    cx.notify();
                    return;
                }

                match event.delta {
                    ScrollDelta::Lines(l) => {
                        // Line-based scroll (e.g. mouse wheel): commit immediately
                        this.scroll_offset_px = 0.0;
                        let mut lines = l.y as i32;
                        if lines < 0 {
                            let step_px =
                                (this.terminal.read(cx).size().line_height / px(1.0)) as f32;
                            let remaining_px =
                                this.consume_keyboard_top_reveal_px((-lines) as f32 * step_px);
                            lines = if remaining_px > 0.0 {
                                -((remaining_px / step_px).ceil() as i32)
                            } else {
                                0
                            };
                        }
                        if lines != 0 {
                            let grid_origin = this.grid_origin;
                            let moved = this.terminal.update(cx, |terminal, _| {
                                terminal.commit_scroll_lines(lines, event, grid_origin)
                            });
                            let (display_offset, history_size, step_px) = {
                                let terminal = this.terminal.read(cx);
                                (
                                    terminal.display_offset(),
                                    terminal.history_size(),
                                    (terminal.size().line_height / px(1.0)) as f32,
                                )
                            };
                            if lines > 0 && !moved && display_offset > 0 {
                                this.add_keyboard_top_reveal_px(lines as f32 * step_px);
                            } else if display_offset == 0 || display_offset < history_size {
                                this.keyboard_top_reveal_px = 0.0;
                            }
                        }
                    }
                    ScrollDelta::Pixels(pixels) => {
                        if matches!(event.touch_phase, TouchPhase::Ended) {
                            let (snap, step_px) = {
                                let t = this.terminal.read(cx);
                                (t.should_snap_touch_release(event), t.scroll_step_px(event))
                            };
                            if snap && this.scroll_offset_px.abs() > step_px * 0.5 {
                                // Local scrollback benefits from snapping the partial drag
                                // to the nearest line, but remote TUIs should emit while
                                // dragging instead of waiting for finger lift.
                                let lines = if this.scroll_offset_px > 0.0 { 1 } else { -1 };
                                let grid_origin = this.grid_origin;
                                this.terminal.update(cx, |terminal, _| {
                                    terminal.commit_scroll_lines(lines, event, grid_origin);
                                });
                            }
                            this.scroll_offset_px = 0.0;
                        } else {
                            let step_px = this.terminal.read(cx).scroll_step_px(event);
                            let mut py: f32 = (pixels.y / px(1.0)) as f32;
                            if py < 0.0 {
                                py = -this.consume_keyboard_top_reveal_px(-py);
                            }
                            this.scroll_offset_px += py;

                            // Remote terminal scroll should emit small, repeated wheel
                            // ticks while dragging; local scrollback keeps line-based steps.
                            let grid_origin = this.grid_origin;
                            while this.scroll_offset_px >= step_px {
                                let moved = this.terminal.update(cx, |terminal, _| {
                                    terminal.commit_scroll_lines(1, event, grid_origin)
                                });
                                if !moved {
                                    // Keep the keyboard lift scrollable at the top boundary so
                                    // the oldest rows can be revealed instead of clipped.
                                    break;
                                }
                                this.scroll_offset_px -= step_px;
                            }
                            while this.scroll_offset_px <= -step_px {
                                let moved = this.terminal.update(cx, |terminal, _| {
                                    terminal.commit_scroll_lines(-1, event, grid_origin)
                                });
                                if !moved {
                                    // Hit bottom of scrollback — clamp offset
                                    this.scroll_offset_px = 0.0;
                                    break;
                                }
                                this.scroll_offset_px += step_px;
                            }

                            // Local scrollback clamps at the history bounds, but alt-screen
                            // scroll should keep producing cursor-up/down bytes for the PTY.
                            let (alt_scroll, offset, history) = {
                                let t = this.terminal.read(cx);
                                (
                                    t.should_send_alt_scroll(event),
                                    t.display_offset(),
                                    t.history_size(),
                                )
                            };
                            if !alt_scroll {
                                if offset == 0 && this.scroll_offset_px < 0.0 {
                                    this.scroll_offset_px = 0.0; // at bottom
                                }
                                if offset == 0 || offset < history {
                                    this.keyboard_top_reveal_px = 0.0;
                                } else if offset >= history && this.scroll_offset_px > 0.0 {
                                    this.add_keyboard_top_reveal_px(this.scroll_offset_px);
                                    this.scroll_offset_px = 0.0; // at top
                                }
                            }
                        }
                    }
                };
                this.emit_scrollback_position_if_changed(previous_display_offset, cx);
                // Always re-render — sub-line offset changes are visual even without whole-line commits
                cx.notify();
            }))
            .child(TerminalElement::new(
                content,
                size,
                visual_scroll_offset_px,
                self.keyboard_inset,
                self.keyboard_content_offset,
                cx.weak_entity(),
                self.terminal.downgrade(),
                self.focus_handle.clone(),
                self.focus_handle.is_focused(window),
            ))
    }
}

#[cfg(test)]
mod tests {
    use super::{TerminalView, keyboard_content_offset_px};
    use std::path::Path;

    use crate::terminal::{Terminal, TerminalContent, TerminalEvent, TerminalHyperlinkTarget};
    use alacritty_terminal::term::TermMode;
    use futures::{FutureExt as _, StreamExt as _};
    use gpui::{
        Modifiers, Pixels, Point, PointerButton, PointerDownEvent, PointerKind, PointerUpEvent,
        TestAppContext, TouchPhase, VisualTestContext, WindowHandle, point, px, size,
    };

    fn open_terminal_window(cx: &mut TestAppContext) -> WindowHandle<TerminalView> {
        cx.open_window(size(px(320.0), px(240.0)), |window, cx| {
            TerminalView::new("term-1".to_string(), window, size(px(320.0), px(240.0)), cx)
        })
    }

    fn tap_terminal(window: WindowHandle<TerminalView>, cx: &mut TestAppContext) {
        tap_terminal_at(window, cx, point(px(12.0), px(12.0)));
    }

    fn tap_terminal_at(
        window: WindowHandle<TerminalView>,
        cx: &mut TestAppContext,
        position: Point<Pixels>,
    ) {
        let mut window_cx = VisualTestContext::from_window(*window, cx);
        window_cx.simulate_event(PointerDownEvent {
            pointer_id: 1,
            kind: PointerKind::Touch,
            is_primary: true,
            button: PointerButton::Primary,
            position,
            modifiers: Modifiers::default(),
        });
        window_cx.simulate_event(PointerUpEvent {
            pointer_id: 1,
            kind: PointerKind::Touch,
            is_primary: true,
            button: PointerButton::Primary,
            position,
            modifiers: Modifiers::default(),
        });
    }

    fn pointer_down_terminal(window: WindowHandle<TerminalView>, cx: &mut TestAppContext) {
        let mut window_cx = VisualTestContext::from_window(*window, cx);
        window_cx.simulate_event(PointerDownEvent {
            pointer_id: 1,
            kind: PointerKind::Touch,
            is_primary: true,
            button: PointerButton::Primary,
            position: point(px(12.0), px(12.0)),
            modifiers: Modifiers::default(),
        });
    }

    fn scroll_terminal_touch(window: WindowHandle<TerminalView>, cx: &mut TestAppContext) {
        scroll_terminal_touch_with_phase(window, cx, TouchPhase::Moved);
    }

    fn scroll_terminal_touch_with_phase(
        window: WindowHandle<TerminalView>,
        cx: &mut TestAppContext,
        touch_phase: TouchPhase,
    ) {
        scroll_terminal_touch_delta(window, cx, px(16.0), touch_phase);
    }

    fn scroll_terminal_touch_delta(
        window: WindowHandle<TerminalView>,
        cx: &mut TestAppContext,
        delta_y: Pixels,
        touch_phase: TouchPhase,
    ) {
        let mut window_cx = VisualTestContext::from_window(*window, cx);
        window_cx.simulate_event(gpui::ScrollWheelEvent {
            position: point(px(12.0), px(28.0)),
            delta: gpui::ScrollDelta::Pixels(point(px(0.0), delta_y)),
            modifiers: Modifiers::default(),
            touch_phase,
        });
    }

    fn fill_terminal_history(window: WindowHandle<TerminalView>, cx: &mut TestAppContext) {
        window
            .update(cx, |terminal_view, _window, cx| {
                terminal_view.terminal.update(cx, |terminal, _| {
                    for line in 0..40 {
                        terminal.advance_bytes(format!("line {line}\r\n").as_bytes());
                    }
                    terminal.scroll(5);
                });
                assert!(terminal_view.terminal.read(cx).display_offset() > 0);
            })
            .unwrap();
    }

    #[test]
    fn osc_cwd_updates_relative_hyperlink_workdir() {
        let mut cx = TestAppContext::single();
        let window = open_terminal_window(&mut cx);
        cx.run_until_parked();

        let output =
            b"\x1b]7;file:///repo/sub\x1b\\Open \x1b]8;;src/main.rs:12:3\x1b\\source\x1b]8;;\x1b\\ now\r\n";
        window
            .update(&mut cx, |terminal_view, _window, cx| {
                terminal_view.set_grid_origin(point(px(0.0), px(0.0)));
                terminal_view.terminal.update(cx, |terminal, _| {
                    terminal.advance_bytes(output);
                    terminal.feed_osc_bytes(output);
                });
            })
            .unwrap();
        cx.run_until_parked();

        let root = window.root(&mut cx).unwrap();
        let mut events = cx.events(&root);
        let target_position = window
            .update(&mut cx, |terminal_view, _window, cx| {
                let size = terminal_view.terminal.read(cx).size();
                point(size.cell_width * 7.0, size.line_height / 2.0)
            })
            .unwrap();

        tap_terminal_at(window, &mut cx, target_position);
        cx.run_until_parked();

        match events.next().now_or_never().flatten() {
            Some(TerminalEvent::OpenHyperlink(hyperlink)) => match hyperlink.target {
                TerminalHyperlinkTarget::File {
                    path,
                    relative_path,
                    line,
                    column,
                } => {
                    assert_eq!(Path::new(&path), Path::new("/repo/sub/src/main.rs"));
                    assert_eq!(Path::new(&relative_path), Path::new("src/main.rs"));
                    assert_eq!(line, Some(12));
                    assert_eq!(column, Some(3));
                }
                target => panic!("expected file hyperlink, got {target:?}"),
            },
            event => panic!("expected hyperlink event, got {event:?}"),
        }
        cx.quit();
    }

    #[test]
    fn sync_remote_size_after_attach_forces_resize_event() {
        let mut cx = TestAppContext::single();
        let window = open_terminal_window(&mut cx);
        cx.run_until_parked();

        let root = window.root(&mut cx).unwrap();
        let mut events = cx.events(&root);
        let expected_size = window
            .update(&mut cx, |terminal_view, _window, cx| {
                let size = terminal_view.terminal.read(cx).size();
                let expected_size = (size.columns as u16, size.rows as u16);
                terminal_view.last_remote_size = Some(expected_size);
                terminal_view.sync_remote_size_after_attach(cx);
                expected_size
            })
            .unwrap();

        match events.next().now_or_never().flatten() {
            Some(TerminalEvent::RequestResize { cols, rows }) => {
                assert_eq!((cols, rows), expected_size);
            }
            event => panic!("expected forced resize event, got {event:?}"),
        }
        cx.quit();
    }

    #[test]
    fn terminal_pointer_down_does_not_focus_before_completed_press() {
        let mut cx = TestAppContext::single();
        let window = open_terminal_window(&mut cx);
        cx.run_until_parked();

        pointer_down_terminal(window, &mut cx);
        cx.run_until_parked();

        window
            .update(&mut cx, |terminal, window, _| {
                assert!(!terminal.focus_handle.is_focused(window));
                assert!(!window.is_soft_keyboard_visible());
            })
            .unwrap();
        cx.quit();
    }

    #[test]
    fn unfocused_terminal_tap_focuses_and_requests_keyboard() {
        let mut cx = TestAppContext::single();
        let window = open_terminal_window(&mut cx);
        cx.run_until_parked();

        tap_terminal(window, &mut cx);
        cx.run_until_parked();

        window
            .update(&mut cx, |terminal, window, _| {
                assert!(terminal.focus_handle.is_focused(window));
                assert!(window.is_soft_keyboard_visible());
            })
            .unwrap();
        cx.quit();
    }

    #[test]
    fn focused_terminal_tap_hides_keyboard_and_blurs() {
        let mut cx = TestAppContext::single();
        let window = open_terminal_window(&mut cx);
        cx.run_until_parked();

        tap_terminal(window, &mut cx);
        cx.run_until_parked();
        tap_terminal(window, &mut cx);
        cx.run_until_parked();

        window
            .update(&mut cx, |terminal, window, _| {
                assert!(!terminal.focus_handle.is_focused(window));
                assert!(!window.is_soft_keyboard_visible());
            })
            .unwrap();
        cx.quit();
    }

    #[test]
    fn focused_terminal_tap_requests_keyboard_when_keyboard_is_hidden() {
        let mut cx = TestAppContext::single();
        let window = open_terminal_window(&mut cx);
        cx.run_until_parked();

        tap_terminal(window, &mut cx);
        cx.run_until_parked();
        window
            .update(&mut cx, |terminal, window, _| {
                assert!(terminal.focus_handle.is_focused(window));
                window.hide_soft_keyboard();
            })
            .unwrap();

        tap_terminal(window, &mut cx);
        cx.run_until_parked();

        window
            .update(&mut cx, |terminal, window, _| {
                assert!(terminal.focus_handle.is_focused(window));
                assert!(window.is_soft_keyboard_visible());
            })
            .unwrap();
        cx.quit();
    }

    #[test]
    fn touch_scroll_does_not_toggle_keyboard() {
        let mut cx = TestAppContext::single();
        let window = open_terminal_window(&mut cx);
        cx.run_until_parked();

        tap_terminal(window, &mut cx);
        cx.run_until_parked();

        scroll_terminal_touch(window, &mut cx);
        cx.run_until_parked();

        window
            .update(&mut cx, |terminal, window, _| {
                assert!(terminal.focus_handle.is_focused(window));
                assert!(window.is_soft_keyboard_visible());
            })
            .unwrap();
        cx.quit();
    }

    #[test]
    fn scroll_to_bottom_suppresses_in_flight_touch_momentum() {
        let mut cx = TestAppContext::single();
        let window = open_terminal_window(&mut cx);
        cx.run_until_parked();

        fill_terminal_history(window, &mut cx);
        window
            .update(&mut cx, |terminal_view, _window, cx| {
                terminal_view.scroll_to_bottom(cx);
                assert_eq!(terminal_view.terminal.read(cx).display_offset(), 0);
            })
            .unwrap();
        cx.run_until_parked();

        scroll_terminal_touch(window, &mut cx);
        cx.run_until_parked();

        window
            .update(&mut cx, |terminal_view, _window, cx| {
                assert_eq!(terminal_view.terminal.read(cx).display_offset(), 0);
            })
            .unwrap();
        cx.quit();
    }

    #[test]
    fn touch_scroll_emits_scrollback_position_from_view() {
        let mut cx = TestAppContext::single();
        let window = open_terminal_window(&mut cx);
        cx.run_until_parked();

        fill_terminal_history(window, &mut cx);
        window
            .update(&mut cx, |terminal_view, _window, cx| {
                terminal_view.scroll_to_bottom(cx);
                assert_eq!(terminal_view.terminal.read(cx).display_offset(), 0);
            })
            .unwrap();
        cx.run_until_parked();

        let root = window.root(&mut cx).unwrap();
        let mut events = cx.events(&root);
        scroll_terminal_touch_with_phase(window, &mut cx, TouchPhase::Started);
        cx.run_until_parked();

        window
            .update(&mut cx, |terminal_view, _window, cx| {
                assert!(terminal_view.display_offset(cx) > 0);
            })
            .unwrap();

        match events.next().now_or_never().flatten() {
            Some(TerminalEvent::ScrollbackPositionChanged { display_offset, .. }) => {
                assert!(display_offset > 0);
            }
            event => panic!("expected synchronous scrollback event, got {event:?}"),
        }
        cx.quit();
    }

    #[test]
    fn keyboard_content_offset_update_is_skipped_while_in_scrollback() {
        let mut cx = TestAppContext::single();
        let window = open_terminal_window(&mut cx);
        cx.run_until_parked();

        window
            .update(&mut cx, |terminal_view, _window, cx| {
                terminal_view.terminal.update(cx, |terminal, _| {
                    for line in 0..30 {
                        terminal.advance_bytes(format!("line {line}\r\n").as_bytes());
                    }
                });
                assert_eq!(
                    terminal_view.compute_keyboard_content_offset(px(80.0), cx),
                    Some(px(80.0))
                );

                terminal_view.terminal.update(cx, |terminal, _| {
                    terminal.scroll(3);
                });
                assert!(terminal_view.display_offset(cx) > 0);
                assert_eq!(
                    terminal_view.compute_keyboard_content_offset(px(80.0), cx),
                    None
                );
            })
            .unwrap();
        cx.quit();
    }

    #[test]
    fn keyboard_top_reveal_exposes_oldest_rows_at_scrollback_top() {
        let mut cx = TestAppContext::single();
        let window = open_terminal_window(&mut cx);
        cx.run_until_parked();

        window
            .update(&mut cx, |terminal_view, _window, cx| {
                terminal_view.terminal.update(cx, |terminal, _| {
                    for line in 0..40 {
                        terminal.advance_bytes(format!("line {line}\r\n").as_bytes());
                    }
                    terminal.scroll(10_000);
                });
                let terminal = terminal_view.terminal.read(cx);
                assert_eq!(terminal.display_offset(), terminal.history_size());
                terminal_view.keyboard_content_offset = px(80.0);
            })
            .unwrap();

        for _ in 0..5 {
            scroll_terminal_touch_delta(window, &mut cx, px(16.0), TouchPhase::Moved);
            cx.run_until_parked();
        }

        window
            .update(&mut cx, |terminal_view, _window, cx| {
                let terminal = terminal_view.terminal.read(cx);
                assert_eq!(terminal.display_offset(), terminal.history_size());
                assert_eq!(terminal_view.keyboard_top_reveal_px, 80.0);
            })
            .unwrap();
        cx.quit();
    }

    #[test]
    fn keyboard_top_reveal_is_consumed_before_scrolling_down() {
        let mut cx = TestAppContext::single();
        let window = open_terminal_window(&mut cx);
        cx.run_until_parked();

        let top_offset = window
            .update(&mut cx, |terminal_view, _window, cx| {
                terminal_view.terminal.update(cx, |terminal, _| {
                    for line in 0..40 {
                        terminal.advance_bytes(format!("line {line}\r\n").as_bytes());
                    }
                    terminal.scroll(10_000);
                });
                terminal_view.keyboard_content_offset = px(80.0);
                terminal_view.keyboard_top_reveal_px = 80.0;
                terminal_view.terminal.read(cx).display_offset()
            })
            .unwrap();

        for _ in 0..5 {
            scroll_terminal_touch_delta(window, &mut cx, px(-16.0), TouchPhase::Moved);
            cx.run_until_parked();
        }

        window
            .update(&mut cx, |terminal_view, _window, cx| {
                assert_eq!(terminal_view.keyboard_top_reveal_px, 0.0);
                assert_eq!(terminal_view.terminal.read(cx).display_offset(), top_offset);
            })
            .unwrap();

        scroll_terminal_touch_delta(window, &mut cx, px(-16.0), TouchPhase::Moved);
        cx.run_until_parked();

        window
            .update(&mut cx, |terminal_view, _window, cx| {
                assert!(terminal_view.terminal.read(cx).display_offset() < top_offset);
            })
            .unwrap();
        cx.quit();
    }

    #[test]
    fn fresh_touch_scroll_cancels_scroll_to_bottom_suppression() {
        let mut cx = TestAppContext::single();
        let window = open_terminal_window(&mut cx);
        cx.run_until_parked();

        fill_terminal_history(window, &mut cx);
        window
            .update(&mut cx, |terminal_view, _window, cx| {
                terminal_view.scroll_to_bottom(cx);
                assert_eq!(terminal_view.terminal.read(cx).display_offset(), 0);
            })
            .unwrap();
        cx.run_until_parked();

        scroll_terminal_touch_with_phase(window, &mut cx, TouchPhase::Started);
        cx.run_until_parked();

        window
            .update(&mut cx, |terminal_view, _window, cx| {
                assert!(terminal_view.terminal.read(cx).display_offset() > 0);
            })
            .unwrap();
        cx.quit();
    }

    fn content_for_keyboard_offset(output: &[u8]) -> TerminalContent {
        let mut terminal = Terminal::new(20, 10, px(10.0), px(20.0));
        terminal.advance_bytes(output);
        terminal.content()
    }

    fn keyboard_offset_for_content(content: &TerminalContent) -> f32 {
        keyboard_content_offset_px(content, px(200.0), px(20.0), px(80.0))
    }

    #[test]
    fn keyboard_offset_stays_zero_while_content_is_above_keyboard_edge() {
        let content = content_for_keyboard_offset(b"one\r\ntwo\r\nthree\r\n");
        assert_eq!(keyboard_offset_for_content(&content), 0.0);
    }

    #[test]
    fn keyboard_offset_lifts_to_show_last_nonblank_row_with_buffer() {
        // 7 lines + trailing \r\n → last non-blank row is 6 ("seven").
        // content_bottom = (6+1+2)*20 = 180px, visible = 120px, lift = 60px.
        let content =
            content_for_keyboard_offset(b"one\r\ntwo\r\nthree\r\nfour\r\nfive\r\nsix\r\nseven\r\n");
        assert_eq!(keyboard_offset_for_content(&content), 60.0);
    }

    #[test]
    fn keyboard_offset_uses_full_keyboard_inset_when_content_fills_grid() {
        // 10 lines without trailing newline → last non-blank row is 9.
        // content_bottom = (9+1+2)*20 = 240px, visible = 120px, lift = 80px (capped).
        let content = content_for_keyboard_offset(
            b"one\r\ntwo\r\nthree\r\nfour\r\nfive\r\nsix\r\nseven\r\neight\r\nnine\r\nten",
        );
        assert_eq!(keyboard_offset_for_content(&content), 80.0);
    }

    #[test]
    fn keyboard_offset_lifts_for_visible_character_at_bottom() {
        // ESC[10;1H moves cursor to row 9, write a non-blank character → last non-blank = 9.
        let content = content_for_keyboard_offset(b"\x1b[10;1Ha");
        assert_eq!(keyboard_offset_for_content(&content), 80.0);
    }

    #[test]
    fn keyboard_offset_ignores_retained_scrollback_after_clear() {
        let mut terminal = Terminal::new(20, 10, px(10.0), px(20.0));
        terminal.advance_bytes(
            b"one\r\ntwo\r\nthree\r\nfour\r\nfive\r\nsix\r\nseven\r\neight\r\nnine\r\nten\r\n\x1b[2J\x1b[Htop\r\n",
        );
        assert!(terminal.history_size() > 0);
        assert_eq!(keyboard_offset_for_content(&terminal.content()), 0.0);
    }

    #[test]
    fn keyboard_offset_ignores_manual_scrollback_and_alt_screen() {
        let mut content =
            content_for_keyboard_offset(b"one\r\ntwo\r\nthree\r\nfour\r\nfive\r\nsix\r\nseven\r\n");

        content.display_offset = 1;
        assert_eq!(keyboard_offset_for_content(&content), 0.0);

        content.display_offset = 0;
        content.mode = TermMode::ALT_SCREEN;
        assert_eq!(keyboard_offset_for_content(&content), 0.0);
    }
}
