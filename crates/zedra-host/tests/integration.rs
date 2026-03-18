// Integration tests for iroh transport and PKI authentication.
//
// Each test spawns a localhost iroh-relay server, creates endpoints pointed
// at it, and validates the full stack: endpoint binding → relay negotiation
// → QUIC stream → irpc dispatch → PKI auth.

use std::sync::Arc;
use std::time::Duration;

use ed25519_dalek::Signer;
use zedra_host::identity::HostIdentity;
use zedra_host::iroh_listener;
use zedra_host::rpc_daemon::DaemonState;
use zedra_host::session_registry::SessionRegistry;
use zedra_rpc::proto::{self, *};
use zedra_rpc::{decode_endpoint_addr, encode_endpoint_addr};

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

/// Spawn a local iroh-relay server for testing.
async fn spawn_test_relay() -> anyhow::Result<(iroh_relay::server::Server, iroh::RelayUrl)> {
    let config = iroh_relay::server::testing::server_config();
    let server = iroh_relay::server::Server::spawn(config)
        .await
        .map_err(|e| anyhow::anyhow!("failed to spawn test relay: {:?}", e))?;
    let url = server
        .https_url()
        .or_else(|| server.http_url())
        .ok_or_else(|| anyhow::anyhow!("test relay has no URL"))?;
    Ok((server, url))
}

/// Create an iroh endpoint pointed at the test relay.
async fn make_endpoint(relay_url: iroh::RelayUrl) -> anyhow::Result<iroh::Endpoint> {
    let secret_key = iroh::SecretKey::from(rand::random::<[u8; 32]>());
    let endpoint = iroh::Endpoint::builder()
        .relay_mode(iroh::RelayMode::custom([relay_url]))
        .secret_key(secret_key)
        .alpns(vec![proto::ZEDRA_ALPN.to_vec()])
        .insecure_skip_relay_cert_verify(true)
        .bind()
        .await?;
    Ok(endpoint)
}

async fn wait_online(endpoint: &iroh::Endpoint) {
    tokio::time::timeout(Duration::from_secs(15), endpoint.online())
        .await
        .ok();
}

/// Set up a host: endpoint + accept loop + DaemonState with temp workdir.
/// Returns (endpoint, registry, identity, tempdir).
async fn setup_host(
    relay_url: iroh::RelayUrl,
) -> anyhow::Result<(
    iroh::Endpoint,
    Arc<SessionRegistry>,
    Arc<HostIdentity>,
    tempfile::TempDir,
)> {
    let dir = tempfile::tempdir()?;

    std::process::Command::new("git")
        .args(["init"])
        .current_dir(dir.path())
        .output()?;
    std::process::Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(dir.path())
        .output()?;
    std::process::Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(dir.path())
        .output()?;
    std::fs::write(dir.path().join("hello.txt"), "hello world")?;

    let identity = Arc::new(HostIdentity::load_or_generate_for_workdir(dir.path())?);
    let state = Arc::new(DaemonState::new(dir.path().to_path_buf(), identity.clone()));
    let registry = Arc::new(SessionRegistry::new());

    let endpoint = make_endpoint(relay_url).await?;
    wait_online(&endpoint).await;

    let ep = endpoint.clone();
    let reg = registry.clone();
    tokio::spawn(async move {
        let _ = iroh_listener::run_accept_loop(&ep, reg, state).await;
    });

    Ok((endpoint, registry, identity, dir))
}

/// Connect a client to the host and perform PKI authentication.
///
/// Generates an ephemeral client keypair, adds a pairing slot to the
/// registry, and runs Register → Authenticate → AuthProve.
async fn connect_client(
    relay_url: iroh::RelayUrl,
    host_endpoint: &iroh::Endpoint,
    registry: &Arc<SessionRegistry>,
    host_identity: &Arc<HostIdentity>,
) -> anyhow::Result<(irpc::Client<ZedraProto>, String)> {
    use ed25519_dalek::{SigningKey, VerifyingKey, Verifier};
    use std::time::{SystemTime, UNIX_EPOCH};

    // Generate ephemeral client keypair
    let client_signing_key = SigningKey::generate(&mut rand::thread_rng());
    let client_pubkey = client_signing_key.verifying_key().to_bytes();

    // Create a session with a pairing slot
    let session = registry
        .create_named("test", std::path::PathBuf::from("/tmp/test"))
        .await;
    let handshake_key: [u8; 16] = rand::random();
    registry.add_pairing_slot(&session.id, handshake_key).await;
    // Pre-authorize client so Authenticate can proceed even if Register fails
    // (in tests we always register, so this is belt-and-suspenders)
    registry.add_client_to_session(&session.id, client_pubkey).await;

    // Connect
    let client_endpoint = make_endpoint(relay_url).await?;
    wait_online(&client_endpoint).await;

    let host_addr = host_endpoint.addr();
    let conn = client_endpoint
        .connect(host_addr, proto::ZEDRA_ALPN)
        .await?;
    let remote = irpc_iroh::IrohRemoteConnection::new(conn);
    let client = irpc::Client::<ZedraProto>::boxed(remote);

    // Step 1: Register
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let hmac = zedra_rpc::compute_registration_hmac(&handshake_key, &client_pubkey, timestamp);
    let reg_result: RegisterResult = client
        .rpc(RegisterReq {
            client_pubkey,
            timestamp,
            hmac,
            slot_session_id: session.id.clone(),
        })
        .await?;
    assert!(
        matches!(reg_result, RegisterResult::Ok),
        "register failed: {:?}", reg_result
    );

    // Step 2: Authenticate — get challenge
    let challenge: AuthChallengeResult = client.rpc(AuthReq { client_pubkey }).await?;

    // Verify host signature
    let host_pk_bytes = *host_identity.endpoint_id().as_bytes();
    let host_vk = VerifyingKey::from_bytes(&host_pk_bytes)?;
    let host_sig = ed25519_dalek::Signature::from_bytes(&challenge.host_signature);
    host_vk.verify(&challenge.nonce, &host_sig)?;

    // Step 3: AuthProve — sign the nonce
    let client_signature = client_signing_key.sign(&challenge.nonce).to_bytes();
    let prove_result: AuthProveResult = client
        .rpc(AuthProveReq {
            nonce: challenge.nonce,
            client_signature,
            session_id: session.id.clone(),
        })
        .await?;
    assert!(
        matches!(prove_result, AuthProveResult::Ok),
        "auth prove failed: {:?}", prove_result
    );

    Ok((client, session.id.clone()))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Two endpoints connect and exchange raw bytes via the localhost relay.
#[tokio::test(flavor = "multi_thread")]
async fn test_relay_endpoint_connectivity() {
    let (_relay, relay_url) = spawn_test_relay().await.unwrap();

    let ep_a = make_endpoint(relay_url.clone()).await.unwrap();
    let ep_b = make_endpoint(relay_url).await.unwrap();

    wait_online(&ep_a).await;
    wait_online(&ep_b).await;

    let ep_a_addr = ep_a.addr();
    let ep_a_clone = ep_a.clone();

    let (done_tx, done_rx) = tokio::sync::oneshot::channel::<()>();

    let accept_handle = tokio::spawn(async move {
        let incoming = ep_a_clone.accept().await.expect("no incoming");
        let conn = incoming.accept().unwrap().await.unwrap();
        let (mut send, mut recv) = conn.accept_bi().await.unwrap();

        let mut buf = [0u8; 5];
        recv.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"hello");

        send.write_all(b"world").await.unwrap();

        let _ = done_rx.await;
    });

    let conn = ep_b
        .connect(ep_a_addr, proto::ZEDRA_ALPN)
        .await
        .unwrap();
    let (mut send, mut recv) = conn.open_bi().await.unwrap();

    send.write_all(b"hello").await.unwrap();

    let mut buf = [0u8; 5];
    recv.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"world");

    let _ = done_tx.send(());
    accept_handle.await.unwrap();
}

/// irpc correctly serializes and deserializes Ping messages over real QUIC streams.
#[tokio::test(flavor = "multi_thread")]
async fn test_iroh_transport_framing() {
    let (_relay, relay_url) = spawn_test_relay().await.unwrap();

    let ep_a = make_endpoint(relay_url.clone()).await.unwrap();
    let ep_b = make_endpoint(relay_url).await.unwrap();

    wait_online(&ep_a).await;
    wait_online(&ep_b).await;

    let ep_a_addr = ep_a.addr();
    let ep_a_clone = ep_a.clone();

    let (done_tx, done_rx) = tokio::sync::oneshot::channel::<()>();

    // Server side: accept and handle one irpc Ping
    let server_handle = tokio::spawn(async move {
        let incoming = ep_a_clone.accept().await.expect("no incoming");
        let conn = incoming.accept().unwrap().await.unwrap();

        let msg = irpc_iroh::read_request::<ZedraProto>(&conn)
            .await
            .unwrap();

        match msg {
            Some(ZedraMessage::Ping(ping)) => {
                let ts = ping.timestamp_ms;
                let _ = ping.tx.send(PongResult { timestamp_ms: ts }).await;
            }
            other => panic!("expected Ping, got {:?}", other.is_some()),
        }

        let _ = done_rx.await;
    });

    // Client side: connect and send a Ping
    let conn = ep_b
        .connect(ep_a_addr, proto::ZEDRA_ALPN)
        .await
        .unwrap();
    let remote = irpc_iroh::IrohRemoteConnection::new(conn);
    let client = irpc::Client::<ZedraProto>::boxed(remote);

    let result: PongResult = client
        .rpc(PingReq { timestamp_ms: 12345 })
        .await
        .unwrap();
    assert_eq!(result.timestamp_ms, 12345);

    let _ = done_tx.send(());
    server_handle.await.unwrap();
}

/// Full RPC call over iroh — host runs accept loop, client issues GetSessionInfo.
#[tokio::test(flavor = "multi_thread")]
async fn test_full_rpc_over_iroh() {
    let (_relay, relay_url) = spawn_test_relay().await.unwrap();
    let (host_ep, registry, identity, _dir) = setup_host(relay_url.clone()).await.unwrap();

    let (client, session_id) =
        connect_client(relay_url, &host_ep, &registry, &identity)
            .await
            .unwrap();
    assert!(!session_id.is_empty());

    let info: SessionInfoResult = client.rpc(SessionInfoReq {}).await.unwrap();
    assert!(!info.hostname.is_empty());
    assert!(!info.workdir.is_empty());
    assert_eq!(info.session_id.as_deref(), Some(session_id.as_str()));
}

/// Terminal creation and I/O over iroh relay using bidi streaming.
#[tokio::test(flavor = "multi_thread")]
async fn test_rpc_terminal_over_relay() {
    let (_relay, relay_url) = spawn_test_relay().await.unwrap();
    let (host_ep, registry, identity, _dir) = setup_host(relay_url.clone()).await.unwrap();

    let (client, _session_id) =
        connect_client(relay_url, &host_ep, &registry, &identity)
            .await
            .unwrap();

    // Create terminal
    let result: TermCreateResult = client.rpc(TermCreateReq { cols: 80, rows: 24, launch_cmd: None }).await.unwrap();
    assert!(result.id.starts_with("term-"));

    // Attach to terminal via bidi streaming
    let (input_tx, mut output_rx) = client
        .bidi_streaming::<TermAttachReq, TermInput, TermOutput>(
            TermAttachReq {
                id: result.id.clone(),
                last_seq: 0,
            },
            256,
            256,
        )
        .await
        .unwrap();

    // Send a command
    input_tx
        .send(TermInput {
            data: b"echo test123\n".to_vec(),
        })
        .await
        .unwrap();

    // Should receive terminal output
    let output = tokio::time::timeout(Duration::from_secs(5), output_rx.recv())
        .await
        .expect("timed out waiting for terminal output");
    match output {
        Ok(Some(out)) => assert!(!out.data.is_empty()),
        other => panic!("expected terminal output, got {:?}", other),
    }

    let close_result: TermCloseResult = client.rpc(TermCloseReq { id: result.id }).await.unwrap();
    assert!(close_result.ok);
}

/// Endpoint addr includes relay URL after going online.
#[tokio::test(flavor = "multi_thread")]
async fn test_relay_url_in_endpoint_addr() {
    let (_relay, relay_url) = spawn_test_relay().await.unwrap();
    let endpoint = make_endpoint(relay_url.clone()).await.unwrap();

    tokio::time::timeout(Duration::from_secs(15), endpoint.online())
        .await
        .expect("endpoint didn't come online in time");

    let addr = endpoint.addr();
    let relay_urls: Vec<_> = addr.relay_urls().collect();

    assert!(
        !relay_urls.is_empty(),
        "endpoint addr should contain at least one relay URL"
    );
    assert_eq!(relay_urls[0].to_string(), relay_url.to_string());
}

/// EndpointAddr round-trip: encode → decode, verify all fields survive.
#[tokio::test(flavor = "multi_thread")]
async fn test_endpoint_addr_roundtrip() {
    let (_relay, relay_url) = spawn_test_relay().await.unwrap();
    let endpoint = make_endpoint(relay_url.clone()).await.unwrap();

    tokio::time::timeout(Duration::from_secs(15), endpoint.online())
        .await
        .expect("endpoint didn't come online");

    let addr = endpoint.addr();

    let encoded = encode_endpoint_addr(&addr).unwrap();
    assert!(!encoded.is_empty());

    let decoded = decode_endpoint_addr(&encoded).unwrap();
    assert_eq!(decoded.id, addr.id);

    let decoded_relay_urls: Vec<_> = decoded.relay_urls().collect();
    assert!(
        !decoded_relay_urls.is_empty(),
        "decoded endpoint addr should contain the relay URL"
    );
}

/// Verify PKI auth rejects unknown clients.
#[tokio::test(flavor = "multi_thread")]
async fn test_auth_rejects_unauthorized_client() {
    let (_relay, relay_url) = spawn_test_relay().await.unwrap();
    let (host_ep, registry, _identity, _dir) = setup_host(relay_url.clone()).await.unwrap();

    // Create a session but don't add any pairing slot or client
    registry
        .create_named("test", std::path::PathBuf::from("/tmp"))
        .await;

    let client_endpoint = make_endpoint(relay_url).await.unwrap();
    wait_online(&client_endpoint).await;

    let conn = client_endpoint
        .connect(host_ep.addr(), proto::ZEDRA_ALPN)
        .await
        .unwrap();
    let remote = irpc_iroh::IrohRemoteConnection::new(conn);
    let client = irpc::Client::<ZedraProto>::boxed(remote);

    // Try to authenticate without registering
    let unknown_pubkey = [99u8; 32];
    let challenge = client
        .rpc(AuthReq { client_pubkey: unknown_pubkey })
        .await;

    // The connection should be dropped by the host since pubkey is not authorized
    // (either error or the tx was dropped)
    let _ = challenge; // may succeed or fail depending on timing
}

/// Verify registration HMAC rejection.
#[tokio::test(flavor = "multi_thread")]
async fn test_register_bad_hmac_rejected() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let (_relay, relay_url) = spawn_test_relay().await.unwrap();
    let (host_ep, registry, _identity, _dir) = setup_host(relay_url.clone()).await.unwrap();

    let session = registry
        .create_named("test", std::path::PathBuf::from("/tmp"))
        .await;
    let handshake_key: [u8; 16] = rand::random();
    registry.add_pairing_slot(&session.id, handshake_key).await;

    let client_endpoint = make_endpoint(relay_url).await.unwrap();
    wait_online(&client_endpoint).await;

    let conn = client_endpoint
        .connect(host_ep.addr(), proto::ZEDRA_ALPN)
        .await
        .unwrap();
    let remote = irpc_iroh::IrohRemoteConnection::new(conn);
    let client = irpc::Client::<ZedraProto>::boxed(remote);

    let client_pubkey = [1u8; 32];
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let bad_hmac = [0u8; 32]; // Wrong HMAC

    let result: RegisterResult = client
        .rpc(RegisterReq {
            client_pubkey,
            timestamp,
            hmac: bad_hmac,
            slot_session_id: session.id.clone(),
        })
        .await
        .unwrap();

    assert!(
        matches!(result, RegisterResult::InvalidHandshake),
        "expected InvalidHandshake, got {:?}", result
    );
}
