pub mod discovery;
pub mod manager;
pub mod pairing;
pub mod providers;

pub use manager::{TransportManager, TransportState};

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
    /// Relay room code
    pub relay_room: String,
    /// Relay room secret
    pub relay_secret: String,
    /// Host key fingerprint
    pub fingerprint: String,
    /// Friendly hostname
    pub hostname: String,
}
