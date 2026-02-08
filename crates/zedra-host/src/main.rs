// zedra-host: Desktop companion daemon for Zedra terminal
//
// Provides an SSH server that Zedra (Android) connects to for remote terminal access.
// Supports QR code pairing for easy setup and password fallback authentication.

use anyhow::Result;
use clap::{Parser, Subcommand};

use zedra_host::{auth, qr, rpc_daemon, server, store};

#[derive(Parser)]
#[command(name = "zedra-host", about = "Desktop companion daemon for Zedra terminal")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the SSH server daemon (foreground)
    Start {
        /// Port to listen on
        #[arg(short, long, default_value = "2222")]
        port: u16,

        /// Bind address
        #[arg(short, long, default_value = "0.0.0.0")]
        bind: String,
    },
    /// Generate a QR code for pairing with a mobile device
    Pair,
    /// List paired devices
    Devices,
    /// Revoke a paired device
    Revoke {
        /// Device ID to revoke
        device_id: String,
    },
    /// Start the RPC daemon (filesystem, git, terminal over JSON-RPC)
    Daemon {
        /// Port to listen on
        #[arg(short, long, default_value = "2223")]
        port: u16,

        /// Bind address
        #[arg(short, long, default_value = "0.0.0.0")]
        bind: String,

        /// Working directory to serve
        #[arg(short, long, default_value = ".")]
        workdir: String,
    },
    /// Set or update the fallback password
    SetPassword,
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
        Commands::Start { port, bind } => {
            tracing::info!("Starting zedra-host on {}:{}", bind, port);
            server::run_server(&bind, port).await?;
        }
        Commands::Pair => {
            qr::generate_pairing_qr().await?;
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
        Commands::Daemon {
            port,
            bind,
            workdir,
        } => {
            let workdir = std::path::PathBuf::from(workdir)
                .canonicalize()
                .unwrap_or_else(|_| std::path::PathBuf::from("."));
            tracing::info!("Starting RPC daemon on {}:{} serving {}", bind, port, workdir.display());
            rpc_daemon::run_daemon(&bind, port, workdir).await?;
        }
        Commands::SetPassword => {
            let password = rpassword_prompt("Enter new password: ")?;
            let confirm = rpassword_prompt("Confirm password: ")?;
            if password != confirm {
                anyhow::bail!("Passwords do not match");
            }
            auth::set_password(&password)?;
            println!("Password updated.");
        }
    }

    Ok(())
}

fn rpassword_prompt(prompt: &str) -> Result<String> {
    use std::io::{self, Write};
    print!("{}", prompt);
    io::stdout().flush()?;
    let mut password = String::new();
    io::stdin().read_line(&mut password)?;
    Ok(password.trim().to_string())
}
