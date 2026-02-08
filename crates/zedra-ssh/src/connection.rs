// Connection state machine for SSH sessions

use anyhow::Result;
use gpui::*;
use tokio::sync::mpsc;

use crate::TerminalSink;
use crate::bridge::SSHBridge;
use crate::client::SSHSession;

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

        // Spawn the connection on tokio
        cx.spawn(async move |cx| {
            let result = Self::do_connect(terminal_view.clone(), params, cx).await;

            match result {
                Ok(sender) => {
                    let _ = terminal_view.update(cx, |this, cx| {
                        this.set_connected(true);
                        this.set_status("Connected".to_string());

                        // Set up the send callback
                        let sender_clone = sender.clone();
                        this.set_send_bytes(Box::new(move |bytes| {
                            let _ = sender_clone.send(bytes);
                        }));

                        cx.notify();
                    });
                }
                Err(e) => {
                    log::error!("Connection failed: {:?}", e);
                    let _ = terminal_view.update(cx, |this, cx| {
                        this.set_connected(false);
                        this.set_status(format!("Error: {}", e));
                        cx.notify();
                    });
                }
            }

            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    async fn do_connect<T: TerminalSink>(
        terminal_view: WeakEntity<T>,
        params: ConnectionParams,
        cx: &mut AsyncApp,
    ) -> Result<mpsc::UnboundedSender<Vec<u8>>> {
        // Get terminal size
        let (cols, rows) = terminal_view.update(cx, |this, _cx| this.terminal_size_cells())?;

        // Connect via SSH
        let mut session =
            SSHSession::connect(&params.host, params.port, params.expected_fingerprint).await?;

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

        // Open shell with PTY
        let channel = session
            .open_shell(cols, rows)
            .await?;

        // Start the I/O bridge
        let bridge = SSHBridge::start(channel, terminal_view.clone(), cx)?;

        Ok(bridge.sender())
    }
}

impl Default for ConnectionManager {
    fn default() -> Self {
        Self::new()
    }
}
