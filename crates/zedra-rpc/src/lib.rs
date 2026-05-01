// zedra-rpc: RPC protocol types and pairing for Zedra remote tunnel
//
// Provides the typed irpc protocol between mobile client and desktop host,
// and QR pairing (EndpointAddr encoding).

pub mod pairing;
pub mod proto;

pub use pairing::{
    ZedraPairingTicket, compute_registration_hmac, decode_endpoint_addr, encode_endpoint_addr,
    generate_session_id, verify_registration_hmac,
};

/// All known Zedra relay URLs, ordered by priority.
/// iroh probes all relays and picks the lowest-latency one as preferred.
/// If the preferred relay goes down, iroh fails over to the next best.
pub const ZEDRA_RELAY_URLS: &[&str] = &[
    "https://sg1.relay.zedra.dev", // Singapore (ap-southeast-1)
    "https://vn1.relay.zedra.dev", // Vietnam
    "https://us1.relay.zedra.dev", // N. Virginia (us-east-1)
    "https://eu1.relay.zedra.dev", // Frankfurt (eu-central-1)
];
