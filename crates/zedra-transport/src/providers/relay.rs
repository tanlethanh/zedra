use anyhow::{Context, Result};
use zedra_relay::client::RelayClient;
use zedra_relay::transport::RelayTransport;

use super::TransportProvider;

/// Relay transport provider. Connects via the HTTP relay server.
pub struct RelayProvider {
    relay_url: String,
    room_code: String,
    secret: String,
}

impl RelayProvider {
    pub fn new(relay_url: String, room_code: String, secret: String) -> Self {
        Self {
            relay_url,
            room_code,
            secret,
        }
    }
}

#[async_trait::async_trait]
impl TransportProvider for RelayProvider {
    fn name(&self) -> &str {
        "relay"
    }

    async fn connect(&self) -> Result<Box<dyn zedra_rpc::Transport>> {
        let client = RelayClient::new(
            self.relay_url.clone(),
            self.room_code.clone(),
            self.secret.clone(),
            "mobile".to_string(),
        );

        log::debug!("Relay: joining room {}", self.room_code);
        client
            .join_room()
            .await
            .context("Relay: failed to join room")?;

        log::info!("Relay: connected via room {}", self.room_code);
        Ok(Box::new(RelayTransport::new(client)))
    }

    fn priority(&self) -> u32 {
        2
    }
}
