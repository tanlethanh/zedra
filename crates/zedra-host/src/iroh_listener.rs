// iroh_listener: accept incoming connections via iroh endpoint.
//
// Uses irpc typed protocol over QUIC. Each connection goes through session
// binding (first message must be ResumeOrCreate) then enters the dispatch loop.

use anyhow::Result;
use std::sync::Arc;

use crate::rpc_daemon::{self, DaemonState};
use crate::session_registry::SessionRegistry;
use zedra_rpc::proto::ZEDRA_ALPN;

use crate::analytics::Analytics;
use crate::identity::SharedIdentity;

fn ts() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let s = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    format!("{:02}:{:02}:{:02}", (s % 86400) / 3600, (s % 3600) / 60, s % 60)
}

/// Build a relay map from the given URL.
fn relay_map_from_url(url_str: &str) -> Result<iroh::RelayMap> {
    let url: iroh::RelayUrl = url_str.parse()?;
    Ok(iroh::RelayMap::from_iter([iroh::RelayConfig {
        url,
        quic: Some(iroh_relay::RelayQuicConfig::default()), // QUIC addr discovery on port 7842
    }]))
}

/// Create and bind an iroh endpoint with the host's identity.
///
/// `relay_url` overrides the default relay; falls back to `ZEDRA_RELAY_URL`.
/// Returns the endpoint ready for accepting connections and QR code generation.
pub async fn create_endpoint(
    identity: &SharedIdentity,
    relay_url: Option<&str>,
    analytics: std::sync::Arc<Analytics>,
) -> Result<iroh::Endpoint> {
    let relay_mode = iroh::RelayMode::Custom(
        relay_map_from_url(relay_url.unwrap_or(zedra_rpc::ZEDRA_RELAY_URL))?
    );
    let endpoint = iroh::Endpoint::builder()
        .secret_key(identity.iroh_secret_key().clone())
        .alpns(vec![ZEDRA_ALPN.to_vec()])
        .relay_mode(relay_mode)
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
                    let sym_nat = r.mapping_varies_by_dest().unwrap_or(false);
                    let has_ipv4 = r.global_v4.is_some();
                    let has_ipv6 = r.global_v6.is_some();
                    analytics.net_report(has_ipv4, has_ipv6, sym_nat);
                    match (r.global_v4, r.global_v6) {
                        (None, None) => eprintln!("[{}] network:  no public IP found — relay only", ts()),
                        (v4, v6) => {
                            let addr = v4.map(|a| a.to_string())
                                .or_else(|| v6.map(|a| a.to_string()))
                                .unwrap_or_default();
                            if sym_nat {
                                eprintln!("[{}] network:  {} (symmetric NAT — P2P may fail)", ts(), addr);
                            } else {
                                eprintln!("[{}] network:  {} (P2P available)", ts(), addr);
                            }
                        }
                    }
                    break;
                }
                if tokio::time::timeout(
                    std::time::Duration::from_secs(10),
                    watcher.updated(),
                )
                .await
                .is_err()
                {
                    tracing::warn!("net_report: STUN did not complete within 10s");
                    eprintln!("[{}] network:  STUN timed out — relay only", ts());
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
            eprintln!("[{}] inbound:  {}", ts(), conn.remote_id().fmt_short());

            if let Err(e) = rpc_daemon::handle_connection(conn, registry, state).await {
                tracing::warn!("irpc connection error: {}", e);
            }
        });
    }

    Ok(())
}
