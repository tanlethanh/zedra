// zedra-host: Desktop companion daemon for Zedra
//
// Provides an RPC daemon that Zedra (Android) connects to for remote terminal,
// filesystem, git, and AI operations over JSON-RPC.
//
// All connections go through iroh (QUIC/TLS 1.3) — handles LAN, relay, and
// hole-punched connections through a single Endpoint.

use anyhow::Result;
use clap::{Parser, Subcommand};

use zedra_host::{identity, iroh_listener, qr, rpc_daemon, session_registry, store};

#[derive(Parser)]
#[command(name = "zedra-host", about = "Desktop companion daemon for Zedra")]
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

        /// Relay/coordination server URL (for CF Worker discovery)
        #[arg(long, default_value = zedra_transport::DEFAULT_RELAY_URL)]
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
    /// Show QR code for pairing
    Qr,
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
            workdir,
            relay_url,
            no_relay,
        } => {
            let workdir = std::path::PathBuf::from(workdir)
                .canonicalize()
                .unwrap_or_else(|_| std::path::PathBuf::from("."));

            tracing::info!("Starting zedra-host (iroh transport)");
            tracing::info!("Serving workdir: {}", workdir.display());

            // Load or generate persistent host identity
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
            tracing::info!(
                "Created session '{}' for {}",
                session_name,
                workdir.display()
            );

            let mut state = rpc_daemon::DaemonState::new(workdir.clone());
            state = state.with_identity(host_identity.clone());
            let state = std::sync::Arc::new(state);

            // 1. Bind iroh endpoint
            let coord = if no_relay {
                None
            } else {
                Some(relay_url.as_str())
            };
            let endpoint = iroh_listener::create_endpoint(&host_identity, coord).await?;

            // 2. Generate QR code (needs endpoint info)
            let endpoint_info = iroh_listener::get_endpoint_info(&endpoint);
            if let Err(e) = qr::generate_pairing_qr(&endpoint_info, &host_identity, coord) {
                tracing::warn!("Failed to generate QR code: {}", e);
            }

            // 3. Spawn CF Worker publish loop (background task)
            if let Some(url) = coord {
                let publish_url = url.to_string();
                let publish_endpoint = endpoint.clone();
                tokio::spawn(async move {
                    iroh_listener::run_publish_loop(&publish_url, &publish_endpoint).await;
                });
            }

            // 4. Spawn session cleanup (background task)
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

            // 5. Run iroh accept loop (blocks main)
            iroh_listener::run_accept_loop(&endpoint, registry, state).await?;
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
            SessionAction::Qr => {
                // Load identity and create a temporary endpoint for QR generation.
                let host_identity = identity::HostIdentity::load_or_generate()?;
                let id = std::sync::Arc::new(host_identity);
                let endpoint = iroh_listener::create_endpoint(&id, None).await?;
                let endpoint_info = iroh_listener::get_endpoint_info(&endpoint);
                if let Err(e) = qr::generate_pairing_qr(&endpoint_info, &id, None) {
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
