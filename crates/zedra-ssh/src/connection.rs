// Connection state machine for SSH sessions

use std::sync::OnceLock;

use anyhow::Result;
use gpui::*;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use crate::TerminalSink;
use crate::client::SSHSession;

/// Global tokio runtime for SSH operations
fn ssh_runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        log::info!("Creating tokio runtime for SSH");
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime for SSH");
        log::info!("Tokio runtime created successfully");
        rt
    })
}

/// Connection state
#[derive(Clone, Debug, PartialEq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Authenticating,
    Connected,
    Error(String),
}

/// Authentication method
pub enum AuthMethod {
    Password { username: String, password: String },
    PairingToken { token: String },
}

/// Connection parameters
pub struct ConnectionParams {
    pub host: String,
    pub port: u16,
    pub auth: AuthMethod,
    pub expected_fingerprint: Option<String>,
}

/// Manages an SSH connection lifecycle
pub struct ConnectionManager {
    state: ConnectionState,
    _input_sender: Option<mpsc::UnboundedSender<Vec<u8>>>,
}

impl ConnectionManager {
    pub fn new() -> Self {
        Self {
            state: ConnectionState::Disconnected,
            _input_sender: None,
        }
    }

    pub fn state(&self) -> &ConnectionState {
        &self.state
    }

    /// Connect to a remote host and set up the terminal bridge.
    /// This spawns async work on the tokio runtime and updates the
    /// terminal view on the GPUI main thread.
    pub fn connect<T: TerminalSink>(
        terminal_view: WeakEntity<T>,
        params: ConnectionParams,
        cx: &mut App,
    ) {
        let view = terminal_view.clone();

        // Update state to Connecting
        let _ = view.update(cx, |this, cx| {
            this.set_status("Connecting...".to_string());
            cx.notify();
        });

        // Get terminal size before spawning
        let (cols, rows) = match terminal_view.update(cx, |this, _cx| this.terminal_size_cells()) {
            Ok(size) => size,
            Err(e) => {
                log::error!("Failed to get terminal size: {:?}", e);
                return;
            }
        };

        // Get the output buffer before spawning (this is on main thread)
        let output_buffer = match terminal_view.update(cx, |this, _cx| this.output_buffer()) {
            Ok(buf) => buf,
            Err(e) => {
                log::error!("Failed to get output buffer: {:?}", e);
                return;
            }
        };

        // Run connection on tokio runtime directly (GPUI spawn doesn't work on Android)
        log::info!("Spawning SSH connection task on tokio runtime");

        ssh_runtime().spawn(async move {
            log::info!("Tokio task started: connecting to {}:{}", params.host, params.port);

            let result = Self::do_connect_tokio(params, cols, rows).await;

            match result {
                Ok((sender, mut receiver)) => {
                    log::info!("SSH connection established, setting up terminal");

                    // Store the input sender globally so terminal view can use it
                    crate::set_input_sender(sender);
                    log::info!("SSH connected! Input channel ready.");

                    // Read loop for SSH output - write to the shared buffer
                    while let Some(data) = receiver.recv().await {
                        log::info!("Received {} bytes from SSH", data.len());
                        if let Ok(mut buffer) = output_buffer.lock() {
                            buffer.push_back(data);
                            log::info!("Buffer now has {} items", buffer.len());
                        }
                        // Signal main thread that data is available
                        crate::signal_terminal_data();
                    }
                    log::info!("SSH receiver closed");

                    // Clear the input sender when connection closes
                    crate::clear_input_sender();
                }
                Err(e) => {
                    log::error!("SSH connection failed: {:?}", e);
                }
            }
        });

        log::info!("SSH connection task spawned");
    }

    /// Connect to SSH on tokio runtime, returning channels for I/O
    async fn do_connect_tokio(
        params: ConnectionParams,
        cols: u32,
        rows: u32,
    ) -> Result<(mpsc::UnboundedSender<Vec<u8>>, mpsc::UnboundedReceiver<Vec<u8>>)> {
        log::info!("Connecting to {}:{}", params.host, params.port);

        // Connect via SSH
        let mut session =
            SSHSession::connect(&params.host, params.port, params.expected_fingerprint).await?;

        log::info!("TCP connected, authenticating...");

        // Authenticate
        let authenticated = match params.auth {
            AuthMethod::Password {
                ref username,
                ref password,
            } => session.auth_password(username, password).await?,
            AuthMethod::PairingToken { ref token } => {
                session.auth_password("zedra-pair", token).await?
            }
        };

        if !authenticated {
            return Err(anyhow::anyhow!("Authentication failed"));
        }

        log::info!("Authenticated, opening shell...");

        // Open shell with PTY
        let channel = session
            .open_shell(cols, rows)
            .await?;

        log::info!("Shell opened, starting I/O bridge");

        // Create channels for terminal I/O
        let (input_tx, mut input_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let (output_tx, output_rx) = mpsc::unbounded_channel::<Vec<u8>>();

        // Spawn tokio task to handle SSH I/O
        tokio::spawn(async move {
            use russh::ChannelMsg;

            let mut channel = channel;
            log::info!("SSH I/O task started, waiting for channel messages");

            loop {
                tokio::select! {
                    // Handle input from terminal (user typing)
                    Some(data) = input_rx.recv() => {
                        log::info!("Sending {} bytes to SSH channel", data.len());
                        if let Err(e) = channel.data(&data[..]).await {
                            log::error!("Failed to send data to SSH: {:?}", e);
                            break;
                        }
                    }
                    // Handle output from SSH (server responses)
                    msg = channel.wait() => {
                        match msg {
                            Some(ChannelMsg::Data { data }) => {
                                log::info!("SSH channel received {} bytes of data", data.len());
                                if output_tx.send(data.to_vec()).is_err() {
                                    log::info!("Output channel closed");
                                    break;
                                }
                            }
                            Some(ChannelMsg::ExtendedData { data, ext }) => {
                                log::info!("SSH channel received {} bytes of extended data (ext={})", data.len(), ext);
                                if output_tx.send(data.to_vec()).is_err() {
                                    log::info!("Output channel closed");
                                    break;
                                }
                            }
                            Some(ChannelMsg::Eof) => {
                                log::info!("SSH channel EOF");
                                break;
                            }
                            Some(ChannelMsg::Close) => {
                                log::info!("SSH channel closed");
                                break;
                            }
                            None => {
                                log::info!("SSH channel ended");
                                break;
                            }
                            other => {
                                log::debug!("SSH channel received other message: {:?}", other);
                            }
                        }
                    }
                }
            }

            log::info!("SSH I/O task finished");
        });

        Ok((input_tx, output_rx))
    }
}

impl Default for ConnectionManager {
    fn default() -> Self {
        Self::new()
    }
}
