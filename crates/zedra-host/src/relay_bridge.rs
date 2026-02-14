// Relay bridge: connects zedra-host to a cloud relay server so that mobile
// clients can reach the host without direct LAN access.
//
// Flow:
// 1. Create a relay room via RelayClient
// 2. Display QR code with room info (code + secret + relay URL)
// 3. Wait for the mobile client to join
// 4. Wrap the relay connection in RelayTransport
// 5. Bridge to handle_transport_connection() for RPC dispatch

use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use zedra_relay::client::RelayClient;
use zedra_relay::transport::RelayTransport;

use crate::qr;
use crate::rpc_daemon::{handle_transport_connection, DaemonState};
use crate::session_registry::SessionRegistry;

/// Run the host in relay mode: create a room, show QR, wait for client, serve RPC.
pub async fn run_relay_mode(
    _workdir: PathBuf,
    relay_url: &str,
    registry: Arc<SessionRegistry>,
    daemon_state: Arc<DaemonState>,
) -> Result<()> {
    // 1. Create a relay room
    let room = RelayClient::create_room(relay_url).await?;
    tracing::info!("Relay room created: code={}", room.code);

    // 2. Display QR code with relay + LAN info
    let local_ip = qr::get_local_ip().unwrap_or_else(|| "unknown".to_string());
    display_relay_qr(relay_url, &room.code, &room.secret, &local_ip);

    // 3. Create RelayClient as "host" role and join the room
    let client = RelayClient::new(
        relay_url.to_string(),
        room.code.clone(),
        room.secret.clone(),
        "host".to_string(),
    );
    client.join_room().await?;
    tracing::info!("Joined relay room as host, waiting for mobile client...");

    // 4. Wait for the mobile client to join by polling signals
    wait_for_peer(&client).await?;
    tracing::info!("Mobile client connected via relay");

    // 5. Spawn a heartbeat task to keep the relay room alive
    let hb_client = RelayClient::new(
        relay_url.to_string(),
        room.code.clone(),
        room.secret.clone(),
        "host".to_string(),
    );
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            if hb_client.heartbeat().await.is_err() {
                tracing::warn!("Relay heartbeat failed");
                break;
            }
        }
    });

    // 6. Create RelayTransport and bridge to RPC handler
    let transport = RelayTransport::new(client);
    handle_transport_connection(Box::new(transport), registry, daemon_state).await
}

/// Wait for the peer (mobile) to join the relay room.
/// Polls the signal endpoint until the peer sets their signal data.
async fn wait_for_peer(client: &RelayClient) -> Result<()> {
    for _ in 0..120 {
        // 2 minutes timeout
        match client.get_signal().await {
            Ok(resp) if resp.data.is_some() => return Ok(()),
            _ => {}
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    anyhow::bail!("Timed out waiting for mobile client to join relay room")
}

/// Display a QR code with relay connection info.
fn display_relay_qr(relay_url: &str, room_code: &str, secret: &str, local_ip: &str) {
    let hostname = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "unknown".to_string());

    let payload = serde_json::json!({
        "v": 2,
        "mode": "relay",
        "relay_url": relay_url,
        "room_code": room_code,
        "secret": secret,
        "lan_ip": local_ip,
        "lan_port": 2123,
        "name": hostname,
    });

    let json = serde_json::to_string(&payload).unwrap_or_default();
    let encoded = base64_url::encode(&json);
    let uri = format!("zedra://connect?d={}", encoded);

    // Generate QR code
    if let Ok(code) = qrcode::QrCode::new(uri.as_bytes()) {
        let rendered = render_qr(&code);
        println!();
        println!("  Zedra Relay Mode");
        println!("  ================");
        println!();
        println!("  Scan this QR code with the Zedra app.");
        println!("  Host: {} (LAN: {})", hostname, local_ip);
        println!("  Relay: {}", relay_url);
        println!("  Room: {}", room_code);
        println!();
        println!("{}", rendered);
        println!();
    } else {
        println!("  Relay room created: {}", room_code);
        println!("  Relay URL: {}", relay_url);
        println!("  Connect from Zedra app using these details.");
    }
}

fn render_qr(code: &qrcode::QrCode) -> String {
    let width = code.width();
    let mut result = String::new();
    let mut row = 0;
    while row < width {
        result.push_str("    ");
        for col in 0..width {
            let top = code[(col, row)] == qrcode::types::Color::Dark;
            let bottom = if row + 1 < width {
                code[(col, row + 1)] == qrcode::types::Color::Dark
            } else {
                false
            };
            match (top, bottom) {
                (true, true) => result.push('\u{2588}'),
                (true, false) => result.push('\u{2580}'),
                (false, true) => result.push('\u{2584}'),
                (false, false) => result.push(' '),
            }
        }
        result.push('\n');
        row += 2;
    }
    result
}
