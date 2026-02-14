// QR code pairing protocol
// Parses zedra://pair URIs and performs the pairing handshake

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Pairing payload from QR code (supports v1 and v2)
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PairingPayload {
    /// Protocol version (1 or 2)
    pub v: u32,
    /// Host address (primary LAN IP for backward compat)
    pub host: String,
    /// SSH port
    pub port: u16,
    /// One-time pairing token
    pub token: String,
    /// Expected host key fingerprint
    pub fingerprint: String,
    /// Friendly name for the host
    pub name: String,
    // v2 fields — optional with serde defaults for backward compat with v1
    /// All LAN IPs discovered on the host
    #[serde(default)]
    pub host_addrs: Vec<String>,
    /// Tailscale 100.x.x.x address, if available
    #[serde(default)]
    pub tailscale_addr: Option<String>,
    /// Cloudflare Worker relay URL
    #[serde(default)]
    pub relay_url: Option<String>,
    /// Relay room code
    #[serde(default)]
    pub relay_room: Option<String>,
    /// Relay room secret
    #[serde(default)]
    pub relay_secret: Option<String>,
}

impl PairingPayload {
    /// Convert this pairing payload into a PeerInfo for TransportManager.
    pub fn to_peer_info(&self) -> zedra_transport::PeerInfo {
        let mut host_addrs = self.host_addrs.clone();
        if host_addrs.is_empty() {
            host_addrs.push(self.host.clone());
        }
        zedra_transport::PeerInfo {
            host_addrs,
            tailscale_addr: self.tailscale_addr.clone(),
            port: self.port,
            relay_url: self.relay_url.clone().unwrap_or_default(),
            relay_room: self.relay_room.clone().unwrap_or_default(),
            relay_secret: self.relay_secret.clone().unwrap_or_default(),
            fingerprint: self.fingerprint.clone(),
            hostname: self.name.clone(),
        }
    }
}

/// Parse a zedra://pair?d=<payload> URI
pub fn parse_pairing_uri(uri: &str) -> Result<PairingPayload> {
    let prefix = "zedra://pair?d=";
    if !uri.starts_with(prefix) {
        anyhow::bail!("Invalid pairing URI: expected zedra://pair?d=...");
    }

    let encoded = &uri[prefix.len()..];
    let json_bytes = base64_url::decode(encoded)
        .map_err(|e| anyhow::anyhow!("Failed to decode base64: {}", e))?;
    let json_str = String::from_utf8(json_bytes)?;
    let payload: PairingPayload = serde_json::from_str(&json_str)?;

    if payload.v != 1 && payload.v != 2 {
        anyhow::bail!("Unsupported pairing protocol version: {}", payload.v);
    }

    Ok(payload)
}

/// Saved host credentials (stored on the device after pairing)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SavedHost {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub fingerprint: String,
    pub username: String,
    pub auth_method: SavedAuthMethod,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum SavedAuthMethod {
    Password { password: String },
    PublicKey { private_key_pem: String },
}
