// SSH server using russh
// Handles incoming connections, authentication, and PTY/shell sessions

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use russh::server::{self, Auth, Handler, Msg, Server, Session};
use russh::{Channel, ChannelId, CryptoVec, MethodSet};
use tokio::sync::Mutex;

use crate::auth;
use crate::pty::ShellSession;
use crate::store;

/// Run the SSH server
pub async fn run_server(bind: &str, port: u16) -> Result<()> {
    let host_key = load_or_generate_host_key()?;

    let config = server::Config {
        keys: vec![host_key],
        auth_rejection_time: std::time::Duration::from_secs(1),
        auth_rejection_time_initial: Some(std::time::Duration::from_secs(0)),
        ..Default::default()
    };

    let config = Arc::new(config);
    let addr = format!("{}:{}", bind, port);

    tracing::info!("SSH server listening on {}", addr);

    let mut server = ZedraServer;
    server.run_on_address(config, &addr).await?;

    Ok(())
}

/// Load host key from disk or generate a new one
fn load_or_generate_host_key() -> Result<russh_keys::PrivateKey> {
    let key_path = store::host_key_path()?;

    if key_path.exists() {
        let key_data = std::fs::read_to_string(&key_path)?;
        let key = russh_keys::decode_secret_key(&key_data, None)?;
        tracing::info!("Loaded host key from {:?}", key_path);
        Ok(key)
    } else {
        tracing::info!("Generating new host key");
        let key = russh_keys::PrivateKey::random(
            &mut rand::thread_rng(),
            russh_keys::Algorithm::Ed25519,
        )
        .map_err(|e| anyhow::anyhow!("Failed to generate key: {}", e))?;

        // Save to disk
        let openssh = key
            .to_openssh(ssh_key::LineEnding::LF)
            .map_err(|e| anyhow::anyhow!("Failed to serialize key: {}", e))?;
        std::fs::create_dir_all(key_path.parent().unwrap())?;
        std::fs::write(&key_path, openssh.as_bytes())?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))?;
        }

        Ok(key)
    }
}

/// Server factory - creates handlers for each new connection
struct ZedraServer;

#[async_trait]
impl server::Server for ZedraServer {
    type Handler = ZedraHandler;

    fn new_client(&mut self, addr: Option<std::net::SocketAddr>) -> Self::Handler {
        tracing::info!("New connection from {:?}", addr);
        ZedraHandler {
            username: None,
            shells: Arc::new(Mutex::new(HashMap::new())),
            pending_readers: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

/// Per-connection handler
struct ZedraHandler {
    username: Option<String>,
    shells: Arc<Mutex<HashMap<ChannelId, ShellState>>>,
    /// Readers are stored separately and moved to the read task on shell_request.
    /// This avoids holding a lock during blocking PTY reads.
    pending_readers: Arc<Mutex<HashMap<ChannelId, Box<dyn Read + Send>>>>,
}

struct ShellState {
    writer: Box<dyn Write + Send>,
    master: Box<dyn portable_pty::MasterPty + Send>,
}

#[async_trait]
impl Handler for ZedraHandler {
    type Error = anyhow::Error;

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        tracing::info!("Channel open session: {}", channel.id());
        Ok(true)
    }

    async fn auth_password(
        &mut self,
        user: &str,
        password: &str,
    ) -> Result<Auth, Self::Error> {
        tracing::info!("Password auth attempt for user: {}", user);

        match auth::authenticate(user, password) {
            Ok(true) => {
                self.username = Some(user.to_string());
                tracing::info!("Authentication successful for {}", user);
                Ok(Auth::Accept)
            }
            Ok(false) => {
                tracing::warn!("Authentication failed for {}", user);
                Ok(Auth::Reject {
                    proceed_with_methods: Some(MethodSet::PASSWORD),
                })
            }
            Err(e) => {
                tracing::error!("Authentication error: {:?}", e);
                Ok(Auth::Reject {
                    proceed_with_methods: Some(MethodSet::PASSWORD),
                })
            }
        }
    }

    async fn pty_request(
        &mut self,
        channel_id: ChannelId,
        term: &str,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(russh::Pty, u32)],
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        tracing::info!(
            "PTY request: term={}, size={}x{}",
            term,
            col_width,
            row_height
        );

        // Spawn shell with PTY
        let shell = ShellSession::spawn(col_width as u16, row_height as u16)?;
        let (reader, writer, master) = shell.take_reader();

        // Store reader separately for the read task (will be taken in shell_request)
        // Store writer + master in shared map for data() and resize operations
        self.shells.lock().await.insert(
            channel_id,
            ShellState {
                writer,
                master,
            },
        );

        // Store reader in a separate map for the spawned read task
        self.pending_readers.lock().await.insert(channel_id, reader);

        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel_id: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        tracing::info!("Shell request on channel {}", channel_id);

        // Take the reader out - it's only used by the read task, no lock contention
        let reader = self.pending_readers.lock().await.remove(&channel_id);
        let mut reader = match reader {
            Some(r) => r,
            None => {
                tracing::error!("No reader for channel {}", channel_id);
                return Ok(());
            }
        };

        let shells = self.shells.clone();
        let handle = session.handle();

        // Spawn a task to read from PTY and send to SSH channel
        // Reader is moved here - no lock needed during blocking read
        tokio::spawn(async move {
            let mut buf = [0u8; 4096];
            loop {
                // Blocking read without holding any lock
                let n = match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(e) => {
                        tracing::debug!("PTY read error: {:?}", e);
                        break;
                    }
                };

                let data = CryptoVec::from_slice(&buf[..n]);
                if handle.data(channel_id, data).await.is_err() {
                    break;
                }
            }

            // Shell exited - clean up and close channel
            shells.lock().await.remove(&channel_id);
            let _ = handle.close(channel_id).await;
            tracing::info!("Shell exited on channel {}", channel_id);
        });

        Ok(())
    }

    async fn data(
        &mut self,
        channel_id: ChannelId,
        data: &[u8],
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        // Forward data from SSH channel to PTY stdin
        let mut shells = self.shells.lock().await;
        if let Some(shell) = shells.get_mut(&channel_id) {
            shell.writer.write_all(data)?;
            shell.writer.flush()?;
        }
        Ok(())
    }

    async fn window_change_request(
        &mut self,
        channel_id: ChannelId,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        tracing::info!(
            "Window change: {}x{} on channel {}",
            col_width,
            row_height,
            channel_id
        );

        let shells = self.shells.lock().await;
        if let Some(shell) = shells.get(&channel_id) {
            shell
                .master
                .resize(portable_pty::PtySize {
                    rows: row_height as u16,
                    cols: col_width as u16,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(|e| anyhow::anyhow!("Failed to resize PTY: {}", e))?;
        }

        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel_id: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let command = String::from_utf8_lossy(data).to_string();
        tracing::info!("Exec request: {}", command);

        // Handle key registration during pairing
        if command.starts_with("zedra-register-key ") {
            let public_key = command.trim_start_matches("zedra-register-key ").trim();
            let device = store::PairedDevice {
                id: uuid_v4(),
                name: format!(
                    "device-{}",
                    &public_key[..8.min(public_key.len())]
                ),
                public_key: public_key.to_string(),
                paired_at: format!(
                    "{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs()
                ),
                last_connected: None,
            };

            match store::add_device(device) {
                Ok(()) => {
                    tracing::info!("Device registered with key");
                    let response = CryptoVec::from_slice(b"OK\n");
                    let _ = session.handle().data(channel_id, response).await;
                }
                Err(e) => {
                    tracing::error!("Failed to register device: {:?}", e);
                    let response =
                        CryptoVec::from_slice(format!("ERROR: {}\n", e).as_bytes());
                    let _ = session.handle().data(channel_id, response).await;
                }
            }

            let _ = session.handle().close(channel_id).await;
        }

        Ok(())
    }
}

fn uuid_v4() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    // Set version 4 and variant bits
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{}-{}-{}-{}-{}",
        hex::encode(&bytes[0..4]),
        hex::encode(&bytes[4..6]),
        hex::encode(&bytes[6..8]),
        hex::encode(&bytes[8..10]),
        hex::encode(&bytes[10..16])
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uuid_v4_format() {
        let uuid = uuid_v4();
        let parts: Vec<&str> = uuid.split('-').collect();
        assert_eq!(parts.len(), 5);
        assert_eq!(parts[0].len(), 8);
        assert_eq!(parts[1].len(), 4);
        assert_eq!(parts[2].len(), 4);
        assert_eq!(parts[3].len(), 4);
        assert_eq!(parts[4].len(), 12);
    }

    #[test]
    fn test_uuid_v4_version_bit() {
        let uuid = uuid_v4();
        // Third group should start with '4' (version 4)
        let parts: Vec<&str> = uuid.split('-').collect();
        assert!(parts[2].starts_with('4'));
    }

    #[test]
    fn test_uuid_v4_variant_bits() {
        let uuid = uuid_v4();
        let parts: Vec<&str> = uuid.split('-').collect();
        // Fourth group first char should be 8, 9, a, or b (variant 1)
        let first_char = parts[3].chars().next().unwrap();
        assert!(
            "89ab".contains(first_char),
            "Expected variant char in [8,9,a,b], got: {}",
            first_char
        );
    }

    #[test]
    fn test_uuid_v4_uniqueness() {
        let u1 = uuid_v4();
        let u2 = uuid_v4();
        assert_ne!(u1, u2);
    }

    #[test]
    fn test_uuid_v4_is_valid_hex() {
        let uuid = uuid_v4();
        let hex_only = uuid.replace('-', "");
        assert!(hex_only.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(hex_only.len(), 32);
    }
}
