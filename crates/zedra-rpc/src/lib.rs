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
};

