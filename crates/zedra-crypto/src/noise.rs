use anyhow::Result;
use snow::{Builder, HandshakeState, TransportState};

use crate::connection_id::ConnectionId;

const NOISE_PATTERN: &str = "Noise_IK_25519_ChaChaPoly_BLAKE2s";

/// Result of a successful handshake.
pub struct HandshakeResult {
    /// The transport state for encrypting/decrypting messages.
    pub transport: TransportState,
    /// The agreed-upon Connection ID.
    pub connection_id: ConnectionId,
    /// The remote peer's static public key (32 bytes).
    pub remote_static_key: [u8; 32],
}

/// Initiator side of Noise_IK handshake (client).
///
/// In Noise_IK, the initiator knows the responder's static public key ahead of time
/// (e.g., from QR code or trust store). The handshake is 1-RTT and provides mutual
/// authentication plus forward secrecy.
pub struct NoiseInitiator {
    state: HandshakeState,
    connection_id: ConnectionId,
}

impl NoiseInitiator {
    /// Create a new initiator.
    /// - `local_static_secret`: our 32-byte Curve25519 secret key
    /// - `remote_static_public`: responder's 32-byte public key (known from QR/trust)
    pub fn new(local_static_secret: &[u8; 32], remote_static_public: &[u8; 32]) -> Result<Self> {
        let builder = Builder::new(NOISE_PATTERN.parse()?)
            .local_private_key(local_static_secret)
            .remote_public_key(remote_static_public);

        let state = builder.build_initiator()?;
        let connection_id = ConnectionId::generate();

        Ok(Self {
            state,
            connection_id,
        })
    }

    /// Perform the handshake over a Transport.
    ///
    /// Protocol:
    /// 1. Send: connection_id(32) + noise_message_1(variable) with payload
    /// 2. Recv: noise_message_2(variable) with payload
    ///
    /// `payload` is the initiator's handshake payload (e.g., JSON with device_id, otp).
    /// Returns HandshakeResult + responder's payload.
    pub async fn handshake(
        mut self,
        transport: &mut dyn zedra_rpc::Transport,
        payload: &[u8],
    ) -> Result<(HandshakeResult, Vec<u8>)> {
        // Step 1: Write message 1 (-> e, es, s, ss + payload)
        let mut msg1_buf = vec![0u8; 65535];
        let msg1_len = self.state.write_message(payload, &mut msg1_buf)?;
        msg1_buf.truncate(msg1_len);

        // Send: connection_id + msg1
        let mut frame = Vec::with_capacity(32 + msg1_len);
        frame.extend_from_slice(self.connection_id.as_bytes());
        frame.extend_from_slice(&msg1_buf);
        transport.send(&frame).await?;

        // Step 2: Read message 2 (<- e, ee, se + payload)
        let frame2 = transport.recv().await?;
        let mut resp_payload = vec![0u8; 65535];
        let resp_len = self.state.read_message(&frame2, &mut resp_payload)?;
        resp_payload.truncate(resp_len);

        // Get remote static key
        let remote_static = self
            .state
            .get_remote_static()
            .ok_or_else(|| anyhow::anyhow!("no remote static key after handshake"))?;
        let mut remote_key = [0u8; 32];
        remote_key.copy_from_slice(remote_static);

        // Convert to transport mode
        let transport_state = self.state.into_transport_mode()?;

        Ok((
            HandshakeResult {
                transport: transport_state,
                connection_id: self.connection_id,
                remote_static_key: remote_key,
            },
            resp_payload,
        ))
    }
}

/// Responder side of Noise_IK handshake (host).
pub struct NoiseResponder {
    state: HandshakeState,
}

impl NoiseResponder {
    /// Create a new responder.
    /// - `local_static_secret`: our 32-byte Curve25519 secret key
    pub fn new(local_static_secret: &[u8; 32]) -> Result<Self> {
        let builder =
            Builder::new(NOISE_PATTERN.parse()?).local_private_key(local_static_secret);

        let state = builder.build_responder()?;

        Ok(Self { state })
    }

    /// Perform the handshake over a Transport.
    ///
    /// Protocol:
    /// 1. Recv: connection_id(32) + noise_message_1(variable) with initiator payload
    /// 2. Send: noise_message_2(variable) with our payload
    ///
    /// `payload` is the responder's handshake payload (e.g., JSON with device_id, ok).
    /// Returns HandshakeResult + initiator's payload.
    pub async fn handshake(
        mut self,
        transport: &mut dyn zedra_rpc::Transport,
        payload: &[u8],
    ) -> Result<(HandshakeResult, Vec<u8>)> {
        // Step 1: Read message 1 (-> e, es, s, ss + payload)
        let frame1 = transport.recv().await?;
        if frame1.len() < 32 {
            anyhow::bail!("handshake frame too short: {} bytes", frame1.len());
        }

        // Extract connection_id
        let mut conn_id_bytes = [0u8; 32];
        conn_id_bytes.copy_from_slice(&frame1[..32]);
        let connection_id = ConnectionId::from_bytes(conn_id_bytes);

        // Read noise message
        let mut init_payload = vec![0u8; 65535];
        let init_len = self.state.read_message(&frame1[32..], &mut init_payload)?;
        init_payload.truncate(init_len);

        // Step 2: Write message 2 (<- e, ee, se + payload)
        let mut msg2_buf = vec![0u8; 65535];
        let msg2_len = self.state.write_message(payload, &mut msg2_buf)?;
        msg2_buf.truncate(msg2_len);
        transport.send(&msg2_buf).await?;

        // Get remote static key
        let remote_static = self
            .state
            .get_remote_static()
            .ok_or_else(|| anyhow::anyhow!("no remote static key after handshake"))?;
        let mut remote_key = [0u8; 32];
        remote_key.copy_from_slice(remote_static);

        // Convert to transport mode
        let transport_state = self.state.into_transport_mode()?;

        Ok((
            HandshakeResult {
                transport: transport_state,
                connection_id,
                remote_static_key: remote_key,
            },
            init_payload,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use async_trait::async_trait;
    use tokio::sync::mpsc;

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
        let (tx_a, rx_b) = mpsc::channel(16);
        let (tx_b, rx_a) = mpsc::channel(16);
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
    fn generate_keypair() -> (Vec<u8>, Vec<u8>) {
        let builder = Builder::new(NOISE_PATTERN.parse().unwrap());
        let keypair = builder.generate_keypair().unwrap();
        (keypair.private, keypair.public)
    }

    #[tokio::test]
    async fn handshake_success() {
        let (client_secret, client_public) = generate_keypair();
        let (server_secret, server_public) = generate_keypair();

        let mut client_secret_arr = [0u8; 32];
        client_secret_arr.copy_from_slice(&client_secret);

        let mut client_public_arr = [0u8; 32];
        client_public_arr.copy_from_slice(&client_public);

        let mut server_secret_arr = [0u8; 32];
        server_secret_arr.copy_from_slice(&server_secret);

        let mut server_public_arr = [0u8; 32];
        server_public_arr.copy_from_slice(&server_public);

        let initiator =
            NoiseInitiator::new(&client_secret_arr, &server_public_arr).unwrap();
        let responder = NoiseResponder::new(&server_secret_arr).unwrap();

        let (mut client_transport, mut server_transport) = mock_transport_pair();

        let client_payload = b"hello from client";
        let server_payload = b"hello from server";

        let (client_result, server_result) = tokio::join!(
            initiator.handshake(&mut client_transport, client_payload),
            responder.handshake(&mut server_transport, server_payload),
        );

        let (client_hs, server_resp_payload) = client_result.unwrap();
        let (server_hs, client_init_payload) = server_result.unwrap();

        // Verify payloads were exchanged correctly
        assert_eq!(client_init_payload, b"hello from client");
        assert_eq!(server_resp_payload, b"hello from server");

        // Verify both sides agree on the connection ID
        assert_eq!(client_hs.connection_id, server_hs.connection_id);

        // Verify remote static keys: each side sees the other's public key
        assert_eq!(client_hs.remote_static_key, server_public_arr);
        assert_eq!(server_hs.remote_static_key, client_public_arr);
    }

    #[tokio::test]
    async fn handshake_with_empty_payloads() {
        let (client_secret, _client_public) = generate_keypair();
        let (server_secret, server_public) = generate_keypair();

        let mut client_secret_arr = [0u8; 32];
        client_secret_arr.copy_from_slice(&client_secret);

        let mut server_secret_arr = [0u8; 32];
        server_secret_arr.copy_from_slice(&server_secret);

        let mut server_public_arr = [0u8; 32];
        server_public_arr.copy_from_slice(&server_public);

        let initiator =
            NoiseInitiator::new(&client_secret_arr, &server_public_arr).unwrap();
        let responder = NoiseResponder::new(&server_secret_arr).unwrap();

        let (mut client_transport, mut server_transport) = mock_transport_pair();

        let (client_result, server_result) = tokio::join!(
            initiator.handshake(&mut client_transport, b""),
            responder.handshake(&mut server_transport, b""),
        );

        let (client_hs, server_resp_payload) = client_result.unwrap();
        let (server_hs, client_init_payload) = server_result.unwrap();

        assert_eq!(client_init_payload, b"");
        assert_eq!(server_resp_payload, b"");
        assert_eq!(client_hs.connection_id, server_hs.connection_id);
    }
}
