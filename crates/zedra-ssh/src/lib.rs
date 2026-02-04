#![allow(refining_impl_trait)]

pub mod bridge;
pub mod client;
pub mod connection;
pub mod pairing;

/// Trait for types that can receive SSH terminal data.
/// Implemented by TerminalView in zedra-terminal.
pub trait TerminalSink: 'static {
    fn advance_bytes(&mut self, bytes: &[u8]);
    fn set_connected(&mut self, connected: bool);
    fn set_status(&mut self, status: String);
    fn set_send_bytes(&mut self, callback: Box<dyn Fn(Vec<u8>) + Send + 'static>);
    fn terminal_size_cells(&self) -> (u32, u32); // (cols, rows)
}
