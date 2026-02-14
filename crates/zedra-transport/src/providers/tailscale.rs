use anyhow::{Context, Result};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::time::timeout;
use zedra_rpc::TcpTransport;

use super::TransportProvider;

/// Tailscale transport provider. Connects via Tailscale's 100.x.x.x address.
pub struct TailscaleProvider {
    addr: String,
    port: u16,
}

impl TailscaleProvider {
    pub fn new(addr: String, port: u16) -> Self {
        Self { addr, port }
    }
}

#[async_trait::async_trait]
impl TransportProvider for TailscaleProvider {
    fn name(&self) -> &str {
        "tailscale"
    }

    async fn connect(&self) -> Result<Box<dyn zedra_rpc::Transport>> {
        let target = format!("{}:{}", self.addr, self.port);
        log::debug!("Tailscale: trying {}", target);

        let stream = timeout(Duration::from_secs(2), TcpStream::connect(&target))
            .await
            .map_err(|_| anyhow::anyhow!("timeout connecting to {}", target))?
            .context("Tailscale: connection failed")?;

        log::info!("Tailscale: connected to {}", target);
        Ok(Box::new(TcpTransport::new(stream)))
    }

    fn priority(&self) -> u32 {
        1
    }
}
