// QR code pairing protocol
// Parses zedra://pair URIs (iroh-based)

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Pairing payload for iroh-based transport.
///
/// Contains the host's iroh EndpointId (Ed25519 public key) which is both
/// the host's identity and addressing key for iroh connections.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PairingPayload {
    /// Protocol version (always 1)
    pub v: u32,
    /// iroh EndpointId (z-base-32 encoded Ed25519 public key)
    pub endpoint_id: String,
    /// Friendly hostname
    pub name: String,
    /// iroh relay URL the host is connected to
    #[serde(default)]
    pub relay_url: Option<String>,
    /// Direct addresses (host IPs with iroh UDP port)
    #[serde(default)]
    pub addrs: Vec<String>,
}

impl PairingPayload {
    /// Convert this payload into an iroh `EndpointAddr`.
    pub fn to_endpoint_addr(&self) -> Result<iroh::EndpointAddr> {
        let endpoint_id: iroh::PublicKey = self
            .endpoint_id
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid endpoint_id: {}", e))?;

        let mut addr = iroh::EndpointAddr::from(endpoint_id);

        if let Some(ref relay) = self.relay_url {
            if let Ok(relay_url) = relay.parse::<iroh::RelayUrl>() {
                addr = addr.with_relay_url(relay_url);
            }
        }

        for a in &self.addrs {
            if let Ok(sock_addr) = a.parse::<std::net::SocketAddr>() {
                addr = addr.with_ip_addr(sock_addr);
            }
        }

        Ok(addr)
    }
}

/// Parse a zedra://pair?d=<payload> URI.
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
    if payload.v != 1 {
        anyhow::bail!(
            "Unsupported pairing protocol version: {} (only v1 supported)",
            payload.v
        );
    }
    Ok(payload)
}
