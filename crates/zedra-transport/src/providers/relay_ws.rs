// WebSocket Relay Transport Provider
//
// Connects to the relay server via WebSocket for persistent bidirectional
// communication. Priority 2 (same as HTTP relay, but preferred when available
// because it has lower latency and no polling overhead).

use anyhow::Result;
use zedra_relay::ws_transport::{WsRelayTransport, build_ws_url};

use super::TransportProvider;

/// WebSocket relay transport provider.
pub struct WsRelayProvider {
    relay_url: String,
    room_code: String,
    secret: String,
}

impl WsRelayProvider {
    pub fn new(relay_url: String, room_code: String, secret: String) -> Self {
        Self {
            relay_url,
            room_code,
            secret,
        }
    }
}

#[async_trait::async_trait]
impl TransportProvider for WsRelayProvider {
    fn name(&self) -> &str {
        "relay-ws"
    }

    async fn connect(&self) -> Result<Box<dyn zedra_rpc::Transport>> {
        let ws_url = build_ws_url(&self.relay_url, &self.room_code, &self.secret, "mobile");
        log::debug!("WsRelayProvider: connecting to {}", ws_url);

        let transport = WsRelayTransport::connect(&ws_url).await?;
        log::info!("WsRelayProvider: connected via WebSocket");

        Ok(Box::new(transport))
    }

    fn priority(&self) -> u32 {
        // Same tier as HTTP relay, but discovery will prefer WS when both connect
        // because WS connects first (no polling delay).
        2
    }
}
