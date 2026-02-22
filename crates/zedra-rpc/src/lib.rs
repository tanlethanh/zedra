// zedra-rpc: RPC protocol types and pairing for Zedra remote tunnel
//
// Provides the typed irpc protocol between mobile client and desktop host,
// and QR pairing (EndpointAddr encoding).

pub mod pairing;
pub mod proto;

pub use pairing::{decode_endpoint_addr, encode_endpoint_addr};

pub const DEFAULT_RELAY_URL: &str = "https://relay.zedra.dev";
