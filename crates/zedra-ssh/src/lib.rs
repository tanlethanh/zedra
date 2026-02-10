#![allow(refining_impl_trait)]

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::mpsc;

pub mod bridge;
pub mod client;
pub mod connection;
pub mod pairing;

/// Thread-safe buffer for receiving SSH output
pub type OutputBuffer = Arc<Mutex<VecDeque<Vec<u8>>>>;

/// Global flag indicating terminal data is pending.
/// Set by the SSH I/O task when data arrives.
/// Checked by the main frame loop to trigger terminal refreshes.
pub static TERMINAL_DATA_PENDING: AtomicBool = AtomicBool::new(false);

/// Global input sender for SSH - allows terminal to send keystrokes
static INPUT_SENDER: OnceLock<Mutex<Option<mpsc::UnboundedSender<Vec<u8>>>>> = OnceLock::new();

fn input_sender_slot() -> &'static Mutex<Option<mpsc::UnboundedSender<Vec<u8>>>> {
    INPUT_SENDER.get_or_init(|| Mutex::new(None))
}

/// Store the input sender (called from SSH connection task)
pub fn set_input_sender(sender: mpsc::UnboundedSender<Vec<u8>>) {
    if let Ok(mut slot) = input_sender_slot().lock() {
        *slot = Some(sender);
        log::info!("Input sender stored for terminal");
    }
}

/// Clear the input sender (called when connection closes)
pub fn clear_input_sender() {
    if let Ok(mut slot) = input_sender_slot().lock() {
        *slot = None;
    }
}

/// Send bytes to SSH (called from terminal view)
pub fn send_to_ssh(data: Vec<u8>) -> bool {
    if let Ok(slot) = input_sender_slot().lock() {
        if let Some(ref sender) = *slot {
            if sender.send(data).is_ok() {
                return true;
            }
        }
    }
    false
}

/// Check if SSH input is ready
pub fn is_input_ready() -> bool {
    if let Ok(slot) = input_sender_slot().lock() {
        return slot.is_some();
    }
    false
}

/// Signal that terminal data is available (called from SSH I/O task)
pub fn signal_terminal_data() {
    TERMINAL_DATA_PENDING.store(true, Ordering::Release);
}

/// Check and clear the terminal data pending flag (called from main thread)
pub fn check_and_clear_terminal_data() -> bool {
    TERMINAL_DATA_PENDING.swap(false, Ordering::AcqRel)
}

/// Trait for types that can receive SSH terminal data.
/// Implemented by TerminalView in zedra-terminal.
pub trait TerminalSink: 'static {
    fn advance_bytes(&mut self, bytes: &[u8]);
    fn set_connected(&mut self, connected: bool);
    fn set_status(&mut self, status: String);
    fn set_send_bytes(&mut self, callback: Box<dyn Fn(Vec<u8>) + Send + 'static>);
    fn terminal_size_cells(&self) -> (u32, u32); // (cols, rows)
    fn output_buffer(&self) -> OutputBuffer;
}
