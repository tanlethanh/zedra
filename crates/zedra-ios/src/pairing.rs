/// QR code pairing protocol — inlined from zedra-ssh::pairing
///
/// This avoids pulling in the full zedra-ssh crate which depends on gpui.
/// When zedra-ssh gets feature-gated gpui support, this can be replaced
/// with a direct dependency.
use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Pairing payload from QR code (supports v1 and v2)
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PairingPayload {
    pub v: u32,
    pub host: String,
    pub port: u16,
    pub token: String,
    pub fingerprint: String,
    pub name: String,
    #[serde(default)]
    pub host_addrs: Vec<String>,
    #[serde(default)]
    pub tailscale_addr: Option<String>,
    #[serde(default)]
    pub relay_url: Option<String>,
    #[serde(default)]
    pub relay_room: Option<String>,
    #[serde(default)]
    pub relay_secret: Option<String>,
}

impl PairingPayload {
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
