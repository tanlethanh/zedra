// Relay bridge: helpers for registering a relay room and accepting relay
// connections alongside the LAN TCP listener.
//
// The relay is an optional transport — if the host can reach the relay server
// it registers a room and embeds the room credentials in the QR code. Clients
// can then connect via relay if LAN is unavailable.

use anyhow::Result;
use std::sync::Arc;
use std::time::Duration;

use zedra_relay::client::RelayClient;
use zedra_relay::transport::RelayTransport;

use crate::qr::RelayInfo;
use crate::rpc_daemon::{handle_transport_connection, DaemonState};
use crate::session_registry::SessionRegistry;

/// Try to register a relay room. Returns `Some(RelayInfo)` on success,
/// `None` if the relay server is unreachable (non-fatal).
pub async fn try_register_relay(relay_url: &str) -> Option<RelayInfo> {
    match RelayClient::create_room(relay_url).await {
        Ok(room) => {
            tracing::info!("Relay room registered: {}", room.code);
            Some(RelayInfo {
                relay_url: relay_url.to_string(),
                room_code: room.code,
                secret: room.secret,
            })
        }
        Err(e) => {
            tracing::warn!("Relay unavailable ({}), running LAN-only", e);
            None
        }
    }
}

/// Accept relay connections in a loop. Joins the relay room as "host", waits
/// for a peer, then dispatches to `handle_transport_connection`. Repeats for
/// the next client after each connection ends.
pub async fn accept_relay_connections(
    relay_info: RelayInfo,
    registry: Arc<SessionRegistry>,
    daemon_state: Arc<DaemonState>,
) {
    loop {
        if let Err(e) = accept_one_relay(
            &relay_info,
            registry.clone(),
            daemon_state.clone(),
        )
        .await
        {
            tracing::error!("Relay connection error: {}", e);
            // Brief pause before retrying
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }
}

/// Accept a single relay client connection and serve it.
async fn accept_one_relay(
    relay_info: &RelayInfo,
    registry: Arc<SessionRegistry>,
    daemon_state: Arc<DaemonState>,
) -> Result<()> {
    let client = RelayClient::new(
        relay_info.relay_url.clone(),
        relay_info.room_code.clone(),
        relay_info.secret.clone(),
        "host".to_string(),
    );
    client.join_room().await?;
    tracing::info!("Joined relay room as host, waiting for client...");

    // Wait for a peer to join (up to 10 minutes)
    wait_for_peer(&client).await?;
    tracing::info!("Client connected via relay");

    // Spawn a heartbeat to keep the room alive during this connection
    let hb_client = RelayClient::new(
        relay_info.relay_url.clone(),
        relay_info.room_code.clone(),
        relay_info.secret.clone(),
        "host".to_string(),
    );
    let hb_handle = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            if hb_client.heartbeat().await.is_err() {
                tracing::warn!("Relay heartbeat failed");
                break;
            }
        }
    });

    let transport = RelayTransport::new(client);
    let result = handle_transport_connection(Box::new(transport), registry, daemon_state).await;

    hb_handle.abort();
    result
}

/// Poll the signal endpoint until the peer sets their signal data.
async fn wait_for_peer(client: &RelayClient) -> Result<()> {
    for _ in 0..600 {
        // 10 minutes timeout
        match client.get_signal().await {
            Ok(resp) if resp.data.is_some() => return Ok(()),
            _ => {}
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    anyhow::bail!("Timed out waiting for client to join relay room")
}
