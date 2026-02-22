// QR code generation for device pairing (iroh)
//
// Encodes the host's EndpointAddr as a compact postcard+base64-url string.
// Hostname and other metadata are discovered post-connection via GetSessionInfo.

use anyhow::Result;
use qrcode::render::unicode;
use qrcode::{EcLevel, QrCode};
use serde::Serialize;

/// Machine-readable startup output for `--json` mode.
#[derive(Serialize)]
pub struct StartupInfo {
    pub status: String,
    pub host: String,
    pub endpoint_id: String,
    pub relay_url: Option<String>,
    pub direct_addrs: Vec<String>,
    pub pairing_code: String,
    pub qr_code: String,
}

/// Build the pairing info (QR string, metadata) without printing anything.
pub fn build_pairing_info(addr: &iroh::EndpointAddr) -> Result<StartupInfo> {
    let hostname = gethostname();
    let endpoint_id = addr.id.to_string();
    let relay_url = addr.relay_urls().next().map(|u| u.to_string());
    let direct_addrs: Vec<String> = addr.ip_addrs().map(|a| a.to_string()).collect();

    let pairing_code = zedra_rpc::encode_endpoint_addr(addr)?;

    let code = QrCode::with_error_correction_level(pairing_code.as_bytes(), EcLevel::L)?;
    let qr_string = render_qr_compact(&code);

    Ok(StartupInfo {
        status: "ready".to_string(),
        host: hostname,
        endpoint_id,
        relay_url,
        direct_addrs,
        pairing_code,
        qr_code: qr_string,
    })
}

/// Generate and display a pairing QR code for iroh-based connections.
pub fn generate_pairing_qr(addr: &iroh::EndpointAddr) -> Result<()> {
    let info = build_pairing_info(addr)?;
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

/// Render a compact QR code for terminal display.
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
}
