// iroh_listener: accept incoming connections via iroh's unified Endpoint.
//
// This single listener handles LAN, Tailscale, hole-punched, and relay
// connections — all through iroh's Endpoint::accept(). No separate TCP
// or WS listener needed.

use anyhow::Result;
use std::sync::Arc;

use crate::identity::SharedIdentity;
use crate::rpc_daemon::{self, DaemonState};
use crate::session_registry::SessionRegistry;
use zedra_transport::{CfWorkerDiscovery, IrohTransport};

/// ALPN protocol identifier for Zedra RPC over iroh.
const ZEDRA_ALPN: &[u8] = b"zedra/rpc/1";

/// Create and bind an iroh endpoint with the host's identity.
///
/// Returns the endpoint ready for `accept()` calls and QR code generation.
pub async fn create_endpoint(
    identity: &SharedIdentity,
    coord_url: Option<&str>,
) -> Result<iroh::Endpoint> {
    let mut builder = iroh::Endpoint::builder()
        .secret_key(identity.iroh_secret_key().clone())
        .alpns(vec![ZEDRA_ALPN.to_vec()]);

    // Add CF Worker discovery if coord URL is available
    if let Some(url) = coord_url {
        builder = builder.address_lookup(CfWorkerDiscovery::new(url));
    }

    let endpoint = builder.bind().await?;

    tracing::info!("iroh endpoint bound: {}", endpoint.id().fmt_short());
    tracing::info!("iroh endpoint addr: {:?}", endpoint.addr());

    Ok(endpoint)
}

/// Run the iroh accept loop on a pre-bound endpoint.
///
/// This is the main blocking call for the daemon — it accepts connections
/// until the endpoint is closed.
pub async fn run_accept_loop(
    endpoint: &iroh::Endpoint,
    registry: Arc<SessionRegistry>,
    state: Arc<DaemonState>,
) -> Result<()> {
    loop {
        let incoming = match endpoint.accept().await {
            Some(incoming) => incoming,
            None => {
                tracing::info!("iroh endpoint closed");
                break;
            }
        };

        let registry = registry.clone();
        let state = state.clone();

        tokio::spawn(async move {
            if let Err(e) = handle_incoming(incoming, registry, state).await {
                tracing::warn!("iroh connection error: {}", e);
            }
        });
    }

    Ok(())
}

/// Run the iroh listener — creates endpoint and accepts connections.
///
/// Convenience wrapper that combines `create_endpoint` + `run_accept_loop`.
pub async fn run_iroh_listener(
    identity: SharedIdentity,
    registry: Arc<SessionRegistry>,
    state: Arc<DaemonState>,
    coord_url: Option<&str>,
) -> Result<()> {
    let endpoint = create_endpoint(&identity, coord_url).await?;

    // Publish endpoint address to coordination server
    if let Some(url) = coord_url {
        let publish_url = url.to_string();
        let publish_endpoint = endpoint.clone();
        tokio::spawn(async move {
            run_publish_loop(&publish_url, &publish_endpoint).await;
        });
    }

    run_accept_loop(&endpoint, registry, state).await
}

/// Handle a single incoming iroh connection.
async fn handle_incoming(
    incoming: iroh::endpoint::Incoming,
    registry: Arc<SessionRegistry>,
    state: Arc<DaemonState>,
) -> Result<()> {
    let accepting = incoming.accept()?;
    let conn = accepting.await?;

    let remote = conn.remote_id();
    tracing::info!(
        "iroh: accepted connection from {}",
        remote.fmt_short()
    );

    // Accept a bidirectional stream for RPC
    let (send, recv) = conn.accept_bi().await?;
    let transport = IrohTransport::new(send, recv);

    rpc_daemon::handle_transport_connection(Box::new(transport), registry, state).await
}

/// Periodically publish endpoint addressing info to the CF Worker.
pub async fn run_publish_loop(coord_url: &str, endpoint: &iroh::Endpoint) {
    let endpoint_id = endpoint.id();

    loop {
        let addr = endpoint.addr();

        let relay_url: Option<iroh::RelayUrl> = addr.relay_urls().next().cloned();
        let direct_addrs: Vec<std::net::SocketAddr> = addr
            .ip_addrs()
            .cloned()
            .collect();

        if let Err(e) = zedra_transport::cf_discovery::publish_endpoint(
            coord_url,
            &endpoint_id,
            relay_url.as_ref(),
            &direct_addrs,
        )
        .await
        {
            tracing::warn!("Failed to publish endpoint to coord server: {}", e);
        } else {
            tracing::debug!(
                "Published endpoint {} ({} direct addrs, relay: {})",
                endpoint_id.fmt_short(),
                direct_addrs.len(),
                relay_url.as_ref().map(|u| u.to_string()).unwrap_or_else(|| "none".into()),
            );
        }

        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
    }
}

/// Get the iroh endpoint's address info for QR code generation.
pub fn get_endpoint_info(endpoint: &iroh::Endpoint) -> EndpointQrInfo {
    let addr = endpoint.addr();
    let relay_url = addr.relay_urls().next().map(|u| u.to_string());
    let direct_addrs: Vec<String> = addr
        .ip_addrs()
        .map(|a| a.to_string())
        .collect();

    EndpointQrInfo {
        endpoint_id: endpoint.id().to_string(),
        relay_url,
        direct_addrs,
    }
}

/// Info needed for QR code generation from an iroh endpoint.
pub struct EndpointQrInfo {
    pub endpoint_id: String,
    pub relay_url: Option<String>,
    pub direct_addrs: Vec<String>,
}
