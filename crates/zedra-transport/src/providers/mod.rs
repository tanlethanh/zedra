pub mod lan;
pub mod relay;
pub mod tailscale;

use anyhow::Result;
use zedra_rpc::Transport;

/// A transport provider can attempt to establish a connection to a peer.
#[async_trait::async_trait]
pub trait TransportProvider: Send + Sync {
    /// Human-readable name.
    fn name(&self) -> &str;
    /// Attempt connection. Returns a boxed Transport on success.
    async fn connect(&self) -> Result<Box<dyn Transport>>;
    /// Priority (lower = preferred). LAN=0, Tailscale=1, Relay=2.
    fn priority(&self) -> u32;
}
