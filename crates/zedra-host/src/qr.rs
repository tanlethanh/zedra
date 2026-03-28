// QR code generation for device pairing.
//
// Encodes a ZedraPairingTicket (endpoint_id + handshake_secret + session_id) as a
// compact postcard+base64url string embedded in a `zedra://connect?ticket=` URL.
// The host's IP/relay information is obtained at connect time via pkarr resolution.

use anyhow::Result;
use qrcode::render::unicode;
use qrcode::{EcLevel, QrCode};
use serde::Serialize;
use zedra_rpc::ZedraPairingTicket;

/// Machine-readable startup output for `--json` mode.
#[derive(Serialize)]
pub struct StartupInfo {
    pub status: String,
    pub host: String,
    pub endpoint_id: String,
    pub relay_urls: Vec<String>,
    pub direct_addrs: Vec<String>,
    pub pairing_url: String,
    pub qr_code: String,
}

/// Build the pairing info (QR string, metadata) without printing anything.
///
/// `configured_relay_urls` are the relay URLs the endpoint was configured with.
/// These are used for display; the live `endpoint.addr()` relay list is empty
/// at startup because the relay connection is established asynchronously.
pub fn build_pairing_info(
    ticket: &ZedraPairingTicket,
    endpoint: &iroh::Endpoint,
    configured_relay_urls: &[String],
) -> Result<StartupInfo> {
    let hostname = gethostname();
    let endpoint_id = ticket.endpoint_id.to_string();

    // Direct addresses from live endpoint (populated after STUN completes).
    let addr = endpoint.addr();
    let relay_urls: Vec<String> = configured_relay_urls.to_vec();
    let direct_addrs: Vec<String> = addr.ip_addrs().map(|a| a.to_string()).collect();

    let pairing_url = ticket.to_pairing_url()?;

    tracing::info!(
        "QR pairing code: {} bytes, endpoint={}, session={}, relays={}, direct_addrs={}",
        pairing_url.len(),
        &endpoint_id[..16.min(endpoint_id.len())],
        ticket.session_id,
        relay_urls.len(),
        direct_addrs.len(),
    );

    let code = QrCode::with_error_correction_level(pairing_url.as_bytes(), EcLevel::L)?;
    let qr_code = render_qr_compact(&code);

    Ok(StartupInfo {
        status: "ready".to_string(),
        host: hostname,
        endpoint_id,
        relay_urls,
        direct_addrs,
        pairing_url,
        qr_code,
    })
}

/// Generate and display a pairing QR code.
pub fn generate_pairing_qr(
    ticket: &ZedraPairingTicket,
    endpoint: &iroh::Endpoint,
    configured_relay_urls: &[String],
) -> Result<()> {
    let info = build_pairing_info(ticket, endpoint, configured_relay_urls)?;
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
    println!(
        "  Endpoint: {}",
        &info.endpoint_id[..16.min(info.endpoint_id.len())]
    );
    let regions: Vec<&str> = info.relay_urls.iter().map(|u| relay_region(u)).collect();
    println!("  Relays: {}", regions.join(", "));
    println!("  Direct addrs: {}", info.direct_addrs.len());
    println!();
    println!("{}", info.qr_code);
    println!("{}", info.pairing_url);
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

/// Extract region label from a relay URL (e.g. "https://ap1.relay.zedra.dev" → "ap1").
/// Falls back to the full URL if the pattern doesn't match.
fn relay_region(url: &str) -> &str {
    let host = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    host.split('.').next().unwrap_or(host)
}

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
