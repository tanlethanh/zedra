// zedra-host: Desktop companion daemon for Zedra
//
// Provides an RPC daemon that Zedra (Android) connects to for remote terminal,
// filesystem, git, and AI operations over JSON-RPC.

use anyhow::Result;
use clap::{Parser, Subcommand};

use zedra_host::{qr, rpc_daemon, store};

#[derive(Parser)]
#[command(name = "zedra-host", about = "Desktop companion daemon for Zedra")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the RPC daemon (foreground)
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
    // Initialize logging
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
        } => {
            let workdir = std::path::PathBuf::from(workdir)
                .canonicalize()
                .unwrap_or_else(|_| std::path::PathBuf::from("."));
            let local_ip = qr::get_local_ip().unwrap_or_else(|| "unknown".to_string());
            tracing::info!("Starting zedra-host on {}:{}", bind, port);
            tracing::info!("Local IP: {} (use this to connect from Zedra)", local_ip);
            tracing::info!("Serving workdir: {}", workdir.display());
            rpc_daemon::run_daemon(&bind, port, workdir).await?;
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
