// QR code generation for device pairing
// Generates a QR code containing connection info + one-time pairing token

use anyhow::Result;
use qrcode::render::unicode;
use qrcode::{EcLevel, QrCode};
use serde::Serialize;

use crate::auth;
use crate::identity::SharedIdentity;

/// Relay info included in the QR code when relay is available.
pub struct RelayInfo {
    pub relay_url: String,
    pub room_code: String,
    pub secret: String,
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
    fn test_gethostname_returns_nonempty() {
        let name = gethostname();
        assert!(!name.is_empty());
    }

    #[test]
    fn test_qr_code_from_uri() {
        let uri = "zedra://pair?d=eyJ2IjoxfQ";
        let code = QrCode::new(uri.as_bytes());
        assert!(code.is_ok());
    }
}
