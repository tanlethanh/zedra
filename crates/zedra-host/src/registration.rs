// Host Registration Loop
//
// Registers with the coordination server on startup and sends heartbeats
// every 30 seconds. Non-fatal: daemon works in LAN-only mode if the
// coordination server is unreachable.

use std::time::Duration;

use zedra_relay::coord::{
    CoordClient, HeartbeatRequest, HostAddress, HostSession, RegisterRequest,
};

use crate::identity::SharedIdentity;
use crate::qr::collect_lan_addrs;

/// Heartbeat interval in seconds.
const HEARTBEAT_INTERVAL_SECS: u64 = 30;

/// Configuration for host registration.
pub struct RegistrationConfig {
    pub coord_url: String,
    pub identity: SharedIdentity,
    pub port: u16,
    pub workdir: std::path::PathBuf,
    pub version: String,
}

/// Run the coordination server registration loop.
///
/// Registers the host on startup, then heartbeats every 30s.
/// Runs indefinitely. Non-fatal — logs warnings on failures.
pub async fn run_registration_loop(config: RegistrationConfig) {
    let client = CoordClient::new(&config.coord_url);
    let device_id = config.identity.device_id.to_string();
    let public_key = base64_url::encode(&config.identity.public_key_bytes());

    // Build initial registration
    let addresses = build_addresses(config.port);
    let sessions = build_sessions(&config.workdir);
    let hostname = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "unknown".to_string());

    let req = RegisterRequest {
        device_id: device_id.clone(),
        public_key,
        hostname,
        addresses: addresses.clone(),
        sessions: sessions.clone(),
        capabilities: vec![
            "terminal".to_string(),
            "fs".to_string(),
            "git".to_string(),
        ],
        version: config.version,
    };

    // Initial registration
    match client.register(&req).await {
        Ok(resp) => {
            tracing::info!(
                "Registered with coordination server (TTL: {}s, relay: {})",
                resp.ttl,
                resp.relay_endpoint,
            );
        }
        Err(e) => {
            tracing::warn!(
                "Failed to register with coordination server: {} (LAN-only mode)",
                e
            );
        }
    }

    // Heartbeat loop
    let mut interval = tokio::time::interval(Duration::from_secs(HEARTBEAT_INTERVAL_SECS));
    interval.tick().await; // skip immediate tick

    loop {
        interval.tick().await;

        // Re-collect addresses in case network changed
        let current_addrs = build_addresses(config.port);

        let hb = HeartbeatRequest {
            addresses: Some(current_addrs),
            sessions: None, // sessions don't change frequently
        };

        match client.heartbeat(&device_id, &hb).await {
            Ok(_resp) => {
                tracing::debug!("Heartbeat sent to coordination server");
            }
            Err(e) => {
                tracing::debug!("Heartbeat failed: {} (will retry)", e);
                // If heartbeat fails, try re-registering on next iteration
                // The registration will create a fresh entry with new TTL
                if let Err(re) = client.register(&req).await {
                    tracing::debug!("Re-registration also failed: {}", re);
                } else {
                    tracing::info!("Re-registered with coordination server after heartbeat failure");
                }
            }
        }
    }
}

/// Build address list from current network interfaces.
fn build_addresses(port: u16) -> Vec<HostAddress> {
    collect_lan_addrs()
        .into_iter()
        .map(|ip| HostAddress {
            addr_type: "lan".to_string(),
            addr: format!("{}:{}", ip, port),
        })
        .collect()
}

/// Build session list from the working directory.
fn build_sessions(workdir: &std::path::Path) -> Vec<HostSession> {
    let name = workdir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("default")
        .to_string();

    vec![HostSession {
        id: uuid_from_path(workdir),
        name,
        workdir: workdir.display().to_string(),
    }]
}

/// Deterministic UUID from a path (for stable session IDs across restarts).
fn uuid_from_path(path: &std::path::Path) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    let hash = hasher.finish();
    format!("{:016x}", hash)
}
