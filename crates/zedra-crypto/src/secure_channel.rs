use anyhow::Result;
use async_trait::async_trait;
use snow::TransportState;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::connection_id::ConnectionId;

/// Maximum frame payload size (16 MB, matching existing Transport limit).
const MAX_PAYLOAD_SIZE: usize = 16 * 1024 * 1024;

/// AEAD overhead: 16 bytes for Poly1305 tag.
const AEAD_TAG_SIZE: usize = 16;

/// Encrypted transport wrapper.
///
/// Wraps an inner `dyn Transport` with Noise-derived ChaCha20-Poly1305 encryption.
/// Each frame sent/received is encrypted/decrypted transparently.
///
/// Frame format on the wire (sent via inner transport's send/recv):
///   [connection_id: 32 bytes][encrypted_payload + tag: variable]
///
/// The inner transport handles length-framing, so we don't add our own length prefix.
pub struct SecureTransport {
    inner: Box<dyn zedra_rpc::Transport>,
    transport_state: TransportState,
    connection_id: ConnectionId,
    send_counter: AtomicU64,
}

impl SecureTransport {
    pub fn new(
        inner: Box<dyn zedra_rpc::Transport>,
        transport_state: TransportState,
        connection_id: ConnectionId,
    ) -> Self {
        Self {
            inner,
            transport_state,
            connection_id,
            send_counter: AtomicU64::new(0),
        }
    }

    /// Get the connection ID.
    pub fn connection_id(&self) -> &ConnectionId {
        &self.connection_id
    }

    /// Get the number of messages sent so far.
    pub fn send_count(&self) -> u64 {
        self.send_counter.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl zedra_rpc::Transport for SecureTransport {
    async fn send(&mut self, payload: &[u8]) -> Result<()> {
        if payload.len() > MAX_PAYLOAD_SIZE {
            anyhow::bail!("payload too large: {} bytes", payload.len());
        }

        // Encrypt the payload
        let mut ciphertext = vec![0u8; payload.len() + AEAD_TAG_SIZE];
        let len = self
            .transport_state
            .write_message(payload, &mut ciphertext)?;
        ciphertext.truncate(len);

        // Build frame: connection_id + ciphertext
        let mut frame = Vec::with_capacity(32 + ciphertext.len());
        frame.extend_from_slice(self.connection_id.as_bytes());
        frame.extend_from_slice(&ciphertext);

        self.inner.send(&frame).await?;
        self.send_counter.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    async fn recv(&mut self) -> Result<Vec<u8>> {
        let frame = self.inner.recv().await?;
        if frame.len() < 32 + AEAD_TAG_SIZE {
            anyhow::bail!("encrypted frame too short: {} bytes", frame.len());
        }

        // Verify connection ID
        let received_conn_id = &frame[..32];
        if received_conn_id != self.connection_id.as_bytes() {
            anyhow::bail!("connection ID mismatch");
        }

        // Decrypt
        let ciphertext = &frame[32..];
        let mut plaintext = vec![0u8; ciphertext.len()];
        let len = self
            .transport_state
            .read_message(ciphertext, &mut plaintext)?;
        plaintext.truncate(len);

        Ok(plaintext)
    }

    fn name(&self) -> &str {
        "secure"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noise::{NoiseInitiator, NoiseResponder};
    use anyhow::Result;
    use async_trait::async_trait;
    use snow::Builder;
    use tokio::sync::mpsc;
    use zedra_rpc::Transport;

    const NOISE_PATTERN: &str = "Noise_IK_25519_ChaChaPoly_BLAKE2s";

    /// A simple in-memory transport for testing, backed by mpsc channels.
    struct MockTransport {
        incoming: mpsc::Receiver<Vec<u8>>,
        outgoing: mpsc::Sender<Vec<u8>>,
    }

    #[async_trait]
    impl zedra_rpc::Transport for MockTransport {
        async fn send(&mut self, payload: &[u8]) -> Result<()> {
            self.outgoing
                .send(payload.to_vec())
                .await
                .map_err(|_| anyhow::anyhow!("mock send failed"))?;
            Ok(())
        }

        async fn recv(&mut self) -> Result<Vec<u8>> {
            self.incoming
                .recv()
                .await
                .ok_or_else(|| anyhow::anyhow!("mock recv failed"))
        }

        fn name(&self) -> &str {
            "mock"
        }
    }

    /// Create a pair of connected MockTransports.
    fn mock_transport_pair() -> (MockTransport, MockTransport) {
        let (tx_a, rx_b) = mpsc::channel(64);
        let (tx_b, rx_a) = mpsc::channel(64);
        (
            MockTransport {
                incoming: rx_a,
                outgoing: tx_a,
            },
            MockTransport {
                incoming: rx_b,
                outgoing: tx_b,
            },
        )
    }

    /// Generate a Curve25519 keypair using snow's builder.
    fn generate_keypair() -> ([u8; 32], [u8; 32]) {
        let builder = Builder::new(NOISE_PATTERN.parse().unwrap());
        let keypair = builder.generate_keypair().unwrap();
        let mut secret = [0u8; 32];
        let mut public = [0u8; 32];
        secret.copy_from_slice(&keypair.private);
        public.copy_from_slice(&keypair.public);
        (secret, public)
    }

    /// Perform a Noise_IK handshake and return two SecureTransport instances
    /// connected via mock transports.
    async fn create_secure_pair() -> (SecureTransport, SecureTransport) {
        let (client_secret, _client_public) = generate_keypair();
        let (server_secret, server_public) = generate_keypair();

        // Handshake transports (used only during handshake, then discarded)
        let (mut hs_client, mut hs_server) = mock_transport_pair();

        let initiator = NoiseInitiator::new(&client_secret, &server_public).unwrap();
        let responder = NoiseResponder::new(&server_secret).unwrap();

        let (client_result, server_result) = tokio::join!(
            initiator.handshake(&mut hs_client, b""),
            responder.handshake(&mut hs_server, b""),
        );

        let (client_hs, _) = client_result.unwrap();
        let (server_hs, _) = server_result.unwrap();

        // Data transports (used for encrypted communication)
        let (data_client, data_server) = mock_transport_pair();

        let secure_client = SecureTransport::new(
            Box::new(data_client),
            client_hs.transport,
            client_hs.connection_id.clone(),
        );

        let secure_server = SecureTransport::new(
            Box::new(data_server),
            server_hs.transport,
            server_hs.connection_id,
        );

        (secure_client, secure_server)
    }

    #[tokio::test]
    async fn encrypt_decrypt_roundtrip() {
        let (mut client, mut server) = create_secure_pair().await;

        let plaintext = b"hello encrypted world";

        // Client sends, server receives
        client.send(plaintext).await.unwrap();
        let received = server.recv().await.unwrap();
        assert_eq!(received, plaintext);

        // Server sends, client receives
        let reply = b"reply from server";
        server.send(reply).await.unwrap();
        let received_reply = client.recv().await.unwrap();
        assert_eq!(received_reply, reply);
    }

    #[tokio::test]
    async fn send_multiple_messages() {
        let (mut client, mut server) = create_secure_pair().await;

        let messages: Vec<Vec<u8>> = (0..10)
            .map(|i| format!("message number {}", i).into_bytes())
            .collect();

        // Send all messages from client to server
        for msg in &messages {
            client.send(msg).await.unwrap();
        }

        // Receive all messages on server and verify order
        for expected in &messages {
            let received = server.recv().await.unwrap();
            assert_eq!(&received, expected);
        }

        assert_eq!(client.send_count(), 10);
    }

    #[tokio::test]
    async fn send_empty_payload() {
        let (mut client, mut server) = create_secure_pair().await;

        client.send(b"").await.unwrap();
        let received = server.recv().await.unwrap();
        assert_eq!(received, b"");
    }

    #[tokio::test]
    async fn send_large_payload() {
        let (mut client, mut server) = create_secure_pair().await;

        // snow's transport messages have a 65535-byte limit per message
        // Use a payload that fits within that limit
        let payload = vec![0xABu8; 60000];
        client.send(&payload).await.unwrap();
        let received = server.recv().await.unwrap();
        assert_eq!(received, payload);
    }

    #[tokio::test]
    async fn bidirectional_interleaved() {
        let (mut client, mut server) = create_secure_pair().await;

        // Interleaved sends/receives
        client.send(b"c1").await.unwrap();
        server.send(b"s1").await.unwrap();

        let from_client = server.recv().await.unwrap();
        let from_server = client.recv().await.unwrap();

        assert_eq!(from_client, b"c1");
        assert_eq!(from_server, b"s1");

        client.send(b"c2").await.unwrap();
        server.send(b"s2").await.unwrap();

        let from_client = server.recv().await.unwrap();
        let from_server = client.recv().await.unwrap();

        assert_eq!(from_client, b"c2");
        assert_eq!(from_server, b"s2");
    }
}
