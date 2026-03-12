// zedra-host: Desktop companion daemon for Zedra
//
// Provides an RPC daemon that Zedra (Android/iOS) connects to for remote
// terminal, filesystem, git, and AI operations over typed irpc.
//
// Auth model (Phase 1): Public-key based. Clients scan a QR that contains
// a one-use handshake key + session_id. After first pairing their pubkey
// is added to the session ACL. Subsequent connections use Ed25519 challenge/
// response (no QR needed).

use anyhow::Result;
use clap::{Parser, Subcommand};

use zedra_host::{identity, iroh_listener, qr, rpc_daemon, session_registry, workspace_lock};
use zedra_host::client as zedra_client;
use zedra_rpc::ZedraPairingTicket;

#[derive(Parser)]
#[command(name = "zedra", about = "Desktop companion daemon for Zedra")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the daemon (iroh transport)
    Start {
        /// Working directory to serve
        #[arg(short, long, default_value = ".")]
        workdir: String,

        /// Output startup info as a single JSON line (for tool integration)
        #[arg(long)]
        json: bool,

        /// Override relay URL (e.g. https://sg1.relay.zedra.dev)
        #[arg(long)]
        relay_url: Option<String>,
    },
    /// Connect to a running daemon and measure end-to-end RTT
    Client {
        /// Working directory of the running daemon (must match `zedra start --workdir`)
        #[arg(short, long, default_value = ".")]
        workdir: String,

        /// Number of pings to send (0 = run until Ctrl-C)
        #[arg(short, long, default_value = "0")]
        count: u32,

        /// Force relay-only mode (disable P2P hole punching)
        #[arg(long)]
        relay_only: bool,
    },
    /// Stop a running daemon and release its workspace lock
    Stop {
        /// Working directory of the daemon to stop
        #[arg(short, long, default_value = ".")]
        workdir: String,

        /// Seconds to wait for clean exit before sending SIGKILL
        #[arg(long, default_value = "5")]
        grace: u64,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Client { workdir, count, relay_only } => {
            let workdir = std::path::PathBuf::from(workdir)
                .canonicalize()
                .unwrap_or_else(|_| std::path::PathBuf::from("."));
            zedra_client::run(&workdir, count, relay_only).await?;
        }

        Commands::Start { workdir, json, relay_url } => {
            let workdir = std::path::PathBuf::from(workdir)
                .canonicalize()
                .unwrap_or_else(|_| std::path::PathBuf::from("."));

            tracing::info!("Starting zedra-host (iroh transport)");
            tracing::info!("Serving workdir: {}", workdir.display());

            let _lock = workspace_lock::acquire(&workdir)?;
            tracing::info!("Acquired workspace lock for {}", workdir.display());

            let host_identity = match identity::HostIdentity::load_or_generate_for_workdir(&workdir) {
                Ok(id) => std::sync::Arc::new(id),
                Err(e) => anyhow::bail!("Failed to load host identity: {}", e),
            };

            let sessions_path = identity::workspace_config_dir(&workdir)
                .map(|d| d.join("sessions.json"))
                .unwrap_or_else(|_| workdir.join(".zedra-sessions.json"));
            let registry = std::sync::Arc::new(
                session_registry::SessionRegistry::load_or_new(sessions_path).await,
            );

            // Create a named session for the working directory
            let session_name = workdir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("default")
                .to_string();
            let session = registry
                .create_named(&session_name, workdir.clone())
                .await;
            tracing::info!(
                "Created session '{}' (id={}) for {}",
                session_name,
                session.id,
                workdir.display()
            );

            // Create a one-use pairing slot and encode as QR ticket
            let handshake_secret: [u8; 16] = rand::random();
            registry.add_pairing_slot(&session.id, handshake_secret).await;

            let ticket = ZedraPairingTicket {
                endpoint_id: host_identity.endpoint_id(),
                handshake_secret,
                session_id: session.id.clone(),
            };

            let state = std::sync::Arc::new(rpc_daemon::DaemonState::new(
                workdir.clone(),
                host_identity.clone(),
            ));

            // 1. Bind iroh endpoint.
            //    For relay.zedra.dev (CF Worker) append ?host=<base64url(pubkey)> so the
            //    Worker routes both host and client into the same RelayRoom DO. iroh's
            //    relay client preserves query params when constructing the WebSocket URL.
            //    EC2 iroh-relay and other relays ignore unknown query parameters, but we
            //    only add the param when actually talking to the CF Worker.
            let base_relay_url = relay_url.as_deref().unwrap_or(zedra_rpc::ZEDRA_RELAY_URL);
            let endpoint_relay_url = if is_cf_worker_relay(base_relay_url) {
                let host_b64 = base64_url::encode(host_identity.endpoint_id().as_bytes());
                format!("{}?host={}", base_relay_url, host_b64)
            } else {
                base_relay_url.to_string()
            };
            let endpoint = iroh_listener::create_endpoint(&host_identity, Some(&endpoint_relay_url)).await?;

            // Pre-authorize the persistent CLI client key so `zedra client` can
            // connect without QR pairing. The key is generated once per workspace.
            if let Ok(config_dir) = identity::workspace_config_dir(&workdir) {
                match zedra_client::load_or_generate_cli_key(&config_dir) {
                    Ok(cli_key) => {
                        let cli_pubkey: [u8; 32] = cli_key.verifying_key().to_bytes();
                        registry.add_client_to_session(&session.id, cli_pubkey).await;
                        tracing::info!("Pre-authorized CLI client key for session {}", session.id);
                    }
                    Err(e) => tracing::warn!("Failed to load/generate CLI client key: {}", e),
                }

                // Write host-info.json for `zedra client` auto-discovery.
                // Use endpoint_relay_url so `zedra client --relay-only` connects
                // to the same RelayRoom DO as the host (when using CF Worker relay).
                let relay_url_str = endpoint_relay_url.clone();
                let host_info = zedra_client::HostInfo {
                    endpoint_id: host_identity.endpoint_id().to_string(),
                    session_id: session.id.clone(),
                    relay_url: relay_url_str,
                };
                if let Err(e) = zedra_client::write_host_info(&config_dir, &host_info) {
                    tracing::warn!("Failed to write host-info.json: {}", e);
                } else {
                    tracing::info!("Wrote host-info.json to {}", config_dir.display());
                }
            }

            // 2. Generate QR code
            // Note: The QR encodes only endpoint_id (pubkey) — no IPs. The client
            // resolves addresses at connect time via pkarr. STUN runs in the
            // background and PkarrPublisher will republish once the public IP is
            // discovered, before any user could reasonably scan and connect.
            match qr::build_pairing_info(&ticket, &endpoint) {
                Ok(info) => {
                    if json {
                        qr::print_pairing_json(&info);
                    } else {
                        qr::generate_pairing_qr(&ticket, &endpoint).ok();
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to generate QR code: {}", e);
                }
            }

            // 3. Run iroh accept loop (blocks main)
            iroh_listener::run_accept_loop(&endpoint, registry, state).await?;
        }

        Commands::Stop { workdir, grace } => {
            let workdir = std::path::PathBuf::from(&workdir)
                .canonicalize()
                .unwrap_or_else(|_| std::path::PathBuf::from(&workdir));

            match workspace_lock::read_lock_info(&workdir)? {
                None => {
                    eprintln!("No running zedra-host found for: {}", workdir.display());
                    std::process::exit(1);
                }
                Some(info) => {
                    if !workspace_lock::is_process_alive(info.pid) {
                        eprintln!(
                            "Process {} is already gone (stale lock). Cleaning up.",
                            info.pid
                        );
                    } else {
                        eprintln!(
                            "Stopping zedra-host:\n\
                             \n\
                             \x20 PID:     {}\n\
                             \x20 Workdir: {}\n\
                             \x20 Host:    {}\n\
                             \x20 Started: {}\n",
                            info.pid,
                            info.workdir,
                            info.hostname,
                            info.running_for(),
                        );
                    }
                }
            }

            workspace_lock::kill_and_unlock(&workdir, grace)?;
            eprintln!("Done.");
        }

    }

    Ok(())
}

/// Returns true if the relay URL points to the Cloudflare Worker relay (relay.zedra.dev).
/// Only the CF Worker uses the ?host= room-routing mechanism.
fn is_cf_worker_relay(url: &str) -> bool {
    // Strip scheme, then check the host portion is exactly "relay.zedra.dev".
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    rest == "relay.zedra.dev"
        || rest.starts_with("relay.zedra.dev/")
        || rest.starts_with("relay.zedra.dev?")
}
