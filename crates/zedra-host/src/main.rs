// zedra-host: Desktop companion daemon for Zedra
//
// Provides an RPC daemon that Zedra (Android) connects to for remote terminal,
// filesystem, git, and AI operations over JSON-RPC.
//
// A single `start` command handles both LAN and relay transports. LAN TCP is
// always available; relay is coordinated via the coordination server.

use anyhow::Result;
use clap::{Parser, Subcommand};

use zedra_host::{identity, qr, registration, rpc_daemon, session_registry, store};

#[derive(Parser)]
#[command(name = "zedra-host", about = "Desktop companion daemon for Zedra")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the daemon (LAN + optional relay)
    Start {
        /// Port to listen on
        #[arg(short, long, default_value = "2123")]
        port: u16,

        /// Bind address
        #[arg(short, long, default_value = "0.0.0.0")]
        bind: String,

        /// Working directory to serve
        #[arg(short, long, default_value = ".")]
        workdir: String,

        /// Relay server URL (omit to disable relay)
        #[arg(long, default_value = zedra_relay::DEFAULT_RELAY_URL)]
        relay_url: String,

        /// Disable relay transport
        #[arg(long)]
        no_relay: bool,
    },
    /// List paired devices
    Devices,
    /// Revoke a paired device
    Revoke {
        /// Device ID to revoke
        device_id: String,
    },
    /// Session management
    Session {
        #[command(subcommand)]
        action: SessionAction,
    },
}

#[derive(Subcommand)]
enum SessionAction {
    /// Create a named session for a working directory
    Create {
        /// Human-readable session name
        #[arg(short, long)]
        name: String,
        /// Working directory for this session
        #[arg(short, long)]
        workdir: String,
    },
    /// List active sessions
    List,
    /// Show QR code for a specific session
    Qr {
        /// Session name
        name: String,
    },
    /// Remove a named session
    Remove {
        /// Session name
        name: String,
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
        Commands::Start {
            port,
            bind,
            workdir,
            relay_url,
            no_relay,
        } => {
            let workdir = std::path::PathBuf::from(workdir)
                .canonicalize()
                .unwrap_or_else(|_| std::path::PathBuf::from("."));

            let local_ip = qr::get_local_ip().unwrap_or_else(|| "unknown".to_string());
            tracing::info!("Starting zedra-host on {}:{}", bind, port);
            tracing::info!("Local IP: {}", local_ip);
            tracing::info!("Serving workdir: {}", workdir.display());

            // Load or generate persistent host identity (required for v3 encryption)
            let host_identity = match identity::HostIdentity::load_or_generate() {
                Ok(id) => std::sync::Arc::new(id),
                Err(e) => {
                    anyhow::bail!("Failed to load host identity: {}", e);
                }
            };

            let registry = std::sync::Arc::new(session_registry::SessionRegistry::new());

            // Create a named session for the working directory.
            let session_name = workdir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("default")
                .to_string();
            registry
                .create_named(&session_name, workdir.clone(), "auto")
                .await;
            tracing::info!("Created session '{}' for {}", session_name, workdir.display());

            let mut state = rpc_daemon::DaemonState::new(workdir.clone());
            state = state.with_identity(host_identity.clone());
            let state = std::sync::Arc::new(state);

            // Show QR code
            if let Err(e) = qr::generate_pairing_qr_v3(port, &host_identity, None) {
                tracing::warn!("Failed to generate v3 QR code: {}", e);
            }

            // Spawn coordination server registration loop (non-fatal)
            if !no_relay {
                let reg_config = registration::RegistrationConfig {
                    coord_url: relay_url.clone(),
                    identity: host_identity.clone(),
                    port,
                    workdir: workdir.clone(),
                    version: env!("CARGO_PKG_VERSION").to_string(),
                };
                tokio::spawn(registration::run_registration_loop(reg_config));
            }

            // Spawn mDNS/UDP local discovery announcer (non-fatal)
            {
                let lan_addrs = qr::collect_lan_addrs();
                let addresses: Vec<String> = lan_addrs
                    .iter()
                    .map(|ip| format!("{}:{}", ip, port))
                    .collect();

                let announcement = zedra_transport::Announcement {
                    v: 1,
                    device_id: host_identity.device_id.to_string(),
                    addresses,
                    sessions: vec![workdir
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("default")
                        .to_string()],
                };
                tokio::spawn(zedra_transport::mdns::run_announcer(announcement));
                tracing::info!("mDNS local discovery announcer started");
            }

            // Spawn session cleanup task
            let cleanup_registry = registry.clone();
            tokio::spawn(async move {
                let grace_period = std::time::Duration::from_secs(300);
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                    let removed = cleanup_registry.cleanup(grace_period).await;
                    if !removed.is_empty() {
                        tracing::info!("Cleaned up {} idle sessions", removed.len());
                    }
                }
            });

            // Run LAN TCP listener (blocks)
            rpc_daemon::run_daemon(&bind, port, registry, state).await?;
        }
        Commands::Devices => {
            let devices = store::list_devices()?;
            if devices.is_empty() {
                println!("No paired devices.");
            } else {
                println!("{:<36} {:<20} {:<24}", "ID", "Name", "Paired At");
                println!("{}", "-".repeat(80));
                for device in devices {
                    println!(
                        "{:<36} {:<20} {:<24}",
                        device.id, device.name, device.paired_at
                    );
                }
            }
        }
        Commands::Revoke { device_id } => {
            store::revoke_device(&device_id)?;
            println!("Device {} revoked.", device_id);
        }
        Commands::Session { action } => match action {
            SessionAction::Create { name, workdir } => {
                let workdir = std::path::PathBuf::from(workdir)
                    .canonicalize()
                    .unwrap_or_else(|_| std::path::PathBuf::from("."));
                println!("Session '{}' created for {}", name, workdir.display());
                println!(
                    "Note: sessions are created in-memory when the daemon starts. \
                     Use `zedra-host start --workdir {}` to serve this directory.",
                    workdir.display()
                );
            }
            SessionAction::List => {
                println!("Sessions are only available while the daemon is running.");
                println!("Connect a client and use the session/list RPC method,");
                println!("or check daemon logs for active sessions.");
            }
            SessionAction::Qr { name } => {
                // Load identity for QR generation.
                let host_identity = identity::HostIdentity::load_or_generate()?;
                let id = std::sync::Arc::new(host_identity);
                println!("QR code for session '{}':", name);
                if let Err(e) = qr::generate_pairing_qr_v3(2123, &id, None) {
                    eprintln!("Failed to generate QR code: {}", e);
                }
            }
            SessionAction::Remove { name } => {
                println!("Session '{}' marked for removal.", name);
                println!("Active sessions are managed by the running daemon.");
            }
        },
    }

    Ok(())
}
