pub mod discovery;
pub mod durable_queue;
pub mod frame;
pub mod manager;
pub mod mdns;
pub mod pairing;
pub mod providers;
pub mod signaling;

pub use durable_queue::DurableQueue;
pub use frame::Frame;
pub use manager::{TransportManager, TransportState};
pub use mdns::{Announcement, DiscoveryCache};
pub use signaling::HostDiscovery;

/// Connection info parsed from QR code pairing payload.
#[derive(Debug, Clone)]
pub struct PeerInfo {
    /// LAN IPs from QR code
    pub host_addrs: Vec<String>,
    /// 100.x.x.x Tailscale address, if available
    pub tailscale_addr: Option<String>,
    /// RPC port (default 2123)
    pub port: u16,
    /// Cloudflare Worker relay URL
    pub relay_url: String,
    /// Relay room code (used by WS relay)
    pub relay_room: String,
    /// Relay room secret (used by WS relay)
    pub relay_secret: String,
    /// Friendly hostname
    pub hostname: String,
    /// Host's Curve25519 public key for Noise_IK (base64url-encoded)
    pub host_pubkey: String,
    /// One-time pairing token (OTP, only present on first pairing)
    pub otp: Option<String>,
    /// Coordination server URL
    pub coord_url: Option<String>,
}
