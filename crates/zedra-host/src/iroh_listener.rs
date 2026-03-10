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

/// Build a relay map pointing at the self-hosted Zedra relay (Singapore).
fn zedra_relay_map() -> iroh::RelayMap {
    let url: iroh::RelayUrl = zedra_rpc::ZEDRA_RELAY_URL.parse().expect("valid relay url");
    iroh::RelayMap::from_iter([iroh::RelayConfig {
        url,
        quic: Some(iroh_relay::RelayQuicConfig::default()), // QUIC addr discovery on port 7842
    }])
}

/// Create and bind an iroh endpoint with the host's identity.
///
/// Returns the endpoint ready for accepting connections and QR code generation.
pub async fn create_endpoint(identity: &SharedIdentity) -> Result<iroh::Endpoint> {
    // Use the self-hosted Singapore relay for low-latency fallback.
    // pkarr still publishes the host's address for direct connection attempts.
    let endpoint = iroh::Endpoint::builder()
        .secret_key(identity.iroh_secret_key().clone())
        .alpns(vec![ZEDRA_ALPN.to_vec()])
        .relay_mode(iroh::RelayMode::Custom(zedra_relay_map()))
        .address_lookup(iroh::address_lookup::PkarrPublisher::n0_dns())
        .bind()
        .await?;

    tracing::info!("iroh endpoint bound: {}", endpoint.id().fmt_short());
    tracing::info!("iroh endpoint addr: {:?}", endpoint.addr());

    // Log the STUN result once it arrives (async, ~1-2s after bind).
    // global_v4/v6 = STUN-discovered public IP; mapping_varies_by_dest = symmetric NAT flag.
    {
        use iroh::Watcher;
        let mut watcher = endpoint.net_report();
        tokio::spawn(async move {
            loop {
                let report = watcher.get();
                if let Some(ref r) = report {
                    tracing::info!(
                        "net_report: global_v4={:?} global_v6={:?} mapping_varies={:?} preferred_relay={:?}",
                        r.global_v4,
                        r.global_v6,
                        r.mapping_varies_by_dest(),
                        r.preferred_relay,
                    );
                    break;
                }
                if tokio::time::timeout(
                    std::time::Duration::from_secs(10),
                    watcher.updated(),
                )
                .await
                .is_err()
                {
                    tracing::warn!("net_report: STUN did not complete within 10s (no public IP discovered)");
                    break;
                }
            }
        });
    }

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
