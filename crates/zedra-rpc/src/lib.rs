// zedra-rpc: JSON-RPC 2.0 protocol types and transport for Zedra remote tunnel
//
// Provides the wire protocol between mobile client and desktop host.
// Transport-agnostic: works over WebSocket, TCP, or any async stream.

mod protocol;
mod transport;

pub use protocol::*;
pub use transport::*;
