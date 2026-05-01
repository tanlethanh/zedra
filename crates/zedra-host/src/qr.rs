// QR code generation for device pairing.
//
// Encodes a ZedraPairingTicket (endpoint_id + handshake_secret + session_id) as a
// compact postcard+base64url string embedded in a `zedra://connect?ticket=` URL.
// The host's IP/relay information is obtained at connect time via pkarr resolution.

use anyhow::Result;
use qrcode::render::unicode;
use qrcode::{EcLevel, QrCode};
use serde::{Deserialize, Serialize};
use std::path::Path;
use zedra_rpc::ZedraPairingTicket;

use crate::utils;

/// Machine-readable startup output for `--json` mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
pub fn print_pairing_info(info: &StartupInfo) {
    println!();
    utils::println_heading("Zedra Daemon Pairing");
    println!();
    println!("Scan this QR code with the Zedra app to pair this device.");
    println!();
    utils::print_key_values(&pairing_metadata_rows(info));
    println!();
    println!("{}", info.qr_code);
    println!("{}", info.pairing_url);
    println!();
}

pub fn print_started_pairing_info(info: &StartupInfo, workdir: &Path) {
    println!("{}", render_started_pairing_info(info, workdir));
    println!();
}

fn render_started_pairing_info(info: &StartupInfo, workdir: &Path) -> String {
    let mut rows = pairing_metadata_rows(info);
    rows.push(("Workdir", workdir.display().to_string()));

    format!(
        "{}\n\n{}\n\nScan this QR code with the Zedra app to pair this device.\n\n{}\n{}",
        utils::heading_text("Zedra Daemon Started"),
        utils::render_key_values(&rows),
        info.qr_code,
        info.pairing_url
    )
}

fn pairing_metadata_rows(info: &StartupInfo) -> Vec<(&'static str, String)> {
    let regions: Vec<&str> = info.relay_urls.iter().map(|u| relay_region(u)).collect();
    vec![
        ("Relays", regions.join(", ")),
        ("Direct Addrs", info.direct_addrs.len().to_string()),
    ]
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

    #[test]
    fn pairing_metadata_keeps_only_connection_summary() {
        let info = StartupInfo {
            status: "ready".to_string(),
            host: "host".to_string(),
            endpoint_id: "endpoint".to_string(),
            relay_urls: vec![
                "https://sg1.relay.zedra.dev".to_string(),
                "https://vn1.relay.zedra.dev".to_string(),
            ],
            direct_addrs: vec!["192.0.2.1:1234".to_string()],
            pairing_url: "zedra://connect?ticket=test".to_string(),
            qr_code: "qr".to_string(),
        };

        let labels = pairing_metadata_rows(&info)
            .into_iter()
            .map(|(label, _)| label)
            .collect::<Vec<_>>();

        assert_eq!(labels, vec!["Relays", "Direct Addrs"]);
    }

    #[test]
    fn started_pairing_info_merges_daemon_and_qr_summary() {
        let info = StartupInfo {
            status: "ready".to_string(),
            host: "host".to_string(),
            endpoint_id: "endpoint".to_string(),
            relay_urls: vec!["https://sg1.relay.zedra.dev".to_string()],
            direct_addrs: vec!["192.0.2.1:1234".to_string()],
            pairing_url: "zedra://connect?ticket=test".to_string(),
            qr_code: "qr".to_string(),
        };

        let output = render_started_pairing_info(&info, Path::new("/repo"));

        assert!(output.starts_with("Zedra Daemon Started\n\n  Relays        sg1"));
        assert!(output.contains("  Direct Addrs  1"));
        assert!(output.contains("  Workdir       /repo"));
        assert!(output.contains("Scan this QR code with the Zedra app to pair this device."));
        assert!(!output.contains("Zedra Daemon Pairing"));
        assert!(!output.contains("Host"));
        assert!(!output.contains("Endpoint"));
    }
}
