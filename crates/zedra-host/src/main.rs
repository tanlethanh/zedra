// zedra-host: Desktop companion daemon for Zedra
//
// Provides an RPC daemon that Zedra (Android) connects to for remote terminal,
// filesystem, git, and AI operations over typed irpc.
//
// All connections go through iroh (QUIC/TLS 1.3) — handles LAN, relay, and
// hole-punched connections through a single Endpoint.

use anyhow::Result;
use clap::{Parser, Subcommand};

use zedra_host::{identity, iroh_listener, qr, rpc_daemon, session_registry};

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
    },
    /// Show QR code for pairing
    Qr,
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
        Commands::Start { workdir, json } => {
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

            let state = std::sync::Arc::new(rpc_daemon::DaemonState::new(workdir.clone()));

            // 1. Bind iroh endpoint
            let endpoint = iroh_listener::create_endpoint(&host_identity).await?;

            // 2. Generate QR code
            let addr = endpoint.addr();
            match qr::build_pairing_info(&addr) {
                Ok(info) => {
                    if json {
                        qr::print_pairing_json(&info);
                    } else {
                        qr::generate_pairing_qr(&addr).ok();
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to generate QR code: {}", e);
                }
            }

            // 3. Spawn session cleanup (background task)
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

            // 4. Run iroh accept loop (blocks main)
            iroh_listener::run_accept_loop(&endpoint, registry, state).await?;
        }
        Commands::Qr => {
            let host_identity = identity::HostIdentity::load_or_generate()?;
            let id = std::sync::Arc::new(host_identity);
            let endpoint = iroh_listener::create_endpoint(&id).await?;
            if let Err(e) = qr::generate_pairing_qr(&endpoint.addr()) {
                eprintln!("Failed to generate QR code: {}", e);
            }
        }
    }

    Ok(())
}
