// QR code pairing protocol
// Parses zedra://pair URIs and performs the pairing handshake
//
// Supports v3 pairing payload — encrypted with Noise_IK (PairingPayloadV3 with host_pubkey + OTP)

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// v3 pairing payload — encrypted transport with Noise_IK.
///
/// Contains the host's Curve25519 public key (for Noise_IK handshake)
/// and a one-time pairing token for first-contact authentication.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PairingPayloadV3 {
    /// Protocol version (always 3)
    pub v: u32,
    /// Base64url-encoded 32-byte Curve25519 public key
    pub host_pubkey: String,
    /// One-time pairing token
    pub otp: String,
    /// Friendly hostname
    pub name: String,
    /// All LAN addresses
    pub host_addrs: Vec<String>,
    /// Port
    pub port: u16,
    /// Coordination server URL (future use)
    #[serde(default)]
    pub coord_url: Option<String>,
    /// Connection hints
    pub hints: PairingHints,
}

/// Connection hints embedded in v3 QR payload.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PairingHints {
    /// Full addr:port strings for direct connection
    pub addrs: Vec<String>,
    /// Tailscale address if available
    #[serde(default)]
    pub tailscale: Option<String>,
    /// Relay URL with room parameter
    #[serde(default)]
    pub relay: Option<String>,
}

impl PairingPayloadV3 {
    /// Convert this v3 payload into a PeerInfo for TransportManager.
    pub fn to_peer_info(&self) -> crate::PeerInfo {
        // Extract relay URL and room from hints.relay (format: "url?room=code")
        let (relay_url, relay_room) = self
            .hints
            .relay
            .as_ref()
            .and_then(|r| {
                let parts: Vec<&str> = r.splitn(2, "?room=").collect();
                if parts.len() == 2 {
                    Some((parts[0].to_string(), parts[1].to_string()))
                } else {
                    None
                }
            })
            .unwrap_or_default();

        crate::PeerInfo {
            host_addrs: self.host_addrs.clone(),
            tailscale_addr: self.hints.tailscale.clone(),
            port: self.port,
            relay_url,
            relay_room,
            relay_secret: String::new(), // v3 uses Noise encryption, no relay secret needed
            hostname: self.name.clone(),
            host_pubkey: self.host_pubkey.clone(),
            otp: Some(self.otp.clone()),
            coord_url: self.coord_url.clone(),
        }
    }
}

/// Parse a zedra://pair?d=<payload> URI (v3 only).
pub fn parse_pairing_uri(uri: &str) -> Result<PairingPayloadV3> {
    let prefix = "zedra://pair?d=";
    if !uri.starts_with(prefix) {
        anyhow::bail!("Invalid pairing URI: expected zedra://pair?d=...");
    }

    let encoded = &uri[prefix.len()..];
    let json_bytes = base64_url::decode(encoded)
        .map_err(|e| anyhow::anyhow!("Failed to decode base64: {}", e))?;
    let json_str = String::from_utf8(json_bytes)?;

    // Peek at version to validate
    let version: serde_json::Value = serde_json::from_str(&json_str)?;
    let v = version
        .get("v")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    if v != 3 {
        anyhow::bail!(
            "Unsupported pairing protocol version: {} (only v3 is supported)",
            v
        );
    }

    let payload: PairingPayloadV3 = serde_json::from_str(&json_str)?;
    Ok(payload)
}
