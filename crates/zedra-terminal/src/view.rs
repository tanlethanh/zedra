// Terminal view - GPUI Render implementation for the terminal
// Manages terminal state, viewport-driven sizing, and rendering of the terminal grid

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use gpui::{prelude::FluentBuilder as _, *};
use tokio::sync::mpsc;
use tracing::*;

use crate::element::TerminalElement;
use crate::terminal::{Terminal, TerminalEvent};

const FALLBACK_CELL_WIDTH: f32 = 9.0;
const TERMINAL_LINE_HEIGHT: f32 = 16.0;

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

pub struct TerminalView {
    terminal_id: String,
    terminal: Entity<Terminal>,
    focus_handle: FocusHandle,
    scroll_offset_px: f32,
    last_remote_size: Option<(u16, u16)>,
    /// Top-left origin of the painted terminal grid within the window.
    /// Used to turn touch scroll positions into terminal cell coordinates.
    grid_origin: Option<Point<Pixels>>,
    workdir: Option<String>,
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
                if let Err(e) = this.update(cx, |_this, cx| {
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
            last_remote_size: None,
            grid_origin: None,
            workdir: None,
            _event_task: event_task,
            _subscriptions: vec![],
        }
    }

    pub fn set_terminal_id(&mut self, terminal_id: String) {
        self.terminal_id = terminal_id;
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
        self.terminal.update(cx, |terminal, _| {
            terminal.scroll(lines);
        });
    }

    pub fn set_grid_origin(&mut self, origin: Point<Pixels>) {
        self.grid_origin = Some(origin);
    }

    pub fn set_workdir(&mut self, workdir: Option<String>) {
        self.workdir = workdir;
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
        let focus_handle = self.focus_handle.clone();

        div()
            .size_full()
            .overflow_hidden()
            .bg(rgb(0x0e0c0c))
            .track_focus(&focus_handle)
            .key_context("Terminal")
            .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                let hyperlink = event.mouse_position().and_then(|position| {
                    this.terminal.read(cx).hyperlink_at(
                        position,
                        this.grid_origin,
                        this.workdir.as_deref(),
                    )
                });
                if let Some(hyperlink) = hyperlink {
                    cx.emit(TerminalEvent::OpenHyperlink(hyperlink));
                    return;
                }
            }))
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _window, cx| {
                match event.delta {
                    ScrollDelta::Lines(l) => {
                        // Line-based scroll (e.g. mouse wheel): commit immediately
                        this.scroll_offset_px = 0.0;
                        let lines = l.y as i32;
                        if lines != 0 {
                            let grid_origin = this.grid_origin;
                            this.terminal.update(cx, |terminal, _| {
                                terminal.commit_scroll_lines(lines, event, grid_origin);
                            });
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
                            let py: f32 = (pixels.y / px(1.0)) as f32;
                            this.scroll_offset_px += py;

                            // Remote terminal scroll should emit small, repeated wheel
                            // ticks while dragging; local scrollback keeps line-based steps.
                            let grid_origin = this.grid_origin;
                            while this.scroll_offset_px >= step_px {
                                let moved = this.terminal.update(cx, |terminal, _| {
                                    terminal.commit_scroll_lines(1, event, grid_origin)
                                });
                                if !moved {
                                    // Hit top of scrollback — clamp offset
                                    this.scroll_offset_px = 0.0;
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
                                if offset >= history && this.scroll_offset_px > 0.0 {
                                    this.scroll_offset_px = 0.0; // at top
                                }
                            }
                        }
                    }
                };
                // Always re-render — sub-line offset changes are visual even without whole-line commits
                cx.notify();
            }))
            .child(TerminalElement::new(
                content,
                size,
                self.scroll_offset_px,
                cx.weak_entity(),
                self.terminal.downgrade(),
                self.focus_handle.clone(),
                self.focus_handle.is_focused(window),
            ))
    }
}
