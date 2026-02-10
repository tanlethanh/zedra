#![allow(refining_impl_trait)]

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

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
