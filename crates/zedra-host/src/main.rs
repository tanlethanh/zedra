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

use std::io::{IsTerminal, Write};
use std::path::Path;
use std::sync::Arc;
use zedra_host::client as zedra_client;
use zedra_host::ga4::Ga4;
use zedra_host::{
    api, identity, iroh_listener, net_monitor, qr, rpc_daemon, session_registry, version_check,
    workspace_lock,
};
use zedra_rpc::ZedraPairingTicket;
use zedra_telemetry::Event;

mod setup;

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

        /// Force relay-only mode: disable P2P hole punching and direct-path
        /// advertising. All traffic goes through the relay server. Useful when
        /// the host is behind a firewall that blocks direct UDP.
        #[arg(long)]
        relay_only: bool,
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

        /// Command to run in the terminal on startup (e.g. "claude --resume <id>")
        #[arg(long)]
        launch_cmd: Option<String>,
    },

    /// Set up Zedra skills or plugins for an AI coding agent
    Setup {
        /// Skip interactive confirmation prompts
        #[arg(short, long)]
        yes: bool,

        #[command(subcommand)]
        agent: setup::SetupAgent,
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

fn write_api_discovery_file(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no parent")
    })?;
    let file_name = path.file_name().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no file name")
    })?;
    let file_name = file_name.to_string_lossy();

    for _ in 0..16 {
        let tmp_path = parent.join(format!(
            ".{}.{}.{}.tmp",
            file_name,
            std::process::id(),
            rand::random::<u64>()
        ));

        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }

        let mut file = match options.open(&tmp_path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        #[cfg(unix)]
        std::fs::set_permissions(
            &tmp_path,
            std::os::unix::fs::PermissionsExt::from_mode(0o600),
        )?;

        let write_result = file.write_all(contents).and_then(|_| file.sync_all());
        drop(file);
        if let Err(e) = write_result {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(e);
        }

        if let Err(e) = std::fs::rename(&tmp_path, path) {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(e);
        }
        return Ok(());
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not create a unique temporary api discovery file",
    ))
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let log_filter = if cli.verbose {
        let mut filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
        // `tracing` can forward span enter/exit to the `log` crate as TRACE on targets
        // `tracing::span` / `tracing::span::active` (very noisy with iroh QUIC poll loops).
        for directive in [
            "tracing::span=off",
            "tracing::span::active=off",
            "iroh=warn",
            "iroh_quinn=warn",
        ] {
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
            relay_only,
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
            let session_id = session.id.clone();
            tracing::info!(
                "Created session '{}' (id={}) for {}",
                session_name,
                session_id,
                workdir.display()
            );
            let endpoint_id = host_identity.endpoint_id();

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
                iroh_listener::create_endpoint(&host_identity, &endpoint_relay_urls, relay_only)
                    .await?;
            let endpoint_bind_ms = endpoint_bind_start.elapsed().as_millis() as u64;

            // Pre-authorize the persistent CLI client key so `zedra client` can
            // connect without QR pairing. The key is generated once per workspace.
            if let Ok(config_dir) = identity::workspace_config_dir(&workdir) {
                match zedra_client::load_or_generate_cli_key(&config_dir) {
                    Ok(cli_key) => {
                        let cli_pubkey: [u8; 32] = cli_key.verifying_key().to_bytes();
                        registry
                            .add_client_to_session(&session_id, cli_pubkey)
                            .await;
                        tracing::info!("Pre-authorized CLI client key for session {}", session_id);
                    }
                    Err(e) => tracing::warn!("Failed to load/generate CLI client key: {}", e),
                }

                // Write host-info.json for `zedra client` auto-discovery.
                let host_info = zedra_client::HostInfo {
                    endpoint_id: host_identity.endpoint_id().to_string(),
                    session_id: session_id.clone(),
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

            // 2. Generate startup QR code
            // Note: The QR encodes only endpoint_id (pubkey) — no IPs. The client
            // resolves addresses at connect time via pkarr. STUN runs in the
            // background and PkarrPublisher will republish once the public IP is
            // discovered, before any user could reasonably scan and connect.
            if let Err(e) = generate_pairing_qr(
                &registry,
                &session_id,
                endpoint_id,
                &endpoint,
                &endpoint_relay_urls,
                json,
            )
            .await
            {
                tracing::warn!("Failed to generate QR code: {}", e);
            }

            // Allow live QR regeneration while daemon is running.
            #[cfg(unix)]
            if !json && std::io::stdin().is_terminal() {
                eprintln!("Press 'r' to regenerate pairing QR.");
                let endpoint = endpoint.clone();
                let registry = registry.clone();
                let session_id = session_id.clone();
                let relay_urls_for_listener = endpoint_relay_urls.clone();
                tokio::spawn(async move {
                    if let Err(e) = run_qr_key_listener(
                        registry,
                        session_id,
                        endpoint_id,
                        endpoint,
                        relay_urls_for_listener,
                    )
                    .await
                    {
                        tracing::warn!("QR key listener stopped: {}", e);
                    }
                });
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
                        let addr_string = addr.to_string();
                        if let Err(e) = write_api_discovery_file(
                            &config_dir.join("api-addr"),
                            addr_string.as_bytes(),
                        ) {
                            tracing::warn!("Failed to write REST API address: {}", e);
                        }
                        if let Err(e) = write_api_discovery_file(
                            &config_dir.join("api-token"),
                            token.as_bytes(),
                        ) {
                            tracing::warn!("Failed to write REST API token: {}", e);
                        }
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
            let client = reqwest::Client::new();
            match client.get(&url).bearer_auth(token.trim()).send().await {
                Ok(resp) => {
                    let body = resp.text().await.unwrap_or_default();
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
                    if let Some(terminals) = v["terminals"].as_array() {
                        println!("  terminals:   {}", terminals.len());
                        for terminal in terminals {
                            let id = terminal["id"].as_str().unwrap_or("?");
                            let title = terminal["title"].as_str().unwrap_or("(untitled)");
                            let session = terminal["session_name"]
                                .as_str()
                                .or_else(|| terminal["session_id"].as_str())
                                .unwrap_or("?");
                            let created = terminal["created_at_elapsed_secs"]
                                .as_u64()
                                .map(format_duration)
                                .unwrap_or_else(|| "?".to_string());
                            let uptime = terminal["uptime_secs"]
                                .as_u64()
                                .map(format_duration)
                                .unwrap_or_else(|| "?".to_string());
                            println!(
                                "    {id}  {title}  created {created} ago  uptime {uptime}  session {session}"
                            );
                        }
                    }
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
            let client = reqwest::Client::new();
            match client
                .post(&url)
                .bearer_auth(token.trim())
                .json(&body)
                .send()
                .await
            {
                Ok(resp) => {
                    let text = resp.text().await.unwrap_or_default();
                    println!("{}", text);
                }
                Err(e) => {
                    eprintln!("Failed to reach daemon: {}", e);
                    std::process::exit(1);
                }
            }
        }

        Commands::Setup { yes, agent } => {
            setup::run(agent, yes).await?;
        }

        Commands::List => {
            let instances = workspace_lock::scan_all_instances();
            if instances.is_empty() {
                println!("No zedra instances found.");
            } else {
                let http = reqwest::Client::builder()
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
                    let status =
                        if addr.trim().is_empty() {
                            None
                        } else {
                            let url = format!("http://{}/api/status", addr.trim());
                            match http.get(&url).bearer_auth(token.trim()).send().await {
                                Ok(resp) => resp.text().await.ok().and_then(|b| {
                                    serde_json::from_str::<serde_json::Value>(&b).ok()
                                }),
                                Err(_) => None,
                            }
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

async fn generate_pairing_qr(
    registry: &Arc<session_registry::SessionRegistry>,
    session_id: &str,
    endpoint_id: iroh::PublicKey,
    endpoint: &iroh::Endpoint,
    relay_urls: &[String],
    json: bool,
) -> Result<()> {
    let ticket = ZedraPairingTicket {
        endpoint_id,
        handshake_secret: rand::random(),
        session_id: session_id.to_string(),
    };
    registry
        .add_pairing_slot(session_id, ticket.handshake_secret)
        .await;

    if json {
        let info = qr::build_pairing_info(&ticket, endpoint, relay_urls)?;
        qr::print_pairing_json(&info);
    } else {
        qr::generate_pairing_qr(&ticket, endpoint, relay_urls)?;
        eprintln!("Note: this pairing QR is one-time use.");
    }
    Ok(())
}

#[cfg(unix)]
async fn run_qr_key_listener(
    registry: Arc<session_registry::SessionRegistry>,
    session_id: String,
    endpoint_id: iroh::PublicKey,
    endpoint: iroh::Endpoint,
    relay_urls: Vec<String>,
) -> Result<()> {
    use std::io::Read;
    use std::os::fd::AsRawFd;
    use tokio::sync::mpsc;

    let (tx, mut rx) = mpsc::unbounded_channel::<u8>();
    let mut reader_task = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        let stdin = std::io::stdin();
        let _raw = RawModeGuard::new(stdin.as_raw_fd())?;
        let mut handle = stdin.lock();
        let mut byte = [0_u8; 1];

        loop {
            handle.read_exact(&mut byte)?;
            if tx.send(byte[0]).is_err() {
                break;
            }
        }
        Ok(())
    });

    loop {
        tokio::select! {
            maybe_key = rx.recv() => {
                let Some(key) = maybe_key else {
                    break;
                };
                if matches!(key, b'r' | b'R') {
                    if let Err(e) = generate_pairing_qr(&registry, &session_id, endpoint_id, &endpoint, &relay_urls, false).await {
                        tracing::warn!("Failed to regenerate QR code: {}", e);
                    } else {
                        eprintln!("Regenerated pairing QR (press 'r' again to refresh).");
                    }
                }
            }
            reader_result = &mut reader_task => {
                match reader_result {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => return Err(e.into()),
                    Err(e) => return Err(anyhow::anyhow!("QR key reader task failed: {}", e)),
                }
                break;
            }
        }
    }

    Ok(())
}

#[cfg(unix)]
struct RawModeGuard {
    fd: std::os::fd::RawFd,
    original: libc::termios,
}

#[cfg(unix)]
impl RawModeGuard {
    fn new(fd: std::os::fd::RawFd) -> std::io::Result<Self> {
        let mut original = std::mem::MaybeUninit::<libc::termios>::uninit();
        // SAFETY: libc validates the fd and initializes the termios struct on success.
        let ret = unsafe { libc::tcgetattr(fd, original.as_mut_ptr()) };
        if ret != 0 {
            return Err(std::io::Error::last_os_error());
        }

        // SAFETY: `original` was initialized by `tcgetattr` above.
        let original = unsafe { original.assume_init() };
        let mut raw = original;
        raw.c_lflag &= !(libc::ICANON | libc::ECHO);
        raw.c_cc[libc::VMIN] = 1;
        raw.c_cc[libc::VTIME] = 0;

        // SAFETY: `raw` points to a valid termios struct for this fd.
        let ret = unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) };
        if ret != 0 {
            return Err(std::io::Error::last_os_error());
        }

        Ok(Self { fd, original })
    }
}

#[cfg(unix)]
impl Drop for RawModeGuard {
    fn drop(&mut self) {
        // SAFETY: `self.original` came from a successful `tcgetattr` on this fd.
        let ret = unsafe { libc::tcsetattr(self.fd, libc::TCSANOW, &self.original) };
        if ret != 0 {
            tracing::warn!(
                "Failed to restore terminal mode: {}",
                std::io::Error::last_os_error()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn write_api_discovery_file_uses_0600_permissions_on_unix() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let token_path = dir.path().join("api-token");

        write_api_discovery_file(&token_path, b"first-token").unwrap();
        assert_eq!(std::fs::read_to_string(&token_path).unwrap(), "first-token");
        assert_eq!(
            std::fs::metadata(&token_path).unwrap().permissions().mode() & 0o777,
            0o600
        );

        write_api_discovery_file(&token_path, b"second-token").unwrap();
        assert_eq!(
            std::fs::read_to_string(&token_path).unwrap(),
            "second-token"
        );
        assert_eq!(
            std::fs::metadata(&token_path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
