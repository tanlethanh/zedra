// SSH client using russh
// Connects to zedra-host or any SSH server

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use russh::client::{self, Handler, Msg};
use russh::keys::{ssh_key, HashAlg};
use russh::{Channel, Disconnect};

/// SSH client handler implementing russh::client::Handler
pub struct ZedraSSHClient {
    /// Expected host key fingerprint (from QR code pairing), or None for TOFU
    expected_fingerprint: Option<String>,
}

impl ZedraSSHClient {
    pub fn new(expected_fingerprint: Option<String>) -> Self {
        Self {
            expected_fingerprint,
        }
    }
}

#[async_trait]
impl Handler for ZedraSSHClient {
    type Error = anyhow::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        let fingerprint = server_public_key.fingerprint(HashAlg::Sha256);
        log::info!("Server key fingerprint: {}", fingerprint);

        if let Some(ref expected) = self.expected_fingerprint {
            let matches = fingerprint.to_string() == *expected;
            if !matches {
                log::error!(
                    "Host key mismatch! Expected: {}, Got: {}",
                    expected,
                    fingerprint
                );
            }
            Ok(matches)
        } else {
            log::info!("Accepting server key (TOFU)");
            Ok(true)
        }
    }
}

/// Authenticated SSH session wrapper
pub struct SSHSession {
    session: client::Handle<ZedraSSHClient>,
}

impl SSHSession {
    /// Connect to an SSH server
    pub async fn connect(
        host: &str,
        port: u16,
        expected_fingerprint: Option<String>,
    ) -> Result<Self> {
        let config = client::Config {
            ..Default::default()
        };

        let handler = ZedraSSHClient::new(expected_fingerprint);
        let session = client::connect(Arc::new(config), (host, port), handler).await?;

        Ok(Self { session })
    }

    /// Authenticate with password
    pub async fn auth_password(&mut self, username: &str, password: &str) -> Result<bool> {
        let result = self
            .session
            .authenticate_password(username, password)
            .await?;
        Ok(result)
    }

    /// Open a shell channel with PTY
    pub async fn open_shell(
        &mut self,
        columns: u32,
        rows: u32,
    ) -> Result<Channel<Msg>> {
        let channel = self.session.channel_open_session().await?;

        // Request PTY
        channel
            .request_pty(
                false,
                "xterm-256color",
                columns,
                rows,
                0, // pixel width
                0, // pixel height
                &[],
            )
            .await?;

        // Request shell
        channel.request_shell(false).await?;

        Ok(channel)
    }

    /// Send window size change to the server
    pub async fn window_change(
        &mut self,
        channel: &Channel<Msg>,
        columns: u32,
        rows: u32,
    ) -> Result<()> {
        channel
            .window_change(columns, rows, 0, 0)
            .await?;
        Ok(())
    }

    /// Disconnect the session
    pub async fn disconnect(&self) -> Result<()> {
        self.session
            .disconnect(Disconnect::ByApplication, "bye", "en")
            .await?;
        Ok(())
    }
}
