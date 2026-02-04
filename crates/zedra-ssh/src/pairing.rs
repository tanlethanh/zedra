// QR code pairing protocol
// Parses zedra://pair URIs and performs the pairing handshake

use anyhow::Result;
use serde::Deserialize;

/// Pairing payload from QR code
#[derive(Debug, Clone, Deserialize)]
pub struct PairingPayload {
    /// Protocol version
    pub v: u32,
    /// Host address
    pub host: String,
    /// SSH port
    pub port: u16,
    /// One-time pairing token
    pub token: String,
    /// Expected host key fingerprint
    pub fingerprint: String,
    /// Friendly name for the host
    pub name: String,
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

    if payload.v != 1 {
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
