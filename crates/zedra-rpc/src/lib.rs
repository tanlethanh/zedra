// zedra-rpc: RPC protocol types and pairing for Zedra remote tunnel
//
// Provides the typed irpc protocol between mobile client and desktop host,
// and QR pairing (EndpointAddr encoding).

pub mod pairing;
pub mod proto;

pub use pairing::{
    ZedraPairingTicket,
    decode_endpoint_addr, encode_endpoint_addr,
    compute_registration_hmac, verify_registration_hmac,
    generate_session_id,
};

/// Self-hosted relay URL (Singapore, ap-southeast-1).
/// Used by both host and client to avoid n0's default relay latency.
pub const ZEDRA_RELAY_URL: &str = "https://sg1.relay.zedra.dev";

