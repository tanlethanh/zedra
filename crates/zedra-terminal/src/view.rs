// Terminal view - GPUI Render implementation for the terminal
// Manages terminal state, handles keyboard input, and renders the terminal grid

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};

use gpui::*;

use crate::element::TerminalElement;
use crate::{TerminalSize, TerminalState};

/// Callback for sending bytes to the remote PTY.
pub type SendBytesFn = Box<dyn Fn(Vec<u8>) + Send + 'static>;

/// Thread-safe buffer for receiving PTY output.
pub type OutputBuffer = Arc<Mutex<VecDeque<Vec<u8>>>>;

/// Callback for requesting keyboard show/hide.
pub type KeyboardRequestFn = Box<dyn Fn(bool) + Send + 'static>;

/// Callback to query whether the soft keyboard is currently visible.
/// Used to sync local toggle state with actual UIKit/Android state.
pub type IsKeyboardVisibleFn = Box<dyn Fn() -> bool + Send + 'static>;

/// Callback invoked when the terminal grid is resized (columns × rows).
/// Lets the caller relay the new size to the remote PTY without the terminal
/// view knowing anything about sessions or RPC.
pub type ResizeFn = Box<dyn Fn(u16, u16) + Send + 'static>;

/// Event emitted when user requests disconnect
pub struct DisconnectRequested;

/// Terminal view that implements GPUI's Render trait.
///
/// Self-contained: it holds its own `TerminalState` (the emulator grid),
/// an `OutputBuffer` to drain each frame, and three optional callbacks:
///   - `send_bytes` — forward keystroke bytes to the backend PTY
///   - `keyboard_request` — show/hide the soft keyboard
///   - `resize_fn` — notify the backend when the grid dimensions change
///
/// The view knows nothing about sessions, RPC, or global context. All
/// backend wiring is the caller's responsibility.
pub struct TerminalView {
    terminal: TerminalState,
    send_bytes: Option<SendBytesFn>,
    output_buffer: OutputBuffer,
    /// Set when wired to a `RemoteTerminal`; gates `process_output()` on each frame.
    needs_render: Option<Arc<AtomicBool>>,
    connected: bool,
    /// Human-readable status (e.g. "Connected", "Resuming"). Not yet rendered;
    /// future: display as a translucent overlay when `!connected`.
    status_text: String,
    focus_handle: FocusHandle,
    keyboard_request: Option<KeyboardRequestFn>,
    is_keyboard_visible_fn: Option<IsKeyboardVisibleFn>,
    /// Called when the effective grid size changes (keyboard resize or viewport change).
    /// Receives `(cols, rows)` so the backend can relay the new PTY size.
    resize_fn: Option<ResizeFn>,
    /// Sub-line pixel offset for smooth scrolling. Applied as a visual shift
    /// to the terminal grid; when it exceeds line_height, a whole line is committed.
    scroll_offset_px: f32,
    /// Row count without keyboard (the "full" terminal height)
    base_rows: usize,
    /// Last keyboard-adjusted row count to avoid redundant resizes
    last_keyboard_rows: usize,
    /// Position recorded on mouse_down; cleared on mouse_up.
    /// Keyboard show fires in mouse_up only when the finger displacement
    /// is within tap slop (i.e. the gesture was a tap, not a swipe).
    mouse_down_pos: Option<Point<Pixels>>,
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
            terminal: TerminalState::new(columns, rows, cell_width, line_height),
            send_bytes: None,
            output_buffer: Arc::new(Mutex::new(VecDeque::new())),
            needs_render: None,
            connected: false,
            status_text: "Disconnected".to_string(),
            focus_handle: cx.focus_handle(),
            keyboard_request: None,
            is_keyboard_visible_fn: None,
            resize_fn: None,
            scroll_offset_px: 0.0,
            base_rows: rows,
            last_keyboard_rows: rows,
            mouse_down_pos: None,
        }
    }

    /// Set callback to request keyboard show/hide
    pub fn set_keyboard_request(&mut self, callback: KeyboardRequestFn) {
        self.keyboard_request = Some(callback);
    }

    /// Set callback to query whether the keyboard is currently visible.
    /// When provided, tap-to-toggle reads actual platform state so it stays
    /// in sync after external dismissals (e.g. tapping the drawer toggle).
    pub fn set_is_keyboard_visible_fn(&mut self, f: IsKeyboardVisibleFn) {
        self.is_keyboard_visible_fn = Some(f);
    }

    /// Request keyboard to show
    fn request_keyboard(&self, show: bool) {
        if let Some(ref request) = self.keyboard_request {
            request(show);
        }
    }

    /// Get a clone of the output buffer for SSH to write to
    pub fn output_buffer(&self) -> OutputBuffer {
        self.output_buffer.clone()
    }

    /// Wire in the session's output buffer and render-signal flag.
    pub fn set_output_buffer(&mut self, buffer: OutputBuffer, needs_render: Arc<AtomicBool>) {
        self.output_buffer = buffer;
        self.needs_render = Some(needs_render);
    }

    /// Set the callback invoked when the effective grid size changes.
    ///
    /// Called with `(cols, rows)` after a keyboard-height-induced resize.
    /// Use this to relay the new PTY size to the remote backend.
    pub fn set_resize_fn(&mut self, f: ResizeFn) {
        self.resize_fn = Some(f);
    }

    /// Drain the output buffer and feed bytes into the terminal emulator.
    /// Returns true if any data was processed.
    fn process_output(&mut self) -> bool {
        let mut had_data = false;
        let mut total_bytes = 0usize;

        if let Ok(mut buffer) = self.output_buffer.try_lock() {
            while let Some(data) = buffer.pop_front() {
                total_bytes += data.len();
                self.terminal.advance_bytes(&data);
                had_data = true;
            }
        }

        if had_data {
            log::debug!("[TERM DATA] processed {} bytes from PTY", total_bytes);
        }

        if had_data && !self.connected {
            self.connected = true;
            self.status_text = "Connected".to_string();
        }

        had_data
    }

    /// Set the callback for sending bytes to the SSH channel
    pub fn set_send_bytes(&mut self, callback: SendBytesFn) {
        self.send_bytes = Some(callback);
    }

    /// Mark the terminal as connected
    pub fn set_connected(&mut self, connected: bool) {
        self.connected = connected;
        self.status_text = if connected {
            "Connected".to_string()
        } else {
            "Disconnected".to_string()
        };
    }

    /// Set status text
    pub fn set_status(&mut self, status: String) {
        self.status_text = status;
    }

    /// Feed bytes from SSH into the terminal emulator
    pub fn advance_bytes(&mut self, bytes: &[u8]) {
        self.terminal.advance_bytes(bytes);
    }

    /// Resize the terminal
    pub fn resize(&mut self, columns: usize, rows: usize, cell_width: Pixels, line_height: Pixels) {
        self.base_rows = rows;
        self.last_keyboard_rows = rows;
        self.terminal.resize(columns, rows, cell_width, line_height);
    }

    /// Get the terminal size for PTY sizing
    pub fn terminal_size(&self) -> TerminalSize {
        self.terminal.size()
    }

    /// Handle a keystroke, converting to escape sequence and sending via SSH or RPC session
    fn handle_keystroke(&mut self, keystroke: &Keystroke) {
        // Try to convert keystroke to terminal escape sequence
        if let Some(bytes) = self.terminal.try_keystroke(keystroke) {
            self.send_bytes_to_remote(bytes);
        } else if let Some(ref key_char) = keystroke.key_char {
            // For plain characters, send the character directly
            if !keystroke.modifiers.control
                && !keystroke.modifiers.alt
                && !keystroke.modifiers.platform
            {
                let bytes = key_char.as_bytes().to_vec();
                self.send_bytes_to_remote(bytes);
            }
        }
    }

    /// Handle IME text input
    pub fn handle_ime_text(&mut self, text: &str) {
        let bytes = text.as_bytes().to_vec();
        self.send_bytes_to_remote(bytes);
    }

    /// Forward bytes to the remote PTY via the `send_bytes` callback.
    fn send_bytes_to_remote(&self, bytes: Vec<u8>) {
        if let Some(ref send) = self.send_bytes {
            send(bytes);
        }
    }

    /// Scroll the terminal
    pub fn scroll(&mut self, lines: i32) {
        self.terminal.scroll(lines);
    }

    /// Whether the terminal is connected
    pub fn is_connected(&self) -> bool {
        self.connected
    }

    /// Current status text (e.g. "Connected", "Connecting to ...")
    pub fn status_text(&self) -> &str {
        &self.status_text
    }
}

impl Focusable for TerminalView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

// Retained for future use: will be emitted when the PTY output stream closes.
impl gpui::EventEmitter<DisconnectRequested> for TerminalView {}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Process pending PTY output. When `needs_render` is set (wired to a
        // `RemoteTerminal`), only process if the pump signaled new data; the flag
        // is cleared atomically here so the pump can set it again immediately.
        let should_process = self
            .needs_render
            .as_ref()
            .map_or(true, |nr| nr.swap(false, Ordering::AcqRel));
        let had_data = if should_process { self.process_output() } else { false };
        if had_data {
            let size = self.terminal.size();
            log::info!(
                "[PERF] terminal: processed data, grid={}x{}",
                size.columns,
                size.rows
            );
        }

        // Adjust terminal rows based on soft keyboard height.
        // The keyboard height is reported in physical pixels by Android WindowInsets.
        {
            let kb_px = crate::get_keyboard_height();
            if kb_px > 0 || self.last_keyboard_rows != self.base_rows {
                let scale = crate::get_display_density();
                let kb_logical = kb_px as f32 / scale;
                let lh: f32 = (self.terminal.size().line_height / px(1.0)) as f32;
                let kb_rows = if lh > 0.0 {
                    (kb_logical / lh).ceil() as usize
                } else {
                    0
                };
                let effective_rows = self.base_rows.saturating_sub(kb_rows).max(2);
                if effective_rows != self.last_keyboard_rows {
                    log::info!(
                        "Keyboard resize: kb={}px logical={:.0} kb_rows={} base={} effective={}",
                        kb_px,
                        kb_logical,
                        kb_rows,
                        self.base_rows,
                        effective_rows
                    );

                    let size = self.terminal.size();
                    self.terminal.resize(
                        size.columns,
                        effective_rows,
                        size.cell_width,
                        size.line_height,
                    );
                    self.last_keyboard_rows = effective_rows;

                    // Notify the backend of the new PTY size via the resize callback.
                    let cols = size.columns as u16;
                    let rows = effective_rows as u16;
                    if let Some(ref resize) = self.resize_fn {
                        resize(cols, rows);
                    }
                }
            }
        }

        let content = self.terminal.content();
        let size = self.terminal.size();
        let focus_handle = self.focus_handle.clone();

        // Terminal grid only - status bar is rendered by the parent header
        div()
            .size_full()
            .overflow_hidden()
            .bg(rgb(0x0e0c0c))
            .track_focus(&focus_handle)
            .key_context("Terminal")
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, _cx| {
                    this.focus_handle.focus(window, _cx);
                    this.mouse_down_pos = Some(event.position);
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, _window, _cx| {
                    if let Some(down) = this.mouse_down_pos.take() {
                        let dx = ((event.position.x - down.x) / px(1.0)) as f32;
                        let dy = ((event.position.y - down.y) / px(1.0)) as f32;
                        if dx.abs() < 10.0 && dy.abs() < 10.0 {
                            let current = this.is_keyboard_visible_fn.as_ref().map_or(false, |f| f());
                            this.request_keyboard(!current);
                        }
                    }
                }),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, _cx| {
                this.handle_keystroke(&event.keystroke);
            }))
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _window, cx| {
                match event.delta {
                    ScrollDelta::Lines(l) => {
                        // Line-based scroll (e.g. mouse wheel): commit immediately
                        this.scroll_offset_px = 0.0;
                        let lines = l.y as i32;
                        if lines != 0 {
                            this.scroll(lines);
                        }
                    }
                    ScrollDelta::Pixels(pixels) => {
                        if matches!(event.touch_phase, TouchPhase::Ended) {
                            // Touch lifted: snap remaining offset to nearest line
                            let lh: f32 = (this.terminal.size().line_height / px(1.0)) as f32;
                            if this.scroll_offset_px.abs() > lh * 0.5 {
                                if this.scroll_offset_px > 0.0 {
                                    this.scroll(1);
                                } else {
                                    this.scroll(-1);
                                }
                            }
                            this.scroll_offset_px = 0.0;
                        } else {
                            let lh: f32 = (this.terminal.size().line_height / px(1.0)) as f32;
                            let py: f32 = (pixels.y / px(1.0)) as f32;
                            this.scroll_offset_px += py;

                            // Commit whole lines, clamping at scrollback boundaries.
                            while this.scroll_offset_px >= lh {
                                let before = this.terminal.display_offset();
                                this.scroll(1);
                                if this.terminal.display_offset() == before {
                                    // Hit top of scrollback — clamp offset
                                    this.scroll_offset_px = 0.0;
                                    break;
                                }
                                this.scroll_offset_px -= lh;
                            }
                            while this.scroll_offset_px <= -lh {
                                let before = this.terminal.display_offset();
                                this.scroll(-1);
                                if this.terminal.display_offset() == before {
                                    // Hit bottom of scrollback — clamp offset
                                    this.scroll_offset_px = 0.0;
                                    break;
                                }
                                this.scroll_offset_px += lh;
                            }

                            // Also clamp sub-line drift at boundaries
                            let offset = this.terminal.display_offset();
                            let history = this.terminal.history_size();
                            if offset == 0 && this.scroll_offset_px < 0.0 {
                                this.scroll_offset_px = 0.0; // at bottom
                            }
                            if offset >= history && this.scroll_offset_px > 0.0 {
                                this.scroll_offset_px = 0.0; // at top
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
                self.focus_handle.is_focused(window),
            ))
    }
}
