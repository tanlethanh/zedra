use anyhow::Result;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::time::timeout;
use zedra_rpc::TcpTransport;

use super::TransportProvider;

/// LAN TCP transport provider. Tries each address with a 2s timeout.
pub struct LanProvider {
    addrs: Vec<String>,
    port: u16,
}

impl LanProvider {
    pub fn new(addrs: Vec<String>, port: u16) -> Self {
        Self { addrs, port }
    }

    /// Quick probe: checks if any LAN address is reachable (TCP connect only, no framing).
    /// Returns true if at least one address accepts a TCP connection within 500ms.
    pub async fn probe(&self) -> bool {
        for addr in &self.addrs {
            let target = format!("{}:{}", addr, self.port);
            if let Ok(Ok(_)) = timeout(Duration::from_millis(500), TcpStream::connect(&target)).await
            {
                return true;
            }
        }
        false
    }
}

#[async_trait::async_trait]
impl TransportProvider for LanProvider {
    fn name(&self) -> &str {
        "lan-tcp"
    }

    async fn connect(&self) -> Result<Box<dyn zedra_rpc::Transport>> {
        let mut last_err = None;

        for addr in &self.addrs {
            let target = format!("{}:{}", addr, self.port);
            log::debug!("LAN: trying {}", target);

            match timeout(Duration::from_secs(2), TcpStream::connect(&target)).await {
                Ok(Ok(stream)) => {
                    log::info!("LAN: connected to {}", target);
                    return Ok(Box::new(TcpTransport::new(stream)));
                }
                Ok(Err(e)) => {
                    log::debug!("LAN: failed to connect to {}: {}", target, e);
                    last_err = Some(e.into());
                }
                Err(_) => {
                    log::debug!("LAN: timeout connecting to {}", target);
                    last_err = Some(anyhow::anyhow!("timeout connecting to {}", target));
                }
            }
        }

        Err(last_err
            .unwrap_or_else(|| anyhow::anyhow!("no LAN addresses to try"))
            .context("LAN provider: all addresses failed"))
    }

    fn priority(&self) -> u32 {
        0
    }
}
