// Terminal view - GPUI Render implementation for the terminal
// Manages terminal state, handles keyboard input, and renders the terminal grid

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

use gpui::*;

use crate::element::TerminalElement;
use crate::terminal::Terminal;

/// Thread-safe buffer for receiving PTY output.
pub type OutputBuffer = Arc<Mutex<VecDeque<Vec<u8>>>>;

pub struct TerminalView {
    terminal: Entity<Terminal>,
    focus_handle: FocusHandle,
    scroll_offset_px: f32,
    /// Top-left origin of the painted terminal grid within the window.
    /// Used to turn touch scroll positions into terminal cell coordinates.
    grid_origin: Option<Point<Pixels>>,
}

impl TerminalView {
    pub fn new(
        columns: usize,
        rows: usize,
        cell_width: Pixels,
        line_height: Pixels,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            terminal: cx.new(|_cx| Terminal::new(columns, rows, cell_width, line_height)),
            focus_handle: cx.focus_handle(),
            scroll_offset_px: 0.0,
            grid_origin: None,
        }
    }

    pub fn is_channel_attached(&self, cx: &mut Context<Self>) -> bool {
        self.terminal.read(cx).is_channel_attached()
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

    /// Scroll the terminal by line count (positive = up).
    pub fn scroll(&mut self, cx: &mut Context<Self>, lines: i32) {
        self.terminal.update(cx, |terminal, _| {
            terminal.scroll(lines);
        });
    }

    pub fn set_grid_origin(&mut self, origin: Point<Pixels>) {
        self.grid_origin = Some(origin);
    }
}

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
