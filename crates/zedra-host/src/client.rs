// client.rs — Local test client for zedra-host.
//
// `zedra client` reads the running host's connection info (written to
// host-info.json by `zedra start`) and connects using the same relay.
// It authenticates with a persistent CLI client key that the host
// pre-authorizes at startup, then runs a continuous ping loop to measure RTT.

use anyhow::{Context, Result};
use ed25519_dalek::{Signer, SigningKey};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use zedra_rpc::proto::{
    AuthChallengeResult, AuthProveReq, AuthProveResult, AuthReq, PingReq, ZedraProto, ZEDRA_ALPN,
};

// ---------------------------------------------------------------------------
// Host info file — written by `zedra start`, read by `zedra client`
// ---------------------------------------------------------------------------

/// Persisted alongside daemon.lock so `zedra client` can auto-connect.
#[derive(Debug, Serialize, Deserialize)]
pub struct HostInfo {
    /// Iroh endpoint public key (full hex).
    pub endpoint_id: String,
    /// Session ID for AuthProve.
    pub session_id: String,
    /// Relay URLs in use (e.g. ["https://ap1.relay.zedra.dev"]).
    pub relay_urls: Vec<String>,
}

/// Path of `host-info.json` for the given workspace config dir.
pub fn host_info_path(config_dir: &Path) -> PathBuf {
    config_dir.join("host-info.json")
}

/// Write host info alongside the daemon lock (called from `zedra start`).
pub fn write_host_info(config_dir: &Path, info: &HostInfo) -> Result<()> {
    let path = host_info_path(config_dir);
    let json = serde_json::to_string_pretty(info)?;
    std::fs::write(&path, json)
        .with_context(|| format!("failed to write host-info.json to {}", path.display()))?;
    Ok(())
}

/// Read host info written by the running daemon.
pub fn read_host_info(config_dir: &Path) -> Result<HostInfo> {
    let path = host_info_path(config_dir);
    let json = std::fs::read_to_string(&path).with_context(|| {
        format!(
            "host-info.json not found at {} — is `zedra start` running?",
            path.display()
        )
    })?;
    serde_json::from_str(&json).context("failed to parse host-info.json")
}

// ---------------------------------------------------------------------------
// CLI client key — persistent Ed25519 key pre-authorized by the host
// ---------------------------------------------------------------------------

/// Path of the CLI client key for the given workspace config dir.
pub fn cli_client_key_path(config_dir: &Path) -> PathBuf {
    config_dir.join("cli-client.key")
}

/// Load or generate the persistent CLI client key.
///
/// The key is created once per workspace and pre-authorized by `zedra start`.
/// Unlike the mobile client, no QR pairing is needed — the host adds it to the
/// ACL directly at startup.
pub fn load_or_generate_cli_key(config_dir: &Path) -> Result<SigningKey> {
    let path = cli_client_key_path(config_dir);
    let key = if path.exists() {
        let bytes = std::fs::read(&path)
            .with_context(|| format!("failed to read cli-client.key at {}", path.display()))?;
        if bytes.len() != 32 {
            anyhow::bail!(
                "invalid cli-client.key: expected 32 bytes, got {}",
                bytes.len()
            );
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        SigningKey::from_bytes(&arr)
    } else {
        let key = SigningKey::generate(&mut rand::thread_rng());
        std::fs::write(&path, key.to_bytes())
            .with_context(|| format!("failed to write cli-client.key to {}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        key
    };
    Ok(key)
}

// ---------------------------------------------------------------------------
// Client runner
// ---------------------------------------------------------------------------

/// Run the `zedra client` command: connect to the local host and ping.
///
/// `count` — number of pings (0 = run until Ctrl-C).
/// `relay_only` — force relay-only path (no P2P hole punching).
pub async fn run(workdir: &Path, count: u32, relay_only: bool) -> Result<()> {
    let config_dir = crate::identity::workspace_config_dir(workdir)?;

    // Read connection info written by the running host.
    let info = read_host_info(&config_dir)?;
    let relay_urls: Vec<&str> = info.relay_urls.iter().map(|s| s.as_str()).collect();
    eprintln!(
        "Connecting to host endpoint: {}...",
        &info.endpoint_id[..16]
    );
    for u in &relay_urls {
        eprintln!("  Relay: {}", u);
    }
    eprintln!("  Session: {}", info.session_id);
    if relay_only {
        eprintln!("  Mode: relay-only (P2P disabled)");
    }

    // Load persistent CLI client key.
    let cli_key = load_or_generate_cli_key(&config_dir)?;
    let client_pubkey: [u8; 32] = cli_key.verifying_key().to_bytes();

    // Parse host endpoint ID.
    let host_pubkey: iroh::PublicKey = info
        .endpoint_id
        .parse()
        .context("invalid endpoint_id in host-info.json")?;

    // Create an ephemeral iroh endpoint.
    // In relay-only mode: use a custom relay map pointing at the host's relays,
    // with QUIC discovery disabled so iroh cannot establish direct paths.
    // In normal mode: RelayMode::Default uses n0's relay + pkarr for P2P.
    let mut builder = iroh::Endpoint::builder();
    if relay_only {
        anyhow::ensure!(!relay_urls.is_empty(), "no relay URLs in host-info.json");
        let configs = relay_urls
            .iter()
            .map(|u| {
                let url: iroh::RelayUrl =
                    u.parse().context("invalid relay_url in host-info.json")?;
                Ok(iroh::RelayConfig { url, quic: None })
            })
            .collect::<Result<Vec<_>>>()?;
        let relay_map = iroh::RelayMap::from_iter(configs);
        // clear_ip_transports() disables all direct UDP transports so the
        // endpoint can only communicate via relay — no P2P path migration.
        builder = builder
            .relay_mode(iroh::RelayMode::Custom(relay_map))
            .clear_ip_transports();
    } else {
        builder = builder.relay_mode(iroh::RelayMode::Default);
    }
    let endpoint = builder
        .bind()
        .await
        .context("failed to bind iroh endpoint")?;

    // Build host address. In relay-only mode we provide all relay URLs so
    // iroh can reach the host through whichever relay it's connected to.
    // In normal mode we provide just the pubkey and let pkarr resolve addresses.
    let host_addr = if relay_only {
        let mut addr = iroh::EndpointAddr::new(host_pubkey);
        for u in &relay_urls {
            let url: iroh::RelayUrl = u.parse().context("invalid relay_url in host-info.json")?;
            addr = addr.with_relay_url(url);
        }
        addr
    } else {
        iroh::EndpointAddr::from(host_pubkey)
    };
    let conn = endpoint
        .connect(host_addr, ZEDRA_ALPN)
        .await
        .context("failed to connect to host")?;

    eprintln!("Connected. Path: {:?}", {
        use iroh::Watcher;
        conn.paths()
            .get()
            .iter()
            .find(|p| p.is_selected())
            .map(|p| format!("{:?}", p.remote_addr()))
            .unwrap_or_else(|| "unknown".into())
    });

    // Create irpc client.
    let remote = irpc_iroh::IrohRemoteConnection::new(conn.clone());
    let client = irpc::Client::<ZedraProto>::boxed(remote);

    // --- Auth: Authenticate → AuthProve (no Register needed; pre-authorized) ---
    let challenge: AuthChallengeResult = client
        .rpc(AuthReq { client_pubkey })
        .await
        .context("Authenticate RPC failed")?;

    // Verify host signature over the nonce.
    {
        use ed25519_dalek::Verifier;
        let host_vk = ed25519_dalek::VerifyingKey::from_bytes(host_pubkey.as_bytes())
            .context("invalid host public key")?;
        let sig = ed25519_dalek::Signature::from_bytes(&challenge.host_signature);
        host_vk
            .verify(&challenge.nonce, &sig)
            .context("host signature verification failed — wrong host?")?;
    }

    let client_signature: [u8; 64] = cli_key.sign(&challenge.nonce).to_bytes();
    match client
        .rpc(AuthProveReq {
            nonce: challenge.nonce,
            client_signature,
            session_id: info.session_id.clone(),
        })
        .await
        .context("AuthProve RPC failed")?
    {
        AuthProveResult::Ok => {}
        other => anyhow::bail!("AuthProve rejected: {:?}", other),
    }

    eprintln!("Authenticated. Running ping loop (Ctrl-C to stop)...\n");
    eprintln!("{:<6} {:>10}  {:<4}  path", "seq", "rtt", "type");
    eprintln!("{}", "-".repeat(55));

    // --- Ping loop ---
    let mut seq: u32 = 0;
    let mut relay_count: u32 = 0;
    let mut relay_total_rtt: u64 = 0;
    let mut relay_min: u64 = u64::MAX;
    let mut relay_max: u64 = 0;
    let mut direct_count: u32 = 0;
    let mut direct_total_rtt: u64 = 0;
    let mut direct_min: u64 = u64::MAX;
    let mut direct_max: u64 = 0;

    loop {
        seq += 1;
        let t0 = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        match client.rpc(PingReq { timestamp_ms: t0 }).await {
            Ok(pong) => {
                let rtt = (std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64)
                    .saturating_sub(pong.timestamp_ms);

                let (path_label, is_relay) = {
                    use iroh::Watcher;
                    let paths = conn.paths().get();
                    let selected = paths.iter().find(|p| p.is_selected());
                    match selected {
                        Some(p) => {
                            let addr = format!("{:?}", p.remote_addr());
                            let via_relay = addr.contains("Relay");
                            (addr, via_relay)
                        }
                        None => ("unknown".to_string(), false),
                    }
                };

                if is_relay {
                    relay_count += 1;
                    relay_total_rtt += rtt;
                    relay_min = relay_min.min(rtt);
                    relay_max = relay_max.max(rtt);
                    println!("{:<6} {:>8}ms  {:<4}  {}", seq, rtt, "RLY", path_label);
                } else {
                    direct_count += 1;
                    direct_total_rtt += rtt;
                    direct_min = direct_min.min(rtt);
                    direct_max = direct_max.max(rtt);
                    println!("{:<6} {:>8}ms  {:<4}  {}", seq, rtt, "P2P", path_label);
                }
            }
            Err(e) => {
                eprintln!("[{}] RPC error: {}", seq, e);
                break;
            }
        }

        if count > 0 && seq >= count {
            break;
        }

        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }

    eprintln!("\n--- ping statistics ---");
    if relay_count > 0 {
        eprintln!(
            "relay (RLY): {} pings  min={}ms avg={}ms max={}ms",
            relay_count,
            relay_min,
            relay_total_rtt / relay_count as u64,
            relay_max,
        );
    }
    if direct_count > 0 {
        eprintln!(
            "direct (P2P): {} pings  min={}ms avg={}ms max={}ms",
            direct_count,
            direct_min,
            direct_total_rtt / direct_count as u64,
            direct_max,
        );
    }
    if relay_count == 0 && direct_count == 0 {
        eprintln!("no pings completed");
    }

    endpoint.close().await;
    Ok(())
}
