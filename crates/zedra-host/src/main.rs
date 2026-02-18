// zedra-host: Desktop companion daemon for Zedra
//
// Provides an RPC daemon that Zedra (Android) connects to for remote terminal,
// filesystem, git, and AI operations over JSON-RPC.
//
// A single `start` command handles both LAN and relay transports. LAN TCP is
// always available; relay is attempted automatically when --relay-url is set
// and the relay server is reachable.

use anyhow::Result;
use clap::{Parser, Subcommand};

use zedra_host::{qr, relay_bridge, rpc_daemon, session_registry, store};

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

            let registry = std::sync::Arc::new(session_registry::SessionRegistry::new());
            let state = std::sync::Arc::new(rpc_daemon::DaemonState::new(workdir));

            // Try to register a relay room (non-fatal on failure)
            let relay_info = if no_relay {
                tracing::info!("Relay disabled (--no-relay)");
                None
            } else {
                tracing::info!("Registering relay room at {}...", relay_url);
                relay_bridge::try_register_relay(&relay_url).await
            };

            // Show QR code with all available transports
            if let Err(e) = qr::generate_pairing_qr(port, relay_info.as_ref()) {
                tracing::warn!("Failed to generate QR code: {}", e);
            }

            // Spawn relay acceptor alongside LAN TCP if relay registered
            if let Some(info) = relay_info {
                let relay_registry = registry.clone();
                let relay_state = state.clone();
                tokio::spawn(async move {
                    relay_bridge::accept_relay_connections(info, relay_registry, relay_state).await;
                });
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
    }

    Ok(())
}
