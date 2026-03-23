// Terminal view - GPUI Render implementation for the terminal
// Manages terminal state, handles keyboard input, and renders the terminal grid

use std::cmp::min;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use alacritty_terminal::index::{Column as GridColumn, Line as GridLine, Point as GridPoint};
use alacritty_terminal::term::TermMode;
use gpui::*;

use crate::element::TerminalElement;
use crate::{TerminalSize, TerminalState};

const REMOTE_TOUCH_SCROLL_STEP_PX: f32 = 12.0;

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

/// Event emitted when the user taps a hyperlink in the terminal.
/// The payload is the raw URI from the OSC 8 sequence (e.g. `https://…` or `file:///…`).
pub struct LinkTapped(pub String);

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
            grid_origin: None,
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

        if let Ok(mut buffer) = self.output_buffer.try_lock() {
            while let Some(data) = buffer.pop_front() {
                self.terminal.advance_bytes(&data);
                had_data = true;
            }
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

    pub fn set_grid_origin(&mut self, origin: Point<Pixels>) {
        self.grid_origin = Some(origin);
    }

    fn link_at_position(&self, position: Point<Pixels>) -> Option<String> {
        let origin = self.grid_origin?;
        let size = self.terminal.size();
        // The painted grid origin is offset by the sub-line scroll amount.
        let rel_x = position.x - origin.x;
        let rel_y = position.y - (origin.y + px(self.scroll_offset_px));
        if rel_x < px(0.0) || rel_y < px(0.0) {
            return None;
        }
        let col = (rel_x / size.cell_width) as usize;
        let row = (rel_y / size.line_height) as usize;
        self.terminal.link_at(col, row)
    }

    fn mouse_mode(&self, event: &ScrollWheelEvent) -> bool {
        self.send_bytes.is_some()
            && !event.modifiers.shift
            && self.terminal.mode().intersects(TermMode::MOUSE_MODE)
    }

    fn should_send_alt_scroll(&self, event: &ScrollWheelEvent) -> bool {
        if self.mouse_mode(event) || self.send_bytes.is_none() || event.modifiers.shift {
            return false;
        }

        let mode = self.terminal.mode();
        mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL)
    }

    fn scroll_step_px(&self, event: &ScrollWheelEvent) -> f32 {
        if self.mouse_mode(event) || self.should_send_alt_scroll(event) {
            REMOTE_TOUCH_SCROLL_STEP_PX
        } else {
            (self.terminal.size().line_height / px(1.0)) as f32
        }
    }

    fn should_snap_touch_release(&self, event: &ScrollWheelEvent) -> bool {
        !self.mouse_mode(event) && !self.should_send_alt_scroll(event)
    }

    fn scroll_point(&self, event: &ScrollWheelEvent) -> Option<GridPoint> {
        let size = self.terminal.size();
        let columns = size.columns.max(1);
        let rows = size.rows.max(1);

        let (column, line) = if matches!(event.delta, ScrollDelta::Pixels(_)) {
            // Touch scroll is a gesture, not a hover-wheel. Route it through the
            // middle of the viewport so TUIs don't depend on the finger resting on
            // an exact widget the way desktop pointer wheels do.
            (columns / 2, (rows / 2) as i32)
        } else {
            let origin = self.grid_origin?;
            let relative = event.position - origin;
            let x = relative.x.max(px(0.0));
            let y = relative.y.max(px(0.0));
            (
                min((x / size.cell_width) as usize, columns.saturating_sub(1)),
                min((y / size.line_height) as usize, rows.saturating_sub(1)) as i32,
            )
        };

        let line = line - self.terminal.display_offset() as i32;

        Some(GridPoint::new(GridLine(line), GridColumn(column)))
    }

    fn send_mouse_scroll(&self, lines: i32, event: &ScrollWheelEvent) -> bool {
        let Some(point) = self.scroll_point(event) else {
            return false;
        };
        let Some(report) = scroll_report_bytes(point, event, self.terminal.mode()) else {
            return false;
        };

        for _ in 0..lines.unsigned_abs() {
            self.send_bytes_to_remote(report.clone());
        }

        true
    }

    fn commit_scroll_lines(&mut self, lines: i32, event: &ScrollWheelEvent) -> bool {
        if lines == 0 {
            return false;
        }

        if self.mouse_mode(event) {
            return self.send_mouse_scroll(lines, event);
        }

        if self.should_send_alt_scroll(event) {
            self.send_bytes_to_remote(alt_scroll_bytes(lines));
            return true;
        }

        let before = self.terminal.display_offset();
        self.scroll(lines);
        self.terminal.display_offset() != before
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

impl gpui::EventEmitter<DisconnectRequested> for TerminalView {}
impl gpui::EventEmitter<LinkTapped> for TerminalView {}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Process pending PTY output. When `needs_render` is set (wired to a
        // `RemoteTerminal`), only process if the pump signaled new data; the flag
        // is cleared atomically here so the pump can set it again immediately.
        let should_process = self
            .needs_render
            .as_ref()
            .map_or(true, |nr| nr.swap(false, Ordering::AcqRel));
        if should_process {
            self.process_output();
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
                    tracing::info!(
                        kb_px,
                        kb_logical,
                        kb_rows,
                        base = self.base_rows,
                        effective = effective_rows,
                        "terminal: keyboard resize"
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
                cx.listener(|this, event: &MouseUpEvent, _window, cx| {
                    if let Some(down) = this.mouse_down_pos.take() {
                        let dx = ((event.position.x - down.x) / px(1.0)) as f32;
                        let dy = ((event.position.y - down.y) / px(1.0)) as f32;
                        if dx.abs() < 10.0 && dy.abs() < 10.0 {
                            if let Some(url) = this.link_at_position(event.position) {
                                cx.emit(LinkTapped(url));
                            } else {
                                let current =
                                    this.is_keyboard_visible_fn.as_ref().map_or(false, |f| f());
                                this.request_keyboard(!current);
                            }
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
                            this.commit_scroll_lines(lines, event);
                        }
                    }
                    ScrollDelta::Pixels(pixels) => {
                        if matches!(event.touch_phase, TouchPhase::Ended) {
                            if this.should_snap_touch_release(event)
                                && this.scroll_offset_px.abs() > this.scroll_step_px(event) * 0.5
                            {
                                // Local scrollback benefits from snapping the partial drag
                                // to the nearest line, but remote TUIs should emit while
                                // dragging instead of waiting for finger lift.
                                if this.scroll_offset_px > 0.0 {
                                    this.commit_scroll_lines(1, event);
                                } else {
                                    this.commit_scroll_lines(-1, event);
                                }
                            }
                            this.scroll_offset_px = 0.0;
                        } else {
                            let step_px = this.scroll_step_px(event);
                            let py: f32 = (pixels.y / px(1.0)) as f32;
                            this.scroll_offset_px += py;

                            // Remote terminal scroll should emit small, repeated wheel
                            // ticks while dragging; local scrollback keeps line-based steps.
                            while this.scroll_offset_px >= step_px {
                                if !this.commit_scroll_lines(1, event) {
                                    // Hit top of scrollback — clamp offset
                                    this.scroll_offset_px = 0.0;
                                    break;
                                }
                                this.scroll_offset_px -= step_px;
                            }
                            while this.scroll_offset_px <= -step_px {
                                if !this.commit_scroll_lines(-1, event) {
                                    // Hit bottom of scrollback — clamp offset
                                    this.scroll_offset_px = 0.0;
                                    break;
                                }
                                this.scroll_offset_px += step_px;
                            }

                            // Local scrollback clamps at the history bounds, but alt-screen
                            // scroll should keep producing cursor-up/down bytes for the PTY.
                            if !this.should_send_alt_scroll(event) {
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

fn alt_scroll_bytes(lines: i32) -> Vec<u8> {
    let command = if lines > 0 { b'A' } else { b'B' };
    let mut bytes = Vec::with_capacity(lines.unsigned_abs() as usize * 3);

    for _ in 0..lines.abs() {
        bytes.push(0x1b);
        bytes.push(b'O');
        bytes.push(command);
    }

    bytes
}

fn scroll_report_bytes(
    point: GridPoint,
    event: &ScrollWheelEvent,
    mode: TermMode,
) -> Option<Vec<u8>> {
    if !mode.intersects(TermMode::MOUSE_MODE) || point.line < GridLine(0) {
        return None;
    }

    let mut button = if scroll_is_up(event) { 64 } else { 65 };
    if event.modifiers.shift {
        button += 4;
    }
    if event.modifiers.alt {
        button += 8;
    }
    if event.modifiers.control {
        button += 16;
    }

    if mode.contains(TermMode::SGR_MOUSE) {
        Some(
            format!(
                "\x1b[<{};{};{}M",
                button,
                point.column.0 + 1,
                point.line.0 + 1
            )
            .into_bytes(),
        )
    } else {
        normal_mouse_scroll_report(point, button, mode.contains(TermMode::UTF8_MOUSE))
    }
}

fn scroll_is_up(event: &ScrollWheelEvent) -> bool {
    match event.delta {
        ScrollDelta::Pixels(delta) => delta.y > px(0.0),
        ScrollDelta::Lines(delta) => delta.y > 0.0,
    }
}

fn normal_mouse_scroll_report(point: GridPoint, button: u8, utf8: bool) -> Option<Vec<u8>> {
    let line = point.line.0;
    let column = point.column.0;
    let max_point = if utf8 { 2015usize } else { 223usize };

    if line < 0 || line as usize >= max_point || column >= max_point {
        return None;
    }

    let mut report = vec![b'\x1b', b'[', b'M', 32 + button];

    if utf8 && column >= 95 {
        report.extend(encode_mouse_position(column));
    } else {
        report.push(32 + 1 + column as u8);
    }

    let line = line as usize;
    if utf8 && line >= 95 {
        report.extend(encode_mouse_position(line));
    } else {
        report.push(32 + 1 + line as u8);
    }

    Some(report)
}

fn encode_mouse_position(position: usize) -> [u8; 2] {
    let position = 32 + 1 + position;
    [(0xC0 + position / 64) as u8, (0x80 + (position & 63)) as u8]
}
