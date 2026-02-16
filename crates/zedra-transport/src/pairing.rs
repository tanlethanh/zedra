// QR code pairing protocol
// Parses zedra://pair URIs and performs the pairing handshake
//
// Supports three QR payload versions:
//   v1/v2 — legacy plaintext (PairingPayload with token + fingerprint)
//   v3    — encrypted with Noise_IK (PairingPayloadV3 with host_pubkey + OTP)

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Pairing payload from QR code (v1 and v2 — legacy plaintext)
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
    pub fn to_peer_info(&self) -> crate::PeerInfo {
        let mut host_addrs = self.host_addrs.clone();
        if host_addrs.is_empty() {
            host_addrs.push(self.host.clone());
        }
        crate::PeerInfo {
            host_addrs,
            tailscale_addr: self.tailscale_addr.clone(),
            port: self.port,
            relay_url: self.relay_url.clone().unwrap_or_default(),
            relay_room: self.relay_room.clone().unwrap_or_default(),
            relay_secret: self.relay_secret.clone().unwrap_or_default(),
            fingerprint: self.fingerprint.clone(),
            hostname: self.name.clone(),
            host_pubkey: None,
            otp: None,
            coord_url: None,
        }
    }
}

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
            fingerprint: String::new(),  // v3 uses host_pubkey instead
            hostname: self.name.clone(),
            host_pubkey: Some(self.host_pubkey.clone()),
            otp: Some(self.otp.clone()),
            coord_url: self.coord_url.clone(),
        }
    }
}

/// Parsed result from any QR version
pub enum ParsedPairing {
    /// v1/v2 legacy plaintext
    Legacy(PairingPayload),
    /// v3 encrypted (Noise_IK)
    V3(PairingPayloadV3),
}

impl ParsedPairing {
    /// Convert any version into a PeerInfo.
    pub fn to_peer_info(&self) -> crate::PeerInfo {
        match self {
            ParsedPairing::Legacy(p) => p.to_peer_info(),
            ParsedPairing::V3(p) => p.to_peer_info(),
        }
    }
}

/// Parse a zedra://pair?d=<payload> URI (supports v1, v2, and v3).
pub fn parse_pairing_uri(uri: &str) -> Result<ParsedPairing> {
    let prefix = "zedra://pair?d=";
    if !uri.starts_with(prefix) {
        anyhow::bail!("Invalid pairing URI: expected zedra://pair?d=...");
    }

    let encoded = &uri[prefix.len()..];
    let json_bytes = base64_url::decode(encoded)
        .map_err(|e| anyhow::anyhow!("Failed to decode base64: {}", e))?;
    let json_str = String::from_utf8(json_bytes)?;

    // Peek at version to decide which struct to deserialize into
    let version: serde_json::Value = serde_json::from_str(&json_str)?;
    let v = version
        .get("v")
        .and_then(|v| v.as_u64())
        .unwrap_or(1) as u32;

    match v {
        1 | 2 => {
            let payload: PairingPayload = serde_json::from_str(&json_str)?;
            Ok(ParsedPairing::Legacy(payload))
        }
        3 => {
            let payload: PairingPayloadV3 = serde_json::from_str(&json_str)?;
            Ok(ParsedPairing::V3(payload))
        }
        _ => anyhow::bail!("Unsupported pairing protocol version: {}", v),
    }
}

/// Parse a zedra://pair URI and return a legacy PairingPayload (v1/v2 only).
///
/// This is kept for backward compatibility with code that expects the old return type.
pub fn parse_pairing_uri_legacy(uri: &str) -> Result<PairingPayload> {
    match parse_pairing_uri(uri)? {
        ParsedPairing::Legacy(p) => Ok(p),
        ParsedPairing::V3(_) => anyhow::bail!("v3 pairing requires encrypted transport support"),
    }
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
