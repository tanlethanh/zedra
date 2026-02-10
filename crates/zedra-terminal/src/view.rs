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

/// Terminal view that implements GPUI's Render trait
pub struct TerminalView {
    terminal: TerminalState,
    send_bytes: Option<SendBytesFn>,
    output_buffer: OutputBuffer,
    connected: bool,
    status_text: String,
}

impl TerminalView {
    pub fn new(columns: usize, rows: usize, cell_width: Pixels, line_height: Pixels) -> Self {
        Self {
            terminal: TerminalState::new(columns, rows, cell_width, line_height),
            send_bytes: None,
            output_buffer: Arc::new(Mutex::new(VecDeque::new())),
            connected: false,
            status_text: "Disconnected".to_string(),
        }
    }

    /// Get a clone of the output buffer for SSH to write to
    pub fn output_buffer(&self) -> OutputBuffer {
        self.output_buffer.clone()
    }

    /// Process any pending output from SSH
    /// Returns true if any data was processed
    fn process_output(&mut self) -> bool {
        if let Ok(mut buffer) = self.output_buffer.try_lock() {
            let count = buffer.len();
            if count > 0 {
                log::info!("Processing {} buffered SSH outputs", count);
                while let Some(data) = buffer.pop_front() {
                    log::info!("Advancing {} bytes to terminal", data.len());
                    self.terminal.advance_bytes(&data);
                }
                // Mark as connected if we received data
                if !self.connected {
                    self.connected = true;
                    self.status_text = "Connected".to_string();
                }
                return true;
            }
        }
        false
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
        self.terminal.resize(columns, rows, cell_width, line_height);
    }

    /// Get the terminal size for PTY sizing
    pub fn terminal_size(&self) -> TerminalSize {
        self.terminal.size()
    }

    /// Handle a keystroke, converting to escape sequence and sending via SSH
    fn handle_keystroke(&mut self, keystroke: &Keystroke) {
        // Try to convert keystroke to terminal escape sequence
        if let Some(bytes) = self.terminal.try_keystroke(keystroke) {
            if let Some(ref send) = self.send_bytes {
                send(bytes);
            }
        } else if let Some(ref key_char) = keystroke.key_char {
            // For plain characters, send the character directly
            if !keystroke.modifiers.control
                && !keystroke.modifiers.alt
                && !keystroke.modifiers.platform
            {
                if let Some(ref send) = self.send_bytes {
                    send(key_char.as_bytes().to_vec());
                }
            }
        }
    }

    /// Scroll the terminal
    pub fn scroll(&mut self, lines: i32) {
        self.terminal.scroll(lines);
    }
}

/// Implement TerminalSink so SSH can drive this view generically
impl zedra_ssh::TerminalSink for TerminalView {
    fn advance_bytes(&mut self, bytes: &[u8]) {
        self.advance_bytes(bytes);
    }

    fn set_connected(&mut self, connected: bool) {
        self.set_connected(connected);
    }

    fn set_status(&mut self, status: String) {
        self.set_status(status);
    }

    fn set_send_bytes(&mut self, callback: Box<dyn Fn(Vec<u8>) + Send + 'static>) {
        self.set_send_bytes(callback);
    }

    fn terminal_size_cells(&self) -> (u32, u32) {
        let s = self.terminal_size();
        (s.columns as u32, s.rows as u32)
    }

    fn output_buffer(&self) -> zedra_ssh::OutputBuffer {
        self.output_buffer()
    }
}

impl Render for TerminalView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Process any pending SSH output before rendering
        let had_data = self.process_output();

        // If we processed data, schedule another render to check for more
        // This creates a polling loop while data is coming in
        if had_data {
            cx.notify();
        }

        let content = self.terminal.content();
        let size = self.terminal.size();
        let status = self.status_text.clone();
        let connected = self.connected;

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x1e1e1e))
            .child(
                // Status bar at top
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .px_2()
                    .py_1()
                    .bg(rgb(0x282c34))
                    .child(
                        div()
                            .text_color(if connected {
                                rgb(0x98c379) // green
                            } else {
                                rgb(0xe06c75) // red
                            })
                            .text_sm()
                            .child(status),
                    ),
            )
            .child(
                // Terminal grid
                div()
                    .flex_1()
                    .overflow_hidden()
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, _cx| {
                        this.handle_keystroke(&event.keystroke);
                    }))
                    .on_scroll_wheel(cx.listener(
                        |this, event: &ScrollWheelEvent, _window, _cx| {
                            let delta = match event.delta {
                                ScrollDelta::Lines(lines) => lines.y as i32,
                                ScrollDelta::Pixels(pixels) => {
                                    (pixels.y / this.terminal.size().line_height) as i32
                                }
                            };
                            if delta != 0 {
                                this.scroll(-delta);
                            }
                        },
                    ))
                    .child(TerminalElement::new(content, size)),
            )
    }
}
