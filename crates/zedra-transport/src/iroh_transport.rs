// IrohTransport: adapter wrapping iroh QUIC bidirectional streams as the
// existing Transport trait. This lets the RpcClient work unchanged over iroh.

use anyhow::Result;
use async_trait::async_trait;
use iroh::endpoint::{RecvStream, SendStream};
use tokio::sync::mpsc;
use zedra_rpc::Transport;

/// Transport adapter over an iroh QUIC bidirectional stream.
///
/// Implements length-prefix framing (4-byte big-endian length + payload)
/// over QUIC's reliable ordered byte stream, matching the framing used
/// by TcpTransport and other Transport implementations.
pub struct IrohTransport {
    send: SendStream,
    recv: RecvStream,
}

impl IrohTransport {
    pub fn new(send: SendStream, recv: RecvStream) -> Self {
        Self { send, recv }
    }
}

#[async_trait]
impl Transport for IrohTransport {
    async fn send(&mut self, payload: &[u8]) -> Result<()> {
        let len = (payload.len() as u32).to_be_bytes();
        self.send.write_all(&len).await?;
        self.send.write_all(payload).await?;
        Ok(())
    }

    async fn recv(&mut self) -> Result<Vec<u8>> {
        let mut len_buf = [0u8; 4];
        self.recv.read_exact(&mut len_buf).await?;
        let len = u32::from_be_bytes(len_buf) as usize;

        if len > 16 * 1024 * 1024 {
            anyhow::bail!("iroh message too large: {} bytes", len);
        }

        let mut buf = vec![0u8; len];
        self.recv.read_exact(&mut buf).await?;
        Ok(buf)
    }

    fn name(&self) -> &str {
        "iroh-quic"
    }
}

impl IrohTransport {
    /// Convert this transport into mpsc channel pairs suitable for
    /// `RpcClient::spawn_from_channels()`.
    ///
    /// Spawns two background tasks: one bridging recv→incoming_tx,
    /// one bridging outgoing_rx→send. Returns (incoming_rx, outgoing_tx).
    pub fn into_rpc_channels(self) -> (mpsc::Receiver<Vec<u8>>, mpsc::Sender<Vec<u8>>) {
        let (incoming_tx, incoming_rx) = mpsc::channel::<Vec<u8>>(64);
        let (outgoing_tx, mut outgoing_rx) = mpsc::channel::<Vec<u8>>(64);

        let IrohTransport { send, recv } = self;

        // Reader task: recv from QUIC stream → incoming channel
        tokio::spawn(async move {
            let mut recv = recv;
            loop {
                let mut len_buf = [0u8; 4];
                if recv.read_exact(&mut len_buf).await.is_err() {
                    break;
                }
                let len = u32::from_be_bytes(len_buf) as usize;
                if len > 16 * 1024 * 1024 {
                    tracing::error!("iroh recv: message too large ({} bytes)", len);
                    break;
                }
                let mut buf = vec![0u8; len];
                if recv.read_exact(&mut buf).await.is_err() {
                    break;
                }
                if incoming_tx.send(buf).await.is_err() {
                    break;
                }
            }
        });

        // Writer task: outgoing channel → send to QUIC stream
        tokio::spawn(async move {
            let mut send = send;
            while let Some(payload) = outgoing_rx.recv().await {
                let len = (payload.len() as u32).to_be_bytes();
                if send.write_all(&len).await.is_err() {
                    break;
                }
                if send.write_all(&payload).await.is_err() {
                    break;
                }
            }
        });

        (incoming_rx, outgoing_tx)
    }
}
