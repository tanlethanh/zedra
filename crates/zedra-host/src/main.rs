// zedra-host: Desktop companion daemon for Zedra
//
// Provides an RPC daemon that Zedra (Android/iOS) connects to for remote
// terminal, filesystem, git, and AI operations over typed irpc.
//
// Auth model (Phase 1): Public-key based. Clients scan a QR that contains
// a one-use handshake key + session_id. After first pairing their pubkey
// is added to the session ACL. Subsequent connections use Ed25519 challenge/
// response (no QR needed).

use anyhow::{Context, Result};
use clap::{ArgAction, CommandFactory, Parser, Subcommand};

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use zedra_host::client as zedra_client;
use zedra_host::ga4::Ga4;
use zedra_host::{
    api, identity, iroh_listener, metrics, net_monitor, qr, rpc_daemon, session_registry, utils,
    version_check, workspace_lock,
};
use zedra_rpc::ZedraPairingTicket;
use zedra_telemetry::Event;

mod setup;

#[derive(Parser)]
#[command(
    name = "zedra",
    version,
    disable_version_flag = true,
    about = "Zedra CLI - Desktop daemon",
    before_help = concat!(
        "\x1b[1mZedra CLI - Desktop daemon\x1b[0m ",
        "\x1b[2mv",
        env!("CARGO_PKG_VERSION"),
        "\x1b[0m"
    ),
    help_template = "{before-help}{usage-heading} {usage}\n\n{all-args}",
    disable_help_subcommand = true,
    arg_required_else_help = true,
    override_usage = "zedra <COMMAND> [OPTIONS]"
)]
struct Cli {
    /// Print version
    #[arg(short = 'v', long = "version", action = ArgAction::SetTrue)]
    print_version: bool,

    /// Show tracing logs
    #[arg(long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the Zedra daemon for this workspace
    Start {
        /// Working directory to serve
        #[arg(short, long, default_value = ".")]
        workdir: String,

        /// Output startup info as a single JSON line (for tool integration)
        #[arg(long)]
        json: bool,

        /// Run in the background, detached from the controlling terminal.
        /// Startup output and the pairing QR are written to daemon.log.
        #[arg(short = 'd', long, conflicts_with = "json")]
        detach: bool,

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
    /// Connect to a daemon and measure connection RTT
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
    /// Stop the daemon for this workspace
    Stop {
        /// Working directory of the daemon to stop
        #[arg(short, long, default_value = ".")]
        workdir: String,

        /// Seconds to wait for clean exit before sending SIGKILL
        #[arg(long, default_value = "5")]
        grace: u64,
    },

    /// Show daemon status, sessions, and terminals
    Status {
        /// Working directory of the running daemon
        #[arg(short, long, default_value = ".")]
        workdir: String,
    },

    /// Show local usage metrics for a workspace
    Metrics {
        /// Working directory to inspect
        #[arg(short, long, default_value = ".")]
        workdir: String,
    },

    /// Create a fresh one-time pairing QR
    Qr {
        /// Working directory of the running daemon
        #[arg(short, long, default_value = ".")]
        workdir: String,

        /// Output pairing info as a single JSON line
        #[arg(long)]
        json: bool,
    },

    /// List active Zedra daemons across workspaces
    List {
        /// Also show stale workspace locks whose process is gone
        #[arg(long, alias = "show-stale")]
        stale: bool,
    },

    /// Show recent daemon logs
    Logs {
        /// Working directory of the running daemon
        #[arg(short, long, default_value = ".")]
        workdir: String,

        /// Number of log lines to print
        #[arg(short = 'n', long, default_value_t = 80)]
        lines: usize,
    },

    /// Open a terminal on the connected phone
    Terminal {
        /// Working directory of the running daemon
        #[arg(short, long, default_value = ".")]
        workdir: String,

        /// Command to run in the terminal on startup (e.g. "claude --resume <id>")
        #[arg(long)]
        launch_cmd: Option<String>,
    },

    /// Install Zedra skills or plugins for an AI coding agent
    Setup {
        /// Skip interactive confirmation prompts
        #[arg(short, long)]
        yes: bool,

        #[command(subcommand)]
        agent: setup::SetupAgent,
    },

    /// Update the Zedra CLI
    Update {
        /// Install a specific version (e.g. v0.2.0)
        #[arg(long)]
        version: Option<String>,

        /// Skip confirmation prompt
        #[arg(long, short)]
        yes: bool,
    },

    /// Print help for zedra or a command
    Help {
        /// Command to show help for
        #[arg(value_name = "COMMAND")]
        command: Vec<String>,
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

struct DetachedStartOptions {
    workdir: PathBuf,
    verbose: bool,
    relay_url: Vec<String>,
    no_telemetry: bool,
    debug_telemetry: bool,
    relay_only: bool,
}

struct DetachedStartResult {
    pid: u32,
    workdir: PathBuf,
}

fn detached_start_child_args(options: &DetachedStartOptions) -> Vec<String> {
    let mut args = Vec::new();
    if options.verbose {
        args.push("--verbose".to_string());
    }
    args.extend([
        "start".to_string(),
        "--workdir".to_string(),
        options.workdir.display().to_string(),
    ]);
    for relay_url in &options.relay_url {
        args.extend(["--relay-url".to_string(), relay_url.clone()]);
    }
    if options.no_telemetry {
        args.push("--no-telemetry".to_string());
    }
    if options.debug_telemetry {
        args.push("--debug-telemetry".to_string());
    }
    if options.relay_only {
        args.push("--relay-only".to_string());
    }
    args
}

#[cfg(unix)]
fn start_detached(options: DetachedStartOptions) -> Result<DetachedStartResult> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;

    if let Some(existing) = workspace_lock::read_lock_info(&options.workdir)? {
        if workspace_lock::is_process_alive(existing.pid) {
            anyhow::bail!(
                "Zedra daemon is already running for this workspace.\n\
                 \n\
                 \x20 PID:      {}\n\
                 \x20 Workdir:  {}\n\
                 \x20 Host:     {}\n\
                 \x20 Started:  {}\n\
                 \n\
                 Run `zedra stop` from this workspace to stop it.\n\
                 From another directory, add `--workdir <path>`.",
                existing.pid,
                existing.workdir,
                existing.hostname,
                existing.running_for(),
            );
        }
    }

    let config_dir = identity::workspace_config_dir(&options.workdir)?;
    std::fs::create_dir_all(&config_dir)?;
    let log_path = config_dir.join("daemon.log");
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&log_path)?;
    std::fs::set_permissions(&log_path, std::fs::Permissions::from_mode(0o600))?;
    writeln!(
        log,
        "\n--- zedra detached start parent_pid={} workdir={} ---",
        std::process::id(),
        options.workdir.display()
    )?;

    let mut command = std::process::Command::new(std::env::current_exe()?);
    command
        .args(detached_start_child_args(&options))
        .current_dir(&options.workdir)
        .env("ZEDRA_DETACHED", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log));

    unsafe {
        command.pre_exec(|| {
            // Detached hosts must leave the SSH session's process group and
            // controlling terminal, otherwise logout can still deliver SIGHUP.
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let mut child = command.spawn()?;
    let child_pid = child.id();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        if let Some(status) = child.try_wait()? {
            anyhow::bail!(
                "detached zedra-host exited early with status {}. See log: {}",
                status,
                log_path.display()
            );
        }
        if let Some(info) = workspace_lock::read_lock_info(&options.workdir)? {
            if info.pid == child_pid {
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    Ok(DetachedStartResult {
        pid: child_pid,
        workdir: options.workdir,
    })
}

#[cfg(not(unix))]
fn start_detached(_options: DetachedStartOptions) -> Result<DetachedStartResult> {
    anyhow::bail!("`zedra start --detach` is only supported on Unix platforms.");
}

fn telemetry_disabled(no_telemetry: bool) -> bool {
    no_telemetry
        || std::env::var("ZEDRA_TELEMETRY")
            .map(|v| v == "0" || v.eq_ignore_ascii_case("false"))
            .unwrap_or(false)
}

fn new_ga4(
    telemetry_id_fallback_dir: &Path,
    no_telemetry: bool,
    debug_telemetry: bool,
) -> Arc<Ga4> {
    Arc::new(if telemetry_disabled(no_telemetry) {
        Ga4::disabled()
    } else {
        let ga4 = Ga4::new(
            &identity::telemetry_id_path()
                .unwrap_or_else(|_| telemetry_id_fallback_dir.join(".zedra-telemetry-id")),
            debug_telemetry,
        );
        if debug_telemetry {
            eprintln!("[telemetry] debug mode (GA4 validation endpoint, not recorded)");
        }
        ga4
    })
}

fn render_cli_version() -> String {
    format!("{}\n", env!("CARGO_PKG_VERSION"))
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.print_version {
        print!("{}", render_cli_version());
        return Ok(());
    }
    let command = cli.command.unwrap_or_else(|| {
        Cli::command()
            .error(
                clap::error::ErrorKind::MissingSubcommand,
                "a command is required",
            )
            .exit()
    });

    let verbose = cli.verbose;
    if verbose {
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
        tracing_subscriber::fmt().with_env_filter(filter).init();
    } else {
        tracing_subscriber::fmt()
            .compact()
            .without_time()
            .with_target(false)
            .with_env_filter(tracing_subscriber::EnvFilter::new("error"))
            .init();
    }

    match command {
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
            detach,
            relay_url,
            no_telemetry,
            debug_telemetry,
            relay_only,
        } => {
            let workdir = std::path::PathBuf::from(workdir)
                .canonicalize()
                .unwrap_or_else(|_| std::path::PathBuf::from("."));
            if detach {
                let detached = start_detached(DetachedStartOptions {
                    workdir,
                    verbose,
                    relay_url,
                    no_telemetry,
                    debug_telemetry,
                    relay_only,
                })?;
                match wait_for_detached_pairing_qr(&detached.workdir, detached.pid).await {
                    Ok(info) => {
                        qr::print_started_pairing_info(&info, &detached.workdir);
                        utils::println_warn("Note: this pairing QR is one-time use.");
                    }
                    Err(e) => {
                        utils::eprintln_warn(format!(
                            "Could not print a pairing QR yet: {e}. Run `zedra qr` from the workspace, or `zedra logs` to inspect startup output."
                        ));
                    }
                }
                println!();
                println!("{}", render_detached_followup_commands());
                return Ok(());
            }

            let startup_start = std::time::Instant::now();

            tracing::info!("Starting zedra-host (iroh transport)");
            tracing::info!("Serving workdir: {}", workdir.display());

            let _lock = workspace_lock::acquire(&workdir)?;
            tracing::info!("Acquired workspace lock for {}", workdir.display());
            let start_mode = if std::env::var_os("ZEDRA_DETACHED").is_some() {
                metrics::DaemonStartMode::Detached
            } else {
                metrics::DaemonStartMode::Foreground
            };
            if let Err(e) = metrics::record_daemon_start(&workdir, start_mode) {
                tracing::warn!("Failed to record daemon start metrics: {}", e);
            }

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
            let session_was_existing = registry.get_by_name(&session_name).await.is_some();
            let session = registry.create_named(&session_name, workdir.clone()).await;
            let session_id = session.id.clone();
            let session_count = registry.session_count().await;
            if !session_was_existing {
                if let Err(e) = metrics::record_session_created(&workdir, session_count) {
                    tracing::warn!("Failed to record session metrics: {}", e);
                }
            } else if let Err(e) = metrics::record_daemon_heartbeat(&workdir, session_count, 0) {
                tracing::warn!("Failed to record session metrics: {}", e);
            }
            tracing::info!(
                "Created session '{}' (id={}) for {}",
                session_name,
                session_id,
                workdir.display()
            );
            let endpoint_id = host_identity.endpoint_id();

            // Initialize telemetry. Disabled by --no-telemetry flag or ZEDRA_TELEMETRY=0.
            let ga4 = new_ga4(&workdir, no_telemetry, debug_telemetry);
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
                        let update_msg = format!(
                            "New version available: {} (current: v{}). Run `zedra update`.",
                            latest,
                            env!("CARGO_PKG_VERSION")
                        );
                        utils::eprintln_warn(update_msg);
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
                Some(&workdir),
                Some(&workdir),
            )
            .await
            {
                tracing::warn!("Failed to generate QR code: {}", e);
            }

            // Allow live QR regeneration while daemon is running.
            #[cfg(unix)]
            if !json && std::io::stdin().is_terminal() {
                utils::eprintln_note("Press 'r' to regenerate pairing QR.");
                let endpoint = endpoint.clone();
                let registry = registry.clone();
                let session_id = session_id.clone();
                let relay_urls_for_listener = endpoint_relay_urls.clone();
                let qr_workdir = workdir.clone();
                tokio::spawn(async move {
                    if let Err(e) = run_qr_key_listener(
                        registry,
                        session_id,
                        endpoint_id,
                        endpoint,
                        relay_urls_for_listener,
                        qr_workdir,
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
                    endpoint: endpoint.clone(),
                    relay_urls: endpoint_relay_urls.clone(),
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
                let metrics_workdir = workdir.clone();
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
                        if let Err(e) = metrics::record_daemon_heartbeat(
                            &metrics_workdir,
                            session_count,
                            terminal_count,
                        ) {
                            tracing::warn!("Failed to record daemon heartbeat metrics: {}", e);
                        }
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
                utils::eprintln_error(format!(
                    "No running daemon found for: {}",
                    workdir.display()
                ));
                std::process::exit(1);
            }
            let url = format!("http://{}/api/status", addr.trim());
            let client = reqwest::Client::new();
            match client.get(&url).bearer_auth(token.trim()).send().await {
                Ok(resp) => {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    if !status.is_success() {
                        utils::eprintln_error(format!(
                            "Failed to read status: HTTP {} {}",
                            status, body
                        ));
                        std::process::exit(1);
                    }
                    let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                    println!("{}", render_status_output(&v));
                }
                Err(e) => {
                    utils::eprintln_error(format!("Failed to reach daemon: {}", e));
                    std::process::exit(1);
                }
            }
        }

        Commands::Metrics { workdir } => {
            let workdir = std::path::PathBuf::from(workdir)
                .canonicalize()
                .unwrap_or_else(|_| std::path::PathBuf::from("."));
            let snapshot = metrics::snapshot(&workdir)?;
            let http = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(2))
                .build()
                .unwrap_or_default();
            let status = fetch_instance_status(&http, &workdir).await;
            println!(
                "{}",
                render_metrics_output(&workdir, &snapshot, status.as_ref())
            );
        }

        Commands::Qr { workdir, json } => {
            let workdir = std::path::PathBuf::from(workdir)
                .canonicalize()
                .unwrap_or_else(|_| std::path::PathBuf::from("."));
            let info = request_pairing_qr(&workdir).await?;
            if json {
                qr::print_pairing_json(&info);
            } else {
                qr::print_pairing_info(&info);
                utils::println_warn("Note: this pairing QR is one-time use.");
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
                utils::eprintln_error(format!(
                    "No running daemon found for: {}",
                    workdir.display()
                ));
                std::process::exit(1);
            }
            let url = format!("http://{}/api/terminal", addr.trim());
            let body = serde_json::json!({ "launch_cmd": launch_cmd.as_deref() });
            let client = reqwest::Client::new();
            match client
                .post(&url)
                .bearer_auth(token.trim())
                .json(&body)
                .send()
                .await
            {
                Ok(resp) => {
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();
                    if !status.is_success() {
                        utils::eprintln_error(format!(
                            "Failed to open terminal: HTTP {} {}",
                            status, text
                        ));
                        std::process::exit(1);
                    }
                    match render_terminal_created_output(&text, launch_cmd.as_deref()) {
                        Some(output) => println!("{output}"),
                        None => println!("{}", text),
                    }
                }
                Err(e) => {
                    utils::eprintln_error(format!("Failed to reach daemon: {}", e));
                    std::process::exit(1);
                }
            }
        }

        Commands::Setup { yes, agent } => {
            setup::run(agent, yes).await?;
        }

        Commands::List { stale } => {
            let instances = workspace_lock::scan_all_instances();
            if instances.is_empty() {
                utils::println_note("No Zedra instances found.");
            } else {
                let http = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(2))
                    .build()
                    .unwrap_or_default();
                let mut active_rows = Vec::new();
                let mut stale_rows = Vec::new();

                for (_config_dir, lock, alive) in instances {
                    if alive {
                        let status = fetch_instance_status(&http, Path::new(&lock.workdir)).await;
                        active_rows.push(active_instance_row(&lock, status.as_ref()));
                    } else {
                        stale_rows.push(stale_instance_row(&lock));
                    }
                }

                if active_rows.is_empty() {
                    utils::println_note("No active Zedra instances.");
                } else {
                    utils::println_heading("Active Zedra Daemons");
                    println!();
                    println!(
                        "{}",
                        utils::render_table(
                            &[
                                "PID", "STATE", "VERSION", "ENDPOINT", "UPTIME", "SESS", "TERMS",
                                "WORKDIR"
                            ],
                            &active_rows,
                        )
                    );
                }

                if stale {
                    if !stale_rows.is_empty() {
                        if !active_rows.is_empty() {
                            println!();
                        }
                        utils::println_heading("Stale Workspace Locks");
                        println!();
                        println!(
                            "{}",
                            utils::render_table(&["PID", "AGE", "WORKDIR"], &stale_rows)
                        );
                    }
                } else if !stale_rows.is_empty() {
                    println!();
                    utils::println_note(format!(
                        "{} stale workspace lock{} hidden. Use `zedra list --stale` to show {}.",
                        stale_rows.len(),
                        if stale_rows.len() == 1 { "" } else { "s" },
                        if stale_rows.len() == 1 { "it" } else { "them" },
                    ));
                }
            }
        }

        Commands::Logs { workdir, lines } => {
            let workdir = std::path::PathBuf::from(workdir)
                .canonicalize()
                .unwrap_or_else(|_| std::path::PathBuf::from("."));
            let log_path = daemon_log_path(&workdir)?;
            if !log_path.exists() {
                utils::eprintln_error(format!("No daemon log found for: {}", workdir.display()));
                utils::eprintln_note(format!("Expected: {}", log_path.display()));
                std::process::exit(1);
            }

            let output = read_recent_log_lines(&log_path, lines)?;
            if output.is_empty() {
                utils::eprintln_note(format!("No log output yet: {}", log_path.display()));
            } else {
                print!("{output}");
                if !output.ends_with('\n') {
                    println!();
                }
            }
        }

        Commands::Update { version, yes } => {
            let telemetry_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let ga4 = new_ga4(&telemetry_dir, false, false);
            let current = env!("CARGO_PKG_VERSION");
            utils::eprintln_heading("Zedra Update");
            eprintln!();
            utils::eprintln_key_values(&[("Current", format!("v{current}"))]);

            // Check what's available
            let target_tag = if let Some(ref v) = version {
                v.clone()
            } else {
                eprintln!();
                utils::eprintln_step("Checking for updates");
                match version_check::check_latest_version().await {
                    Ok(Some(tag)) => tag,
                    Ok(None) => {
                        utils::eprintln_success("Already up to date.");
                        return Ok(());
                    }
                    Err(e) => {
                        utils::eprintln_error(format!("Failed to check for updates: {e}"));
                        std::process::exit(1);
                    }
                }
            };

            utils::eprintln_key_values(&[("Target", target_tag.clone())]);

            // Warn about running daemons
            let instances = workspace_lock::scan_all_instances();
            let alive: Vec<_> = instances.iter().filter(|(_, _, alive)| *alive).collect();
            if !alive.is_empty() {
                eprintln!();
                utils::eprintln_warn(format!(
                    "{} running daemon(s) found. Restart them after update:",
                    alive.len()
                ));
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
                    utils::eprintln_note("Cancelled.");
                    return Ok(());
                }
            }

            let update_start = std::time::Instant::now();
            match version_check::self_update(&target_tag).await {
                Ok(tag) => {
                    let elapsed_ms = update_start.elapsed().as_millis() as u64;
                    zedra_host::telemetry::send_now(
                        &ga4,
                        Event::SelfUpdate {
                            success: true,
                            target_version: tag.clone(),
                            from_version: env!("CARGO_PKG_VERSION"),
                            error: "",
                            elapsed_ms,
                        },
                    )
                    .await;
                    eprintln!();
                    utils::eprintln_success(format!("Updated to {tag}."));
                    if !alive.is_empty() {
                        utils::eprintln_note("Restart running daemons:");
                        utils::eprintln_shell_command(
                            "zedra stop -w <dir> && zedra start -w <dir>",
                        );
                    }
                }
                Err(e) => {
                    let elapsed_ms = update_start.elapsed().as_millis() as u64;
                    let error_label = classify_update_error(&e);
                    zedra_host::telemetry::send_now(
                        &ga4,
                        Event::SelfUpdate {
                            success: false,
                            target_version: target_tag.clone(),
                            from_version: env!("CARGO_PKG_VERSION"),
                            error: error_label,
                            elapsed_ms,
                        },
                    )
                    .await;
                    utils::eprintln_error(format!("Update failed: {e}"));
                    std::process::exit(1);
                }
            }
        }

        Commands::Help { command } => {
            print_command_help(&command)?;
        }

        Commands::Stop { workdir, grace } => {
            let workdir = std::path::PathBuf::from(&workdir)
                .canonicalize()
                .unwrap_or_else(|_| std::path::PathBuf::from(&workdir));

            match workspace_lock::read_lock_info(&workdir)? {
                None => {
                    utils::eprintln_error(format!(
                        "No running Zedra daemon found for: {}",
                        workdir.display()
                    ));
                    std::process::exit(1);
                }
                Some(info) => {
                    if !workspace_lock::is_process_alive(info.pid) {
                        utils::eprintln_warn(format!(
                            "Process {} is already gone (stale lock). Cleaning up.",
                            info.pid
                        ));
                    } else {
                        utils::eprintln_heading("Stopping Zedra Daemon");
                        eprintln!();
                        utils::eprintln_key_values(&[
                            ("PID", info.pid.to_string()),
                            ("Workdir", info.workdir.clone()),
                            ("Host", info.hostname.clone()),
                            ("Started", info.running_for()),
                        ]);
                        eprintln!();
                    }
                }
            }

            workspace_lock::kill_and_unlock(&workdir, grace)?;
            utils::eprintln_success("Stopped.");
        }
    }

    Ok(())
}

fn print_command_help(command_path: &[String]) -> Result<()> {
    let mut command = Cli::command();
    let target = find_command_help_mut(&mut command, command_path)
        .ok_or_else(|| anyhow::anyhow!("unknown command: {}", command_path.join(" ")))?;
    target.print_help()?;
    println!();
    Ok(())
}

fn find_command_help_mut<'a>(
    command: &'a mut clap::Command,
    command_path: &[String],
) -> Option<&'a mut clap::Command> {
    let Some((name, rest)) = command_path.split_first() else {
        return Some(command);
    };
    let subcommand = command.find_subcommand_mut(name)?;
    find_command_help_mut(subcommand, rest)
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

fn daemon_log_path(workdir: &Path) -> Result<PathBuf> {
    Ok(identity::workspace_config_dir(workdir)?.join("daemon.log"))
}

fn read_recent_log_lines(log_path: &Path, lines: usize) -> Result<String> {
    let contents = std::fs::read_to_string(log_path)
        .with_context(|| format!("failed to read daemon log at {}", log_path.display()))?;
    if lines == 0 || contents.is_empty() {
        return Ok(String::new());
    }

    let mut selected = contents.lines().rev().take(lines).collect::<Vec<_>>();
    selected.reverse();
    let mut output = selected.join("\n");
    if !output.is_empty() && contents.ends_with('\n') {
        output.push('\n');
    }
    Ok(output)
}

fn render_detached_followup_commands() -> String {
    format!(
        "Commands\n{}\n\nFrom another directory, add `--workdir <path>`.",
        utils::render_shell_command_list(&[
            ("zedra qr", "Create a fresh one-time pairing QR."),
            ("zedra status", "Check current daemon status."),
            ("zedra stop", "Stop the daemon.")
        ])
    )
}

fn render_status_output(status: &serde_json::Value) -> String {
    let version = status["version"].as_str().unwrap_or("?");
    let workdir = status["workdir"].as_str().unwrap_or("?");
    let endpoint_id = status["endpoint_id"].as_str().unwrap_or("?");
    let uptime = status["uptime_secs"]
        .as_u64()
        .map(utils::format_duration)
        .unwrap_or_else(|| "-".to_string());
    let sessions = status["sessions"].as_array();
    let terminals = status["terminals"].as_array();
    let session_count = sessions.map(|sessions| sessions.len()).unwrap_or(0);
    let terminal_count = terminals.map(|terminals| terminals.len()).unwrap_or(0);
    let connected_sessions = sessions
        .map(|sessions| {
            sessions
                .iter()
                .filter(|session| session["is_occupied"].as_bool().unwrap_or(false))
                .count()
        })
        .unwrap_or(0);

    let mut sections = vec![
        "Zedra Daemon".to_string(),
        String::new(),
        utils::render_key_values(&[
            ("Version", format!("v{version}")),
            ("Uptime", uptime),
            ("Workdir", workdir.to_string()),
            ("Endpoint", short_id(endpoint_id)),
            ("Sessions", session_count.to_string()),
            ("Connected", connected_sessions.to_string()),
            ("Terminals", terminal_count.to_string()),
        ]),
    ];

    if let Some(sessions) = sessions {
        if !sessions.is_empty() {
            sections.push(String::new());
            sections.push("Sessions".to_string());
            sections.push(utils::render_table(
                &["ID", "NAME", "STATE", "TERMS", "UPTIME", "IDLE", "WORKDIR"],
                &sessions.iter().map(session_status_row).collect::<Vec<_>>(),
            ));
        }
    }

    if let Some(terminals) = terminals {
        if !terminals.is_empty() {
            sections.push(String::new());
            sections.push("Terminals".to_string());
            sections.push(utils::render_table(
                &["ID", "TITLE", "CREATED", "UPTIME", "SESSION"],
                &terminals
                    .iter()
                    .map(terminal_status_row)
                    .collect::<Vec<_>>(),
            ));
        }
    }

    sections.join("\n")
}

fn render_metrics_output(
    workdir: &Path,
    snapshot: &metrics::MetricsSnapshot,
    status: Option<&serde_json::Value>,
) -> String {
    let metrics = &snapshot.metrics;
    let daemon_running = status.is_some();
    let daemon = status
        .and_then(|status| status["uptime_secs"].as_u64())
        .map(|uptime| format!("running ({})", utils::format_duration(uptime)))
        .unwrap_or_else(|| "not running".to_string());
    let current_sessions = status
        .and_then(|status| status["sessions"].as_array())
        .map(|sessions| sessions.len() as u64);
    let current_terminals = status
        .and_then(|status| status["terminals"].as_array())
        .map(|terminals| terminals.len() as u64);
    let active_connections = if daemon_running {
        metrics.active_connection_count
    } else {
        0
    };
    let active_secs = display_active_secs(snapshot, daemon_running);

    let mut rows = vec![
        ("Workdir", workdir.display().to_string()),
        ("Daemon", daemon),
        ("Active Time", utils::format_duration(active_secs)),
        ("Active Now", format_count(active_connections, "connection")),
        ("Connections", metrics.successful_connections.to_string()),
        ("Pairings", metrics.new_pairings.to_string()),
        ("QR Codes", metrics.qr_codes_created.to_string()),
        (
            "Sessions",
            render_current_and_total(
                current_sessions,
                metrics.sessions_created,
                metrics.max_sessions_seen,
                "created",
            ),
        ),
        (
            "Terminals",
            render_current_and_total(
                current_terminals,
                metrics.terminals_created,
                metrics.max_terminals_seen,
                "created",
            ),
        ),
        (
            "Starts",
            format!(
                "{} total ({} detached)",
                metrics.daemon_starts, metrics.detached_starts
            ),
        ),
        (
            "Last Start",
            format_since_unix_secs(
                metrics.last_started_at_unix_secs,
                snapshot.generated_at_unix_secs,
            ),
        ),
        (
            "Last Connect",
            format_since_unix_secs(
                metrics.last_connected_at_unix_secs,
                snapshot.generated_at_unix_secs,
            ),
        ),
    ];

    if metrics.new_pairings > 0 {
        rows.push((
            "Last Pairing",
            format_since_unix_secs(
                metrics.last_pairing_at_unix_secs,
                snapshot.generated_at_unix_secs,
            ),
        ));
    }

    format!(
        "{}\n\n{}",
        utils::heading_text("Zedra Metrics"),
        utils::render_key_values(&rows)
    )
}

fn display_active_secs(snapshot: &metrics::MetricsSnapshot, daemon_running: bool) -> u64 {
    if daemon_running {
        return snapshot.active_secs;
    }

    let metrics = &snapshot.metrics;
    let stale_active_secs = if metrics.active_connection_count > 0 {
        metrics
            .active_started_at_unix_secs
            .zip(metrics.last_seen_at_unix_secs)
            .map(|(started, seen)| seen.saturating_sub(started))
            .unwrap_or_default()
    } else {
        0
    };
    metrics.total_active_secs.saturating_add(stale_active_secs)
}

fn render_current_and_total(
    current: Option<u64>,
    total: u64,
    max_seen: u64,
    total_label: &str,
) -> String {
    match current {
        Some(current) if total > 0 => format!("{current} current / {total} {total_label}"),
        Some(current) if max_seen > 0 => format!("{current} current / {max_seen} max"),
        Some(current) => format!("{current} current"),
        None if total > 0 && max_seen > 0 => format!("{total} {total_label} / {max_seen} max"),
        None if total > 0 => format!("{total} {total_label}"),
        None if max_seen > 0 => format!("{max_seen} max"),
        None => format!("0 {total_label}"),
    }
}

fn format_count(count: u64, singular: &str) -> String {
    if count == 1 {
        format!("1 {singular}")
    } else {
        format!("{count} {singular}s")
    }
}

fn format_since_unix_secs(timestamp: Option<u64>, now: u64) -> String {
    match timestamp {
        Some(timestamp) if timestamp <= now => {
            format!(
                "{} ago",
                utils::format_duration(now.saturating_sub(timestamp))
            )
        }
        Some(_) => "just now".to_string(),
        None => "-".to_string(),
    }
}

fn session_status_row(session: &serde_json::Value) -> Vec<String> {
    let id = session["id"]
        .as_str()
        .map(short_id)
        .unwrap_or_else(|| "-".to_string());
    let name = non_empty_str(&session["name"]).unwrap_or("-");
    let state = if session["is_occupied"].as_bool().unwrap_or(false) {
        "connected"
    } else {
        "idle"
    };
    let terminal_count = session["terminal_count"]
        .as_u64()
        .map(|count| count.to_string())
        .or_else(|| {
            session["terminals"]
                .as_array()
                .map(|terminals| terminals.len().to_string())
        })
        .unwrap_or_else(|| "-".to_string());
    let uptime = session["uptime_secs"]
        .as_u64()
        .map(utils::format_duration)
        .unwrap_or_else(|| "-".to_string());
    let idle = session["idle_secs"]
        .as_u64()
        .map(utils::format_duration)
        .unwrap_or_else(|| "-".to_string());
    let workdir = non_empty_str(&session["workdir"]).unwrap_or("-");

    vec![
        id,
        name.to_string(),
        state.to_string(),
        terminal_count,
        uptime,
        idle,
        workdir.to_string(),
    ]
}

fn terminal_status_row(terminal: &serde_json::Value) -> Vec<String> {
    let id = terminal["id"]
        .as_str()
        .map(short_id)
        .unwrap_or_else(|| "-".to_string());
    let title = non_empty_str(&terminal["title"]).unwrap_or("(untitled)");
    let created = terminal["created_at_elapsed_secs"]
        .as_u64()
        .map(|secs| format!("{} ago", utils::format_duration(secs)))
        .unwrap_or_else(|| "-".to_string());
    let uptime = terminal["uptime_secs"]
        .as_u64()
        .map(utils::format_duration)
        .unwrap_or_else(|| "-".to_string());
    let session = terminal["session_name"]
        .as_str()
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| terminal["session_id"].as_str().map(short_id))
        .unwrap_or_else(|| "-".to_string());

    vec![id, title.to_string(), created, uptime, session]
}

fn render_terminal_created_output(body: &str, launch_cmd: Option<&str>) -> Option<String> {
    let response = serde_json::from_str::<serde_json::Value>(body).ok()?;
    let id = response["id"].as_str()?;
    let session_id = response["session_id"].as_str()?;
    let mut rows = vec![("ID", id.to_string()), ("Session", session_id.to_string())];
    if let Some(launch_cmd) = launch_cmd.filter(|command| !command.is_empty()) {
        rows.push(("Command", launch_cmd.to_string()));
    }

    Some(format!(
        "Terminal Opened\n\n{}",
        utils::render_key_values(&rows)
    ))
}

fn non_empty_str(value: &serde_json::Value) -> Option<&str> {
    value.as_str().filter(|value| !value.is_empty())
}

async fn request_pairing_qr(workdir: &Path) -> Result<qr::StartupInfo> {
    let config_dir = identity::workspace_config_dir(workdir)?;
    let addr = std::fs::read_to_string(config_dir.join("api-addr")).unwrap_or_default();
    let token = std::fs::read_to_string(config_dir.join("api-token")).unwrap_or_default();
    if addr.trim().is_empty() {
        anyhow::bail!("No running daemon found for: {}", workdir.display());
    }

    let url = format!("http://{}/api/qr", addr.trim());
    let resp = reqwest::Client::new()
        .post(&url)
        .bearer_auth(token.trim())
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        if status == reqwest::StatusCode::NOT_FOUND {
            anyhow::bail!(
                "Running daemon does not support `zedra qr`; restart it with the updated zedra binary."
            );
        }
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Failed to request pairing QR: HTTP {} {}", status, body);
    }

    resp.json::<qr::StartupInfo>().await.map_err(Into::into)
}

async fn wait_for_detached_pairing_qr(workdir: &Path, pid: u32) -> Result<qr::StartupInfo> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);

    loop {
        if !workspace_lock::is_process_alive(pid) {
            anyhow::bail!("detached daemon exited before its pairing QR was ready");
        }

        let err = match request_pairing_qr(workdir).await {
            Ok(info) => return Ok(info),
            Err(err) => err,
        };

        if tokio::time::Instant::now() >= deadline {
            return Err(err).context("pairing QR was not ready before the startup timeout");
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
}

async fn fetch_instance_status(
    http: &reqwest::Client,
    workdir: &Path,
) -> Option<serde_json::Value> {
    let config_dir = identity::workspace_config_dir(workdir).ok()?;
    let addr = std::fs::read_to_string(config_dir.join("api-addr")).ok()?;
    let token = std::fs::read_to_string(config_dir.join("api-token")).ok()?;
    if addr.trim().is_empty() {
        return None;
    }

    let url = format!("http://{}/api/status", addr.trim());
    let resp = http.get(&url).bearer_auth(token.trim()).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.text()
        .await
        .ok()
        .and_then(|body| serde_json::from_str::<serde_json::Value>(&body).ok())
}

fn active_instance_row(
    lock: &workspace_lock::LockInfo,
    status: Option<&serde_json::Value>,
) -> Vec<String> {
    let version = status
        .and_then(|v| v["version"].as_str())
        .map(|version| format!("v{version}"))
        .unwrap_or_else(|| "-".to_string());
    let endpoint = status
        .and_then(|v| v["endpoint_id"].as_str())
        .map(short_id)
        .unwrap_or_else(|| "-".to_string());
    let uptime = status
        .and_then(|v| v["uptime_secs"].as_u64())
        .unwrap_or_else(|| elapsed_since_unix_secs(lock.started_secs));
    let workdir = status
        .and_then(|v| v["workdir"].as_str())
        .unwrap_or(&lock.workdir)
        .to_string();

    let sessions = status
        .and_then(|v| v["sessions"].as_array())
        .map(|sessions| sessions.len().to_string())
        .unwrap_or_else(|| "-".to_string());
    let terminals = status
        .and_then(|v| v["terminals"].as_array())
        .map(|terminals| terminals.len().to_string())
        .unwrap_or_else(|| "-".to_string());
    let state = status
        .map(|v| {
            let connected = v["sessions"]
                .as_array()
                .map(|sessions| {
                    sessions
                        .iter()
                        .any(|session| session["is_occupied"].as_bool().unwrap_or(false))
                })
                .unwrap_or(false);
            if connected {
                "connected"
            } else {
                "ready"
            }
        })
        .unwrap_or("no-api");

    vec![
        lock.pid.to_string(),
        state.to_string(),
        version,
        endpoint,
        utils::format_duration(uptime),
        sessions,
        terminals,
        workdir,
    ]
}

fn stale_instance_row(lock: &workspace_lock::LockInfo) -> Vec<String> {
    vec![
        lock.pid.to_string(),
        lock.running_for(),
        lock.workdir.clone(),
    ]
}

fn short_id(id: &str) -> String {
    if id.is_empty() || id == "?" {
        "-".to_string()
    } else {
        id[..id.len().min(8)].to_string()
    }
}

fn elapsed_since_unix_secs(started_secs: u64) -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs().saturating_sub(started_secs))
        .unwrap_or(0)
}

async fn generate_pairing_qr(
    registry: &Arc<session_registry::SessionRegistry>,
    session_id: &str,
    endpoint_id: iroh::PublicKey,
    endpoint: &iroh::Endpoint,
    relay_urls: &[String],
    json: bool,
    started_workdir: Option<&Path>,
    metrics_workdir: Option<&Path>,
) -> Result<()> {
    let ticket = ZedraPairingTicket {
        endpoint_id,
        handshake_secret: rand::random(),
        session_id: session_id.to_string(),
    };
    let info = qr::build_pairing_info(&ticket, endpoint, relay_urls)?;
    registry
        .add_pairing_slot(session_id, ticket.handshake_secret)
        .await;
    if let Some(workdir) = metrics_workdir {
        if let Err(e) = metrics::record_qr_created(workdir) {
            tracing::warn!("Failed to record QR metrics: {}", e);
        }
    }

    if json {
        qr::print_pairing_json(&info);
    } else {
        if let Some(workdir) = started_workdir {
            qr::print_started_pairing_info(&info, workdir);
        } else {
            qr::print_pairing_info(&info);
        }
        utils::println_warn("Note: this pairing QR is one-time use.");
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
    workdir: PathBuf,
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
                    if let Err(e) = generate_pairing_qr(&registry, &session_id, endpoint_id, &endpoint, &relay_urls, false, None, Some(&workdir)).await {
                        tracing::warn!("Failed to regenerate QR code: {}", e);
                    } else {
                        utils::eprintln_success("Regenerated pairing QR. Press 'r' again to refresh.");
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

    #[test]
    fn cli_version_flag_prints_package_version() {
        for flag in ["--version", "-v"] {
            match Cli::try_parse_from(["zedra", flag]) {
                Ok(cli) => assert!(cli.print_version, "{flag} should set the version flag"),
                Err(err) => panic!("{err}"),
            }
        }
        assert_eq!(
            render_cli_version(),
            format!("{}\n", env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn help_title_includes_version() {
        let help = Cli::command().render_help().to_string();

        assert!(help.contains(&format!(
            "Zedra CLI - Desktop daemon v{}",
            env!("CARGO_PKG_VERSION")
        )));
    }

    #[test]
    fn no_args_prints_help() {
        match Cli::try_parse_from(["zedra"]) {
            Ok(_) => panic!("missing command should print top-level help"),
            Err(err) => {
                assert_eq!(
                    err.kind(),
                    clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
                );
                assert!(err.to_string().contains("Commands:"));
            }
        }
    }

    #[test]
    fn start_detach_does_not_require_version_flag() {
        for args in [["zedra", "start", ""], ["zedra", "start", "--detach"]] {
            let args = args.into_iter().filter(|arg| !arg.is_empty());
            match Cli::try_parse_from(args) {
                Ok(cli) => {
                    assert!(!cli.print_version, "start should not set version output");
                    assert!(matches!(cli.command, Some(Commands::Start { .. })));
                }
                Err(err) => panic!("{err}"),
            }
        }
    }

    #[test]
    fn detached_start_child_args_preserve_start_options() {
        let args = detached_start_child_args(&DetachedStartOptions {
            workdir: PathBuf::from("project"),
            verbose: true,
            relay_url: vec![
                "https://sg1.relay.zedra.dev".to_string(),
                "https://us1.relay.zedra.dev".to_string(),
            ],
            no_telemetry: true,
            debug_telemetry: true,
            relay_only: true,
        });

        assert_eq!(
            args,
            vec![
                "--verbose",
                "start",
                "--workdir",
                "project",
                "--relay-url",
                "https://sg1.relay.zedra.dev",
                "--relay-url",
                "https://us1.relay.zedra.dev",
                "--no-telemetry",
                "--debug-telemetry",
                "--relay-only",
            ]
        );
    }

    #[test]
    fn active_instance_row_summarizes_status() {
        let lock = workspace_lock::LockInfo {
            pid: 42,
            workdir: "/fallback".to_string(),
            hostname: "host".to_string(),
            started_secs: 0,
        };
        let status = serde_json::json!({
            "version": "0.2.0",
            "endpoint_id": "abcdef123456",
            "uptime_secs": 65,
            "workdir": "/repo",
            "sessions": [
                { "is_occupied": true },
                { "is_occupied": false }
            ],
            "terminals": [{}, {}]
        });

        assert_eq!(
            active_instance_row(&lock, Some(&status)),
            vec![
                "42",
                "connected",
                "v0.2.0",
                "abcdef12",
                "1m5s",
                "2",
                "2",
                "/repo",
            ]
        );
    }

    #[test]
    fn detached_followup_commands_stays_short() {
        let output = render_detached_followup_commands();

        assert!(output.contains("Commands"));
        assert!(output.contains("  $ zedra qr      Create a fresh one-time pairing QR."));
        assert!(output.contains("  $ zedra status  Check current daemon status."));
        assert!(output.contains("  $ zedra stop    Stop the daemon."));
        assert!(!output.contains("zedra logs"));
        assert!(output.contains("From another directory, add `--workdir <path>`."));
    }

    #[test]
    fn read_recent_log_lines_limits_output() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("daemon.log");
        std::fs::write(&log_path, "one\ntwo\nthree\n").unwrap();

        assert_eq!(read_recent_log_lines(&log_path, 2).unwrap(), "two\nthree\n");
        assert_eq!(read_recent_log_lines(&log_path, 0).unwrap(), "");
    }

    #[test]
    fn status_output_includes_sessions_and_terminals() {
        let status = serde_json::json!({
            "version": "0.2.0",
            "workdir": "/repo",
            "endpoint_id": "abcdef123456",
            "uptime_secs": 65,
            "sessions": [
                {
                    "id": "session123456",
                    "name": "repo",
                    "terminal_count": 1,
                    "uptime_secs": 120,
                    "idle_secs": 5,
                    "is_occupied": true,
                    "workdir": "/repo"
                }
            ],
            "terminals": [
                {
                    "id": "terminal123456",
                    "title": "zsh",
                    "created_at_elapsed_secs": 10,
                    "uptime_secs": 10,
                    "session_name": "repo"
                }
            ]
        });

        let output = render_status_output(&status);

        assert!(output.contains("Zedra Daemon"));
        assert!(output.contains("  Version    v0.2.0"));
        assert!(output.contains("  Endpoint   abcdef12"));
        assert!(output.contains("Sessions"));
        assert!(output.contains("connected"));
        assert!(output.contains("Terminals"));
        assert!(output.contains("zsh"));
    }

    #[test]
    fn metrics_output_includes_local_and_live_counts() {
        let snapshot = metrics::MetricsSnapshot {
            metrics: metrics::WorkspaceMetrics {
                daemon_starts: 3,
                detached_starts: 2,
                successful_connections: 9,
                new_pairings: 1,
                qr_codes_created: 4,
                sessions_created: 1,
                terminals_created: 7,
                active_connection_count: 1,
                last_started_at_unix_secs: Some(90),
                last_connected_at_unix_secs: Some(95),
                ..metrics::WorkspaceMetrics::default()
            },
            generated_at_unix_secs: 100,
            active_secs: 3605,
        };
        let status = serde_json::json!({
            "uptime_secs": 120,
            "sessions": [{ "id": "session" }],
            "terminals": [{ "id": "terminal" }, { "id": "terminal-2" }]
        });

        let output = render_metrics_output(Path::new("/repo"), &snapshot, Some(&status));

        assert!(output.contains("Zedra Metrics"));
        assert!(output.contains("  Workdir       /repo"));
        assert!(output.contains("  Daemon        running (2m0s)"));
        assert!(output.contains("  Active Time   1h0m"));
        assert!(output.contains("  Active Now    1 connection"));
        assert!(output.contains("  Connections   9"));
        assert!(output.contains("  Pairings      1"));
        assert!(output.contains("  QR Codes      4"));
        assert!(output.contains("  Sessions      1 current / 1 created"));
        assert!(output.contains("  Terminals     2 current / 7 created"));
        assert!(output.contains("  Starts        3 total (2 detached)"));
        assert!(output.contains("  Last Start    10s ago"));
        assert!(output.contains("  Last Connect  5s ago"));
    }

    #[test]
    fn metrics_output_caps_stale_active_time_when_daemon_is_not_running() {
        let snapshot = metrics::MetricsSnapshot {
            metrics: metrics::WorkspaceMetrics {
                total_active_secs: 10,
                active_connection_count: 1,
                active_started_at_unix_secs: Some(100),
                last_seen_at_unix_secs: Some(130),
                ..metrics::WorkspaceMetrics::default()
            },
            generated_at_unix_secs: 500,
            active_secs: 410,
        };

        let output = render_metrics_output(Path::new("/repo"), &snapshot, None);

        assert!(output.contains("  Daemon        not running"));
        assert!(output.contains("  Active Time   40s"));
        assert!(output.contains("  Active Now    0 connections"));
    }

    #[test]
    fn terminal_created_output_summarizes_api_response() {
        let output = render_terminal_created_output(
            r#"{"id":"terminal-id","session_id":"session-id"}"#,
            Some("claude"),
        )
        .unwrap();

        assert!(output.contains("Terminal Opened"));
        assert!(output.contains("  ID       terminal-id"));
        assert!(output.contains("  Session  session-id"));
        assert!(output.contains("  Command  claude"));
    }

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
