// Terminal view - GPUI Render implementation for the terminal
// Manages terminal state, handles keyboard input, and renders the terminal grid

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use gpui::*;

use crate::element::TerminalElement;
use crate::{TerminalSize, TerminalState};

/// Callback for sending bytes to the SSH channel
pub type SendBytesFn = Box<dyn Fn(Vec<u8>) + Send + 'static>;

/// Thread-safe buffer for receiving SSH output
pub type OutputBuffer = Arc<Mutex<VecDeque<Vec<u8>>>>;

/// Callback for requesting keyboard show/hide
pub type KeyboardRequestFn = Box<dyn Fn(bool) + Send + 'static>;

/// Event emitted when user requests disconnect
pub struct DisconnectRequested;

/// Terminal view that implements GPUI's Render trait
pub struct TerminalView {
    terminal: TerminalState,
    send_bytes: Option<SendBytesFn>,
    output_buffer: OutputBuffer,
    connected: bool,
    status_text: String,
    focus_handle: FocusHandle,
    keyboard_request: Option<KeyboardRequestFn>,
    /// Accumulated scroll pixels that haven't yet formed a full line
    scroll_pixel_remainder: f32,
    /// Row count without keyboard (the "full" terminal height)
    base_rows: usize,
    /// Last keyboard-adjusted row count to avoid redundant resizes
    last_keyboard_rows: usize,
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
            connected: false,
            status_text: "Disconnected".to_string(),
            focus_handle: cx.focus_handle(),
            keyboard_request: None,
            scroll_pixel_remainder: 0.0,
            base_rows: rows,
            last_keyboard_rows: rows,
        }
    }

    /// Set callback to request keyboard show/hide
    pub fn set_keyboard_request(&mut self, callback: KeyboardRequestFn) {
        self.keyboard_request = Some(callback);
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

    /// Replace the output buffer (used to wire in the session's buffer)
    pub fn set_output_buffer(&mut self, buffer: OutputBuffer) {
        self.output_buffer = buffer;
    }

    /// Process any pending output from SSH or RPC session
    /// Returns true if any data was processed
    fn process_output(&mut self) -> bool {
        let mut had_data = false;
        let mut total_bytes = 0usize;

        // Check local buffer (SSH path)
        if let Ok(mut buffer) = self.output_buffer.try_lock() {
            while let Some(data) = buffer.pop_front() {
                total_bytes += data.len();
                self.terminal.advance_bytes(&data);
                had_data = true;
            }
        }

        // Check active RPC session buffer
        if let Some(session) = zedra_session::active_session() {
            let session_buf = session.output_buffer();
            if let Ok(mut buffer) = session_buf.try_lock() {
                while let Some(data) = buffer.pop_front() {
                    total_bytes += data.len();
                    self.terminal.advance_bytes(&data);
                    had_data = true;
                }
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
        log::debug!("Terminal keystroke: {:?}", keystroke);

        // Try to convert keystroke to terminal escape sequence
        if let Some(bytes) = self.terminal.try_keystroke(keystroke) {
            log::debug!("Sending escape sequence: {:?}", bytes);
            self.send_bytes_to_remote(bytes);
        } else if let Some(ref key_char) = keystroke.key_char {
            // For plain characters, send the character directly
            if !keystroke.modifiers.control
                && !keystroke.modifiers.alt
                && !keystroke.modifiers.platform
            {
                let bytes = key_char.as_bytes().to_vec();
                log::debug!("Sending character: {:?}", bytes);
                self.send_bytes_to_remote(bytes);
            }
        }
    }

    /// Handle IME text input
    pub fn handle_ime_text(&mut self, text: &str) {
        log::debug!("Terminal IME text: {}", text);
        let bytes = text.as_bytes().to_vec();
        self.send_bytes_to_remote(bytes);
    }

    /// Send bytes to the remote host via RPC session or callback fallback.
    fn send_bytes_to_remote(&self, bytes: Vec<u8>) {
        if zedra_session::send_terminal_input(bytes.clone()) {
            return;
        }
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

impl gpui::EventEmitter<DisconnectRequested> for TerminalView {}

impl Render for TerminalView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Process any pending SSH/RPC output before rendering.
        // Re-renders are driven by the frame loop (request_frame_forced) when
        // TERMINAL_DATA_PENDING is set, so no cx.notify() loop is needed here.
        self.process_output();

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
                        kb_px, kb_logical, kb_rows, self.base_rows, effective_rows
                    );
                    let size = self.terminal.size();
                    self.terminal
                        .resize(size.columns, effective_rows, size.cell_width, size.line_height);
                    self.last_keyboard_rows = effective_rows;

                    // Fire-and-forget remote PTY resize
                    let cols = size.columns as u16;
                    let rows = effective_rows as u16;
                    if let Some(session) = zedra_session::active_session() {
                        if let Some(term_id) = session.terminal_id() {
                            zedra_session::session_runtime().spawn(async move {
                                if let Err(e) =
                                    session.terminal_resize(&term_id, cols, rows).await
                                {
                                    log::warn!("Remote PTY resize failed: {}", e);
                                }
                            });
                        }
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
                cx.listener(|this, _event, window, cx| {
                    this.focus_handle.focus(window, cx);
                    this.request_keyboard(true);
                }),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, _cx| {
                this.handle_keystroke(&event.keystroke);
            }))
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _window, cx| {
                let lines = match event.delta {
                    ScrollDelta::Lines(l) => {
                        this.scroll_pixel_remainder = 0.0;
                        l.y as i32
                    }
                    ScrollDelta::Pixels(pixels) => {
                        let lh: f32 = (this.terminal.size().line_height / px(1.0)) as f32;
                        let py: f32 = (pixels.y / px(1.0)) as f32;
                        this.scroll_pixel_remainder += py;
                        let whole = (this.scroll_pixel_remainder / lh) as i32;
                        this.scroll_pixel_remainder -= whole as f32 * lh;
                        whole
                    }
                };
                if lines != 0 {
                    this.scroll(lines);
                    cx.notify();
                }
            }))
            .child(TerminalElement::new(content, size))
    }
}
