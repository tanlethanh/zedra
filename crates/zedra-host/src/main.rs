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

use std::sync::Arc;
use zedra_host::analytics::Analytics;
use zedra_host::client as zedra_client;
use zedra_host::{api, identity, iroh_listener, qr, rpc_daemon, session_registry, workspace_lock};
use zedra_rpc::ZedraPairingTicket;

#[derive(Parser)]
#[command(name = "zedra", about = "Desktop companion daemon for Zedra")]
struct Cli {
    /// Show tracing logs (default: only user-facing output)
    #[arg(long, global = true)]
    verbose: bool,

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

        /// Command to inject into every new terminal on startup.
        /// Example: --launch-cmd "claude --resume" drops the user straight
        /// into a resumed Claude Code session each time a terminal is opened.
        #[arg(long)]
        launch_cmd: Option<String>,
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

    /// Show status of the running daemon (reads api-addr/api-token automatically)
    Status {
        /// Working directory of the running daemon
        #[arg(short, long, default_value = ".")]
        workdir: String,
    },

    /// List all active zedra instances across all workdirs
    List,

    /// Open a new terminal on the connected phone (reads api-addr/api-token automatically)
    Terminal {
        /// Working directory of the running daemon
        #[arg(short, long, default_value = ".")]
        workdir: String,

        /// Command to inject into the terminal on startup (e.g. "claude --resume <id>")
        #[arg(long)]
        launch_cmd: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let log_filter = if cli.verbose {
        let mut filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
        // `tracing` can forward span enter/exit to the `log` crate as TRACE on targets
        // `tracing::span` / `tracing::span::active` (very noisy with iroh QUIC poll loops).
        for directive in ["tracing::span=off", "tracing::span::active=off"] {
            if let Ok(d) = directive.parse::<tracing_subscriber::filter::Directive>() {
                filter = filter.add_directive(d);
            }
        }
        filter
    } else {
        tracing_subscriber::EnvFilter::new("error")
    };
    tracing_subscriber::fmt().with_env_filter(log_filter).init();

    match cli.command {
        Commands::Client {
            workdir,
            count,
            relay_only,
        } => {
            let workdir = std::path::PathBuf::from(workdir)
                .canonicalize()
                .unwrap_or_else(|_| std::path::PathBuf::from("."));
            zedra_client::run(&workdir, count, relay_only).await?;
        }

        Commands::Start {
            workdir,
            json,
            relay_url,
            launch_cmd,
        } => {
            let workdir = std::path::PathBuf::from(workdir)
                .canonicalize()
                .unwrap_or_else(|_| std::path::PathBuf::from("."));

            tracing::info!("Starting zedra-host (iroh transport)");
            tracing::info!("Serving workdir: {}", workdir.display());

            let _lock = workspace_lock::acquire(&workdir)?;
            tracing::info!("Acquired workspace lock for {}", workdir.display());

            let host_identity = match identity::HostIdentity::load_or_generate_for_workdir(&workdir)
            {
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
            let session = registry.create_named(&session_name, workdir.clone()).await;
            tracing::info!(
                "Created session '{}' (id={}) for {}",
                session_name,
                session.id,
                workdir.display()
            );

            // Create a one-use pairing slot and encode as QR ticket
            let handshake_secret: [u8; 16] = rand::random();
            registry
                .add_pairing_slot(&session.id, handshake_secret)
                .await;

            let ticket = ZedraPairingTicket {
                endpoint_id: host_identity.endpoint_id(),
                handshake_secret,
                session_id: session.id.clone(),
            };

            // Initialize analytics. The analytics_id is machine-level (not per-workspace)
            // so connection counts roll up to a single host in the dashboard.
            let analytics = Arc::new(Analytics::new(
                &identity::analytics_id_path().unwrap_or_else(|_| workdir.join(".zedra-analytics-id")),
            ));
            if analytics.is_enabled() {
                eprintln!("[init]     analytics enabled");
            }

            let mut state = rpc_daemon::DaemonState::new(workdir.clone(), host_identity.clone());
            state.default_launch_cmd = launch_cmd;
            state.analytics = analytics.clone();
            let state = Arc::new(state);

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

            // Determine relay_type label for analytics before the endpoint is created.
            let relay_type = if is_cf_worker_relay(base_relay_url) {
                "cf_worker"
            } else if relay_url.is_some() {
                "custom"
            } else {
                "default"
            };
            analytics.daemon_start(relay_type);

            let endpoint = iroh_listener::create_endpoint(
                &host_identity,
                Some(&endpoint_relay_url),
                analytics.clone(),
            )
            .await?;

            // Pre-authorize the persistent CLI client key so `zedra client` can
            // connect without QR pairing. The key is generated once per workspace.
            if let Ok(config_dir) = identity::workspace_config_dir(&workdir) {
                match zedra_client::load_or_generate_cli_key(&config_dir) {
                    Ok(cli_key) => {
                        let cli_pubkey: [u8; 32] = cli_key.verifying_key().to_bytes();
                        registry
                            .add_client_to_session(&session.id, cli_pubkey)
                            .await;
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

            // 3. Start local REST API server (127.0.0.1, OS-assigned port).
            //    Write the bound address and bearer token to the config dir so
            //    tools like `/zedra-start` can discover and authenticate.
            if let Ok(config_dir) = identity::workspace_config_dir(&workdir) {
                let token: String = {
                    let bytes: [u8; 32] = rand::random();
                    bytes.iter().map(|b| format!("{:02x}", b)).collect()
                };
                match api::start(api::ApiState {
                    registry: registry.clone(),
                    daemon_state: state.clone(),
                    token: token.clone(),
                })
                .await
                {
                    Ok(addr) => {
                        let _ = std::fs::write(config_dir.join("api-addr"), addr.to_string());
                        let _ = std::fs::write(config_dir.join("api-token"), &token);
                        tracing::info!("REST API listening on http://{}", addr);
                    }
                    Err(e) => tracing::warn!("Failed to start REST API: {}", e),
                }
            }

            // 4. Run iroh accept loop (blocks main)
            iroh_listener::run_accept_loop(&endpoint, registry, state).await?;
        }

        Commands::Status { workdir } => {
            let workdir = std::path::PathBuf::from(workdir)
                .canonicalize()
                .unwrap_or_else(|_| std::path::PathBuf::from("."));
            let config_dir = identity::workspace_config_dir(&workdir)?;
            let addr = std::fs::read_to_string(config_dir.join("api-addr")).unwrap_or_default();
            let token = std::fs::read_to_string(config_dir.join("api-token")).unwrap_or_default();
            if addr.trim().is_empty() {
                eprintln!("No running daemon found for: {}", workdir.display());
                std::process::exit(1);
            }
            let url = format!("http://{}/api/status", addr.trim());
            let client = reqwest::blocking::Client::new();
            match client.get(&url).bearer_auth(token.trim()).send() {
                Ok(resp) => {
                    let body = resp.text().unwrap_or_default();
                    let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                    let version = v["version"].as_str().unwrap_or("?");
                    let workdir = v["workdir"].as_str().unwrap_or("?");
                    let endpoint_id = v["endpoint_id"].as_str().unwrap_or("?");
                    let uptime = v["uptime_secs"]
                        .as_u64()
                        .map(format_duration)
                        .unwrap_or_default();
                    println!("zedra host v{}", version);
                    println!("  uptime:      {}", uptime);
                    println!("  workdir:     {}", workdir);
                    println!("  endpoint_id: {}", &endpoint_id[..endpoint_id.len().min(8)]);
                }
                Err(e) => {
                    eprintln!("Failed to reach daemon: {}", e);
                    std::process::exit(1);
                }
            }
        }

        Commands::Terminal {
            workdir,
            launch_cmd,
        } => {
            let workdir = std::path::PathBuf::from(workdir)
                .canonicalize()
                .unwrap_or_else(|_| std::path::PathBuf::from("."));
            let config_dir = identity::workspace_config_dir(&workdir)?;
            let addr = std::fs::read_to_string(config_dir.join("api-addr")).unwrap_or_default();
            let token = std::fs::read_to_string(config_dir.join("api-token")).unwrap_or_default();
            if addr.trim().is_empty() {
                eprintln!("No running daemon found for: {}", workdir.display());
                std::process::exit(1);
            }
            let url = format!("http://{}/api/terminal", addr.trim());
            let body = serde_json::json!({ "launch_cmd": launch_cmd });
            let client = reqwest::blocking::Client::new();
            match client
                .post(&url)
                .bearer_auth(token.trim())
                .json(&body)
                .send()
            {
                Ok(resp) => {
                    let text = resp.text().unwrap_or_default();
                    println!("{}", text);
                }
                Err(e) => {
                    eprintln!("Failed to reach daemon: {}", e);
                    std::process::exit(1);
                }
            }
        }

        Commands::List => {
            let instances = workspace_lock::scan_all_instances();
            if instances.is_empty() {
                println!("No zedra instances found.");
            } else {
                let http = reqwest::blocking::Client::builder()
                    .timeout(std::time::Duration::from_secs(2))
                    .build()
                    .unwrap_or_default();
                for (config_dir, lock, alive) in &instances {
                    if !alive {
                        println!("  [stale]  {}  (pid {} gone)", lock.workdir, lock.pid);
                        continue;
                    }
                    let addr =
                        std::fs::read_to_string(config_dir.join("api-addr")).unwrap_or_default();
                    let token =
                        std::fs::read_to_string(config_dir.join("api-token")).unwrap_or_default();
                    let status = if !addr.trim().is_empty() {
                        let url = format!("http://{}/api/status", addr.trim());
                        http.get(&url)
                            .bearer_auth(token.trim())
                            .send()
                            .ok()
                            .and_then(|r| r.text().ok())
                            .and_then(|b| serde_json::from_str::<serde_json::Value>(&b).ok())
                    } else {
                        None
                    };

                    let workdir = status
                        .as_ref()
                        .and_then(|v| v["workdir"].as_str().map(|s| s.to_string()))
                        .unwrap_or_else(|| lock.workdir.clone());
                    let version = status
                        .as_ref()
                        .and_then(|v| v["version"].as_str().map(|s| s.to_string()))
                        .unwrap_or_else(|| "?".to_string());
                    let endpoint_id = status
                        .as_ref()
                        .and_then(|v| v["endpoint_id"].as_str().map(|s| s.to_string()))
                        .unwrap_or_else(|| "?".to_string());

                    println!("  v{version}  {workdir}");
                    println!("    endpoint:  {}", &endpoint_id[..endpoint_id.len().min(8)]);
                    println!(
                        "    pid:       {}  started {}",
                        lock.pid,
                        lock.running_for()
                    );

                    if let Some(sessions) = status.as_ref().and_then(|v| v["sessions"].as_array()) {
                        for s in sessions {
                            let id = s["id"].as_str().unwrap_or("?");
                            let name = s["name"]
                                .as_str()
                                .map(|n| format!(" ({n})"))
                                .unwrap_or_default();
                            let terms = s["terminal_count"].as_u64().unwrap_or(0);
                            let occupied = if s["is_occupied"].as_bool().unwrap_or(false) {
                                " [connected]"
                            } else {
                                ""
                            };
                            let uptime = s["uptime_secs"].as_u64().unwrap_or(0);
                            print!(
                                "    session:   {id}{name}{occupied}  {}",
                                format_duration(uptime)
                            );
                            if terms > 0 {
                                print!("  ({terms} terminal{})", if terms == 1 { "" } else { "s" });
                            }
                            println!();
                        }
                    }
                    println!();
                }
            }
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

fn format_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m{}s", secs / 60, secs % 60)
    } else {
        format!("{}h{}m", secs / 3600, (secs % 3600) / 60)
    }
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
