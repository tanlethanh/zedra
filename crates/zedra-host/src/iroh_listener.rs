// iroh_listener: accept incoming connections via iroh endpoint.
//
// Uses irpc typed protocol over QUIC. Each connection goes through session
// binding (first message must be ResumeOrCreate) then enters the dispatch loop.

use anyhow::Result;
use std::sync::Arc;

use crate::rpc_daemon::{self, DaemonState};
use crate::session_registry::SessionRegistry;
use zedra_rpc::proto::ZEDRA_ALPN;

use crate::identity::SharedIdentity;

/// Create and bind an iroh endpoint with the host's identity.
///
/// Returns the endpoint ready for accepting connections and QR code generation.
pub async fn create_endpoint(identity: &SharedIdentity) -> Result<iroh::Endpoint> {
    // Relay-free: direct P2P only (same LAN or routable IPs).
    let endpoint = iroh::Endpoint::builder()
        .secret_key(identity.iroh_secret_key().clone())
        .alpns(vec![ZEDRA_ALPN.to_vec()])
        .relay_mode(iroh::RelayMode::Disabled)
        .bind()
        .await?;

    tracing::info!("iroh endpoint bound: {}", endpoint.id().fmt_short());
    tracing::info!("iroh endpoint addr: {:?}", endpoint.addr());

    Ok(endpoint)
}

/// Run the iroh accept loop for incoming connections.
///
/// Each connection is dispatched to `handle_connection` which performs session
/// binding and enters the irpc dispatch loop.
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
            let accepting = match incoming.accept() {
                Ok(a) => a,
                Err(e) => {
                    tracing::warn!("iroh accept error: {}", e);
                    return;
                }
            };
            let conn = match accepting.await {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("iroh connection error: {}", e);
                    return;
                }
            };

            tracing::info!(
                "Accepted connection from {} (alpn={})",
                conn.remote_id().fmt_short(),
                String::from_utf8_lossy(conn.alpn()),
            );

            if let Err(e) = rpc_daemon::handle_connection(conn, registry, state).await {
                tracing::warn!("irpc connection error: {}", e);
            }
        });
    }

    Ok(())
}
