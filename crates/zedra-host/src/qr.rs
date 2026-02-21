// QR code generation for device pairing (iroh)

use anyhow::Result;
use qrcode::render::unicode;
use qrcode::{EcLevel, QrCode};
use serde::Serialize;

use crate::identity::SharedIdentity;

/// Pairing payload with iroh EndpointId.
#[derive(Serialize)]
struct PairingPayload {
    v: u32,
    /// iroh EndpointId (z-base-32 encoded Ed25519 public key)
    endpoint_id: String,
    /// Friendly hostname
    name: String,
    /// iroh relay URL the host is connected to
    #[serde(skip_serializing_if = "Option::is_none")]
    relay_url: Option<String>,
    /// Direct addresses (LAN IPs with iroh UDP port)
    addrs: Vec<String>,
    /// Coordination server URL (for CF Worker discovery)
    #[serde(skip_serializing_if = "Option::is_none")]
    coord_url: Option<String>,
}

/// Machine-readable startup output for `--json` mode.
#[derive(Serialize)]
pub struct StartupInfo {
    pub status: String,
    pub host: String,
    pub endpoint_id: String,
    pub device_id: String,
    pub relay_url: Option<String>,
    pub direct_addrs: Vec<String>,
    pub pairing_uri: String,
    pub qr_code: String,
}

/// Build the pairing info (URI, QR string, metadata) without printing anything.
pub fn build_pairing_info(
    endpoint_info: &crate::iroh_listener::EndpointQrInfo,
    identity: &SharedIdentity,
    coord_url: Option<&str>,
) -> Result<StartupInfo> {
    let hostname = gethostname();

    let payload = PairingPayload {
        v: 1,
        endpoint_id: endpoint_info.endpoint_id.clone(),
        name: hostname.clone(),
        relay_url: endpoint_info.relay_url.clone(),
        addrs: endpoint_info.direct_addrs.clone(),
        coord_url: coord_url.map(|s| s.to_string()),
    };

    let json = serde_json::to_string(&payload)?;
    let encoded = base64_url::encode(&json);
    let uri = format!("zedra://pair?d={}", encoded);

    let code = QrCode::with_error_correction_level(uri.as_bytes(), EcLevel::L)?;
    let qr_string = render_qr_compact(&code);

    Ok(StartupInfo {
        status: "ready".to_string(),
        host: hostname,
        endpoint_id: endpoint_info.endpoint_id.clone(),
        device_id: identity.device_id.short().to_string(),
        relay_url: endpoint_info.relay_url.clone(),
        direct_addrs: endpoint_info.direct_addrs.clone(),
        pairing_uri: uri,
        qr_code: qr_string,
    })
}

/// Generate and display a pairing QR code for iroh-based connections.
///
/// The QR includes the iroh EndpointId (Ed25519 public key) which is both
/// the host's identity and the addressing key for iroh connections.
pub fn generate_pairing_qr(
    endpoint_info: &crate::iroh_listener::EndpointQrInfo,
    identity: &SharedIdentity,
    coord_url: Option<&str>,
) -> Result<()> {
    let info = build_pairing_info(endpoint_info, identity, coord_url)?;
    print_pairing_info(&info);
    Ok(())
}

/// Print pairing info in human-readable format.
fn print_pairing_info(info: &StartupInfo) {
    println!();
    println!("  Zedra Host Pairing");
    println!("  ==================");
    println!();
    println!("  Scan this QR code with the Zedra app to pair this device.");
    println!("  Host: {}", info.host);
    println!("  Endpoint: {}", &info.endpoint_id[..16]);
    println!("  Device ID: {}", info.device_id);
    if let Some(ref relay) = info.relay_url {
        println!("  Relay: {}", relay);
    }
    println!("  Direct addrs: {}", info.direct_addrs.len());
    println!();
    println!("{}", info.qr_code);
    println!();
}

/// Print pairing info as a single JSON line to stdout.
pub fn print_pairing_json(info: &StartupInfo) {
    if let Ok(json) = serde_json::to_string(info) {
        println!("{}", json);
    }
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
