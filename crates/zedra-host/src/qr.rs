// QR code generation for device pairing
// Generates a QR code containing connection info + one-time pairing token

use anyhow::Result;
use qrcode::render::unicode;
use qrcode::{EcLevel, QrCode};
use serde::Serialize;

use crate::auth;
use crate::identity::SharedIdentity;
use crate::store;

/// Relay info included in the QR code when relay is available.
pub struct RelayInfo {
    pub relay_url: String,
    pub room_code: String,
    pub secret: String,
}

/// Pairing payload encoded in the QR code.
///
/// Always includes LAN addresses. Relay fields are present only when the
/// host successfully registered a relay room.
#[derive(Serialize)]
struct PairingPayload {
    v: u32,
    host: String,
    port: u16,
    token: String,
    fingerprint: String,
    name: String,
    host_addrs: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tailscale_addr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    relay_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    relay_room: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    relay_secret: Option<String>,
}

/// v3 pairing payload with Curve25519 host public key for Noise_IK handshake.
#[derive(Serialize)]
struct PairingPayloadV3 {
    v: u32,
    /// Base64url-encoded 32-byte Curve25519 public key
    host_pubkey: String,
    /// One-time pairing token (replaces v2's reusable `token`)
    otp: String,
    /// Friendly hostname
    name: String,
    /// All LAN addresses
    host_addrs: Vec<String>,
    /// Port
    port: u16,
    /// Coordination server URL (future use)
    #[serde(skip_serializing_if = "Option::is_none")]
    coord_url: Option<String>,
    /// Connection hints
    hints: PairingHints,
}

#[derive(Serialize)]
struct PairingHints {
    addrs: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tailscale: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    relay: Option<String>,
}

/// Generate and display a v3 pairing QR code with Noise_IK encryption support.
///
/// The QR includes the host's Curve25519 public key for the Noise_IK handshake
/// and a one-time pairing token for first-contact authentication.
pub fn generate_pairing_qr_v3(
    port: u16,
    identity: &SharedIdentity,
    relay: Option<&RelayInfo>,
) -> Result<()> {
    let hostname = gethostname();
    let primary_ip = get_local_ip().unwrap_or_else(|| "localhost".to_string());
    let otp = auth::create_pairing_token();
    let host_addrs = collect_lan_addrs();
    let addrs = if host_addrs.is_empty() {
        vec![primary_ip.clone()]
    } else {
        host_addrs.clone()
    };

    let payload = PairingPayloadV3 {
        v: 3,
        host_pubkey: base64_url::encode(&identity.public_key_bytes()),
        otp,
        name: hostname.clone(),
        host_addrs: addrs.clone(),
        port,
        coord_url: None, // Phase 3: coordination server
        hints: PairingHints {
            addrs: addrs.iter().map(|a| format!("{}:{}", a, port)).collect(),
            tailscale: None,
            relay: relay.map(|r| format!("{}?room={}", r.relay_url, r.room_code)),
        },
    };

    let json = serde_json::to_string(&payload)?;
    let encoded = base64_url::encode(&json);
    let uri = format!("zedra://pair?d={}", encoded);

    let code = QrCode::with_error_correction_level(uri.as_bytes(), EcLevel::L)?;
    let qr_string = render_qr_compact(&code);

    let addrs_display = if addrs.len() == 1 {
        addrs[0].clone()
    } else {
        addrs.join(", ")
    };

    println!();
    println!("  Zedra Host Pairing (v3 - Encrypted)");
    println!("  ====================================");
    println!();
    println!("  Scan this QR code with the Zedra app to pair this device.");
    println!("  Host: {} ({})", hostname, addrs_display);
    println!("  Port: {}", port);
    println!("  Device ID: {}", identity.device_id.short());
    print!("  Transports: LAN (encrypted)");
    if let Some(r) = relay {
        print!(" + Relay (room: {})", r.room_code);
    }
    println!();
    println!("  OTP expires in 5 minutes.");
    println!();
    println!("{}", qr_string);
    println!();

    Ok(())
}

/// Generate and display a pairing QR code (v2 format, legacy/plaintext).
///
/// Always includes LAN addresses. If `relay` is provided, the relay room
/// credentials are embedded so the client can fall back to relay transport.
pub fn generate_pairing_qr(port: u16, relay: Option<&RelayInfo>) -> Result<()> {
    let hostname = gethostname();
    let primary_ip = get_local_ip().unwrap_or_else(|| "localhost".to_string());
    let fingerprint = get_host_fingerprint()?;
    let token = auth::create_pairing_token();

    let host_addrs = collect_lan_addrs();

    let payload = PairingPayload {
        v: 2,
        host: primary_ip.clone(),
        port,
        token,
        fingerprint,
        name: hostname.clone(),
        host_addrs: if host_addrs.is_empty() {
            vec![primary_ip.clone()]
        } else {
            host_addrs.clone()
        },
        tailscale_addr: None,
        relay_url: relay.map(|r| r.relay_url.clone()),
        relay_room: relay.map(|r| r.room_code.clone()),
        relay_secret: relay.map(|r| r.secret.clone()),
    };

    let json = serde_json::to_string(&payload)?;
    let encoded = base64_url::encode(&json);
    let uri = format!("zedra://pair?d={}", encoded);

    let code = QrCode::with_error_correction_level(uri.as_bytes(), EcLevel::L)?;
    let qr_string = render_qr_compact(&code);

    let addrs_display = if host_addrs.is_empty() {
        primary_ip.clone()
    } else {
        host_addrs.join(", ")
    };

    println!();
    println!("  Zedra Host Pairing");
    println!("  ==================");
    println!();
    println!("  Scan this QR code with the Zedra app to pair this device.");
    println!("  Host: {} ({})", hostname, addrs_display);
    println!("  Port: {}", port);
    print!("  Transports: LAN");
    if let Some(r) = relay {
        print!(" + Relay (room: {})", r.room_code);
    }
    println!();
    println!("  Token expires in 5 minutes.");
    println!();
    println!("{}", qr_string);
    println!();
    println!("  Or connect manually:");
    println!("    Host: {}", primary_ip);
    println!("    Port: {}", port);
    println!("    Username: zedra");
    println!();

    Ok(())
}

/// Collect all non-loopback IPv4 addresses on the host.
pub fn collect_lan_addrs() -> Vec<String> {
    if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|iface| !iface.is_loopback())
        .filter_map(|iface| match iface.ip() {
            std::net::IpAddr::V4(v4) => Some(v4.to_string()),
            _ => None,
        })
        .collect()
}

/// Render a compact QR code for terminal display using the qrcode crate's
/// built-in Dense1x2 Unicode renderer (two rows per character via half-blocks).
fn render_qr_compact(code: &QrCode) -> String {
    code.render::<unicode::Dense1x2>()
        .dark_color(unicode::Dense1x2::Dark)
        .light_color(unicode::Dense1x2::Light)
        .quiet_zone(true)
        .build()
}

/// Get the host key fingerprint
fn get_host_fingerprint() -> Result<String> {
    let key_path = store::host_key_path()?;
    if !key_path.exists() {
        // Generate host key on first run
        generate_host_key(&key_path)?;
    }

    let key_data = std::fs::read(&key_path)?;
    let key = ssh_key::PrivateKey::from_openssh(&key_data)
        .map_err(|e| anyhow::anyhow!("Failed to read host key: {}", e))?;
    let public_key = key.public_key();
    let fingerprint = public_key.fingerprint(ssh_key::HashAlg::Sha256);
    Ok(fingerprint.to_string())
}

/// Generate an Ed25519 host key
fn generate_host_key(path: &std::path::Path) -> Result<()> {
    tracing::info!("Generating host key at {:?}", path);

    let key = ssh_key::PrivateKey::random(&mut rand::thread_rng(), ssh_key::Algorithm::Ed25519)
        .map_err(|e| anyhow::anyhow!("Failed to generate key: {}", e))?;

    let openssh = key
        .to_openssh(ssh_key::LineEnding::LF)
        .map_err(|e| anyhow::anyhow!("Failed to serialize key: {}", e))?;

    std::fs::write(path, openssh.as_bytes())?;

    // Set permissions to 600
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }

    Ok(())
}

/// Get local hostname
fn gethostname() -> String {
    hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Get local IP address
pub fn get_local_ip() -> Option<String> {
    // Try to find a non-loopback IPv4 address
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    let addr = socket.local_addr().ok()?;
    Some(addr.ip().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_qr_compact() {
        let code = QrCode::new(b"test").unwrap();
        let rendered = render_qr_compact(&code);
        assert!(!rendered.is_empty());
        assert!(rendered.contains('\n'));
    }

    #[test]
    fn test_pairing_payload_lan_only() {
        let payload = PairingPayload {
            v: 2,
            host: "192.168.1.1".to_string(),
            port: 2123,
            token: "abc123".to_string(),
            fingerprint: "SHA256:xxxx".to_string(),
            name: "my-machine".to_string(),
            host_addrs: vec!["192.168.1.1".to_string()],
            tailscale_addr: None,
            relay_url: None,
            relay_room: None,
            relay_secret: None,
        };

        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("192.168.1.1"));
        assert!(json.contains("2123"));
        assert!(json.contains("abc123"));
        // relay fields should be omitted when None
        assert!(!json.contains("relay_url"));
        assert!(!json.contains("relay_room"));

        let encoded = base64_url::encode(&json);
        let decoded = base64_url::decode(&encoded).unwrap();
        let decoded_str = String::from_utf8(decoded).unwrap();
        assert_eq!(decoded_str, json);
    }

    #[test]
    fn test_pairing_payload_with_relay() {
        let payload = PairingPayload {
            v: 2,
            host: "10.0.0.1".to_string(),
            port: 2123,
            token: "token".to_string(),
            fingerprint: "fp".to_string(),
            name: "host".to_string(),
            host_addrs: vec!["10.0.0.1".to_string()],
            tailscale_addr: None,
            relay_url: Some("https://relay.zedra.dev".to_string()),
            relay_room: Some("ABC123".to_string()),
            relay_secret: Some("secret".to_string()),
        };

        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("relay_url"));
        assert!(json.contains("ABC123"));
    }

    #[test]
    fn test_pairing_uri_format() {
        let payload = PairingPayload {
            v: 2,
            host: "10.0.0.1".to_string(),
            port: 2123,
            token: "token".to_string(),
            fingerprint: "fp".to_string(),
            name: "host".to_string(),
            host_addrs: vec!["10.0.0.1".to_string()],
            tailscale_addr: None,
            relay_url: None,
            relay_room: None,
            relay_secret: None,
        };

        let json = serde_json::to_string(&payload).unwrap();
        let encoded = base64_url::encode(&json);
        let uri = format!("zedra://pair?d={}", encoded);

        assert!(uri.starts_with("zedra://pair?d="));
        let data_part = uri.strip_prefix("zedra://pair?d=").unwrap();
        assert!(!data_part.contains('+'));
        assert!(!data_part.contains('/'));
    }

    #[test]
    fn test_qr_code_from_uri() {
        let uri = "zedra://pair?d=eyJ2IjoxfQ";
        let code = QrCode::new(uri.as_bytes());
        assert!(code.is_ok());
    }

    #[test]
    fn test_gethostname_returns_nonempty() {
        let name = gethostname();
        assert!(!name.is_empty());
    }
}
