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
use zedra_host::client as zedra_client;
use zedra_host::ga4::Ga4;
use zedra_host::{
    api, identity, iroh_listener, net_monitor, qr, rpc_daemon, session_registry, version_check,
    workspace_lock,
};
use zedra_rpc::ZedraPairingTicket;
use zedra_telemetry::Event;

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

        /// Override relay URL(s). Can be specified multiple times for multi-relay.
        /// (e.g. --relay-url https://sg1.relay.zedra.dev --relay-url https://us1.relay.zedra.dev)
        #[arg(long)]
        relay_url: Vec<String>,

        /// Disable anonymous telemetry (usage events sent to Google Analytics).
        /// Can also be set via ZEDRA_TELEMETRY=0 environment variable.
        #[arg(long)]
        no_telemetry: bool,

        /// Debug telemetry: log every event payload and GA4 validation response to
        /// stderr. Uses the GA4 debug endpoint — events are NOT recorded in GA4.
        #[arg(long)]
        debug_telemetry: bool,
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

    /// Update zedra to the latest version
    Update {
        /// Install a specific version (e.g. v0.2.0)
        #[arg(long)]
        version: Option<String>,

        /// Skip confirmation prompt
        #[arg(long, short)]
        yes: bool,
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
            no_telemetry,
            debug_telemetry,
        } => {
            let startup_start = std::time::Instant::now();
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

            // Initialize telemetry. Disabled by --no-telemetry flag or ZEDRA_TELEMETRY=0.
            let telemetry_disabled = no_telemetry
                || std::env::var("ZEDRA_TELEMETRY")
                    .map(|v| v == "0" || v.eq_ignore_ascii_case("false"))
                    .unwrap_or(false);
            let ga4 = Arc::new(if telemetry_disabled {
                Ga4::disabled()
            } else {
                let g = Ga4::new(
                    &identity::telemetry_id_path()
                        .unwrap_or_else(|_| workdir.join(".zedra-telemetry-id")),
                    debug_telemetry,
                );
                if debug_telemetry {
                    eprintln!(
                        "[telemetry] telemetry debug mode (GA4 validation endpoint, not recorded)"
                    );
                }
                g
            });
            let is_first_run = ga4.is_first_run;

            // Register host GA4 backend as the global telemetry provider.
            zedra_host::telemetry::init(ga4.clone());

            // Install panic hook that sends host_panic event via zedra_telemetry
            // before the process aborts. record_panic bypasses the enabled flag.
            let prev_hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                let message = info
                    .payload()
                    .downcast_ref::<&str>()
                    .copied()
                    .or_else(|| info.payload().downcast_ref::<String>().map(|s| s.as_str()))
                    .unwrap_or("unknown");
                let location = info
                    .location()
                    .map(|l| format!("{}:{}", l.file(), l.line()))
                    .unwrap_or_default();
                zedra_telemetry::record_panic(message, &location);
                prev_hook(info);
            }));

            let state = Arc::new(rpc_daemon::DaemonState::new(
                workdir.clone(),
                host_identity.clone(),
            ));

            // 1. Bind iroh endpoint with configured relay URLs.
            let endpoint_relay_urls: Vec<String> = if relay_url.is_empty() {
                zedra_rpc::ZEDRA_RELAY_URLS
                    .iter()
                    .map(|s| s.to_string())
                    .collect()
            } else {
                relay_url.clone()
            };

            let relay_type = if !relay_url.is_empty() {
                "custom"
            } else {
                "default"
            };
            zedra_telemetry::send(Event::DaemonStart {
                relay_type,
                is_first_run,
            });

            let init_ms = startup_start.elapsed().as_millis() as u64;
            let endpoint_bind_start = std::time::Instant::now();
            let endpoint =
                iroh_listener::create_endpoint(&host_identity, &endpoint_relay_urls).await?;
            let endpoint_bind_ms = endpoint_bind_start.elapsed().as_millis() as u64;

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
                let host_info = zedra_client::HostInfo {
                    endpoint_id: host_identity.endpoint_id().to_string(),
                    session_id: session.id.clone(),
                    relay_urls: endpoint_relay_urls.clone(),
                };
                if let Err(e) = zedra_client::write_host_info(&config_dir, &host_info) {
                    tracing::warn!("Failed to write host-info.json: {}", e);
                } else {
                    tracing::info!("Wrote host-info.json to {}", config_dir.display());
                }
            }

            // 1a. Async version check (non-blocking, silent on failure).
            tokio::spawn(async {
                match version_check::check_latest_version().await {
                    Ok(Some(ref latest)) => {
                        eprintln!(
                            "[update]   new version available: {} (current: v{}). Run `zedra update`.",
                            latest,
                            env!("CARGO_PKG_VERSION")
                        );
                        zedra_telemetry::send(Event::UpdateChecked {
                            update_available: true,
                            latest_version: latest.clone(),
                            current_version: env!("CARGO_PKG_VERSION"),
                        });
                    }
                    Ok(None) => {
                        zedra_telemetry::send(Event::UpdateChecked {
                            update_available: false,
                            latest_version: String::new(),
                            current_version: env!("CARGO_PKG_VERSION"),
                        });
                    }
                    Err(_) => {}
                }
            });

            // 1b. Start background network diagnostics monitor.
            //     Watches for IP changes, relay changes, NAT changes, and logs
            //     DNS re-registration when the endpoint address updates.
            net_monitor::spawn_net_monitor(&endpoint);

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
            zedra_telemetry::send(Event::StartupComplete {
                init_ms,
                endpoint_bind_ms,
                total_ms: startup_start.elapsed().as_millis() as u64,
            });

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

            // 4. Spawn periodic heartbeat for uptime tracking (every 10 minutes).
            {
                let registry = registry.clone();
                let started_at = state.started_at;
                tokio::spawn(async move {
                    let mut interval =
                        tokio::time::interval(std::time::Duration::from_secs(10 * 60));
                    interval.tick().await; // skip the immediate first tick
                    loop {
                        interval.tick().await;
                        let uptime_secs = started_at.elapsed().as_secs();
                        let sessions = registry.list_sessions().await;
                        let session_count = sessions.len();
                        let terminal_count: usize = sessions.iter().map(|s| s.terminal_count).sum();
                        zedra_telemetry::send(Event::DaemonHeartbeat {
                            uptime_secs,
                            session_count,
                            terminal_count,
                        });
                    }
                });
            }

            // 5. Run iroh accept loop (blocks main)
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
                    println!(
                        "  endpoint_id: {}",
                        &endpoint_id[..endpoint_id.len().min(8)]
                    );
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
                    println!(
                        "    endpoint:  {}",
                        &endpoint_id[..endpoint_id.len().min(8)]
                    );
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

        Commands::Update { version, yes } => {
            let current = env!("CARGO_PKG_VERSION");
            eprintln!("zedra v{current}");

            // Check what's available
            let target_tag = if let Some(ref v) = version {
                v.clone()
            } else {
                eprintln!("Checking for updates...");
                match version_check::check_latest_version().await {
                    Ok(Some(tag)) => tag,
                    Ok(None) => {
                        eprintln!("Already up to date.");
                        return Ok(());
                    }
                    Err(e) => {
                        eprintln!("Failed to check for updates: {e}");
                        std::process::exit(1);
                    }
                }
            };

            eprintln!("Update available: {target_tag}");

            // Warn about running daemons
            let instances = workspace_lock::scan_all_instances();
            let alive: Vec<_> = instances.iter().filter(|(_, _, alive)| *alive).collect();
            if !alive.is_empty() {
                eprintln!(
                    "\nWarning: {} running daemon(s) found. Restart them after update:",
                    alive.len()
                );
                for (_, lock, _) in &alive {
                    eprintln!("  pid {}  {}", lock.pid, lock.workdir);
                }
                eprintln!();
            }

            // Confirm unless --yes
            if !yes {
                eprint!("Proceed with update? [y/N] ");
                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;
                if !input.trim().eq_ignore_ascii_case("y") {
                    eprintln!("Cancelled.");
                    return Ok(());
                }
            }

            let update_start = std::time::Instant::now();
            match version_check::self_update(&target_tag).await {
                Ok(tag) => {
                    let elapsed_ms = update_start.elapsed().as_millis() as u64;
                    zedra_telemetry::send(Event::SelfUpdate {
                        success: true,
                        target_version: tag.clone(),
                        from_version: env!("CARGO_PKG_VERSION"),
                        error: "",
                        elapsed_ms,
                    });
                    eprintln!("\nUpdated to {tag}.");
                    if !alive.is_empty() {
                        eprintln!(
                            "Restart running daemons: `zedra stop -w <dir> && zedra start -w <dir>`"
                        );
                    }
                }
                Err(e) => {
                    let elapsed_ms = update_start.elapsed().as_millis() as u64;
                    let error_label = classify_update_error(&e);
                    zedra_telemetry::send(Event::SelfUpdate {
                        success: false,
                        target_version: target_tag.clone(),
                        from_version: env!("CARGO_PKG_VERSION"),
                        error: error_label,
                        elapsed_ms,
                    });
                    eprintln!("Update failed: {e}");
                    std::process::exit(1);
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

fn classify_update_error(e: &anyhow::Error) -> &'static str {
    let msg = e.to_string();
    if msg.contains("checksum mismatch") {
        "checksum_mismatch"
    } else if msg.contains("download failed") || msg.contains("error sending request") {
        "download_failed"
    } else if msg.contains("archive did not contain") || msg.contains("failed to extract") {
        "extract_failed"
    } else if msg.contains("failed to install") || msg.contains("failed to rename") {
        "install_failed"
    } else if msg.contains("failed to resolve latest") {
        "version_resolve_failed"
    } else {
        "unknown"
    }
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
