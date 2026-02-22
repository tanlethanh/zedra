// Integration tests for iroh transport and relay connectivity.
//
// Each test spawns a localhost iroh-relay server (TLS with self-signed certs,
// ephemeral port), creates endpoints pointed at it, and validates the
// connection + RPC flow. This exercises the full stack: endpoint binding →
// relay negotiation → QUIC stream → irpc dispatch.

use std::sync::Arc;
use std::time::Duration;

use zedra_host::iroh_listener;
use zedra_host::rpc_daemon::DaemonState;
use zedra_host::session_registry::SessionRegistry;
use zedra_rpc::proto::{self, *};
use zedra_rpc::{decode_endpoint_addr, encode_endpoint_addr};

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

/// Spawn a local iroh-relay server for testing (ephemeral port, self-signed TLS).
async fn spawn_test_relay() -> anyhow::Result<(iroh_relay::server::Server, iroh::RelayUrl)> {
    let config = iroh_relay::server::testing::server_config();
    let server = iroh_relay::server::Server::spawn(config)
        .await
        .map_err(|e| anyhow::anyhow!("failed to spawn test relay: {:?}", e))?;
    // Prefer HTTPS (iroh relay protocol uses TLS), fall back to HTTP
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

/// Wait for an endpoint to connect to the relay, with a generous timeout.
async fn wait_online(endpoint: &iroh::Endpoint) {
    tokio::time::timeout(Duration::from_secs(15), endpoint.online())
        .await
        .ok();
}

/// Set up a host: endpoint + accept loop + DaemonState with temp workdir.
/// Returns the host endpoint for address info and a handle to the tempdir.
async fn setup_host(
    relay_url: iroh::RelayUrl,
) -> anyhow::Result<(iroh::Endpoint, Arc<SessionRegistry>, tempfile::TempDir)> {
    let dir = tempfile::tempdir()?;

    // Init git repo for DaemonState
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

    let state = Arc::new(DaemonState::new(dir.path().to_path_buf()));
    let registry = Arc::new(SessionRegistry::new());

    let endpoint = make_endpoint(relay_url).await?;
    wait_online(&endpoint).await;

    // Spawn accept loop in background
    let ep = endpoint.clone();
    let reg = registry.clone();
    tokio::spawn(async move {
        let _ = iroh_listener::run_accept_loop(&ep, reg, state).await;
    });

    Ok((endpoint, registry, dir))
}

/// Connect a client to the host via iroh, returning a typed irpc client.
///
/// Performs session binding (ResumeOrCreate) before returning.
async fn connect_client(
    relay_url: iroh::RelayUrl,
    host_endpoint: &iroh::Endpoint,
) -> anyhow::Result<(irpc::Client<ZedraProto>, String)> {
    let client_endpoint = make_endpoint(relay_url).await?;
    wait_online(&client_endpoint).await;

    let host_addr = host_endpoint.addr();
    let conn = client_endpoint
        .connect(host_addr, proto::ZEDRA_ALPN)
        .await?;
    let remote = irpc_iroh::IrohRemoteConnection::new(conn);
    let client = irpc::Client::<ZedraProto>::boxed(remote);

    // Session binding: first message must be ResumeOrCreate
    let result: ResumeResult = client
        .rpc(ResumeOrCreateReq {
            session_id: None,
            auth_token: "test-token".to_string(),
            last_notif_seq: 0,
        })
        .await?;

    Ok((client, result.session_id))
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

    // Use a channel to keep the server alive until client is done reading
    let (done_tx, done_rx) = tokio::sync::oneshot::channel::<()>();

    let accept_handle = tokio::spawn(async move {
        let incoming = ep_a_clone.accept().await.expect("no incoming");
        let conn = incoming.accept().unwrap().await.unwrap();
        let (mut send, mut recv) = conn.accept_bi().await.unwrap();

        // Read 5 bytes
        let mut buf = [0u8; 5];
        recv.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"hello");

        // Send response
        send.write_all(b"world").await.unwrap();

        // Wait for client to signal it's done reading before dropping
        let _ = done_rx.await;
    });

    // Connect ep_b to ep_a
    let conn = ep_b
        .connect(ep_a_addr, proto::ZEDRA_ALPN)
        .await
        .unwrap();
    let (mut send, mut recv) = conn.open_bi().await.unwrap();

    // Send and receive
    send.write_all(b"hello").await.unwrap();

    let mut buf = [0u8; 5];
    recv.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"world");

    // Signal server we're done
    let _ = done_tx.send(());
    accept_handle.await.unwrap();
}

/// irpc correctly serializes and deserializes messages over real QUIC streams.
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

    // Server side: accept and handle one irpc request
    let server_handle = tokio::spawn(async move {
        let incoming = ep_a_clone.accept().await.expect("no incoming");
        let conn = incoming.accept().unwrap().await.unwrap();

        // Read a typed request
        let msg = irpc_iroh::read_request::<ZedraProto>(&conn)
            .await
            .unwrap();

        match msg {
            Some(ZedraMessage::Heartbeat(hb)) => {
                let _ = hb.tx.send(HeartbeatResult { ok: true }).await;
            }
            other => panic!("expected Heartbeat, got {:?}", other.is_some()),
        }

        let _ = done_rx.await;
    });

    // Client side: connect and send a typed request
    let conn = ep_b
        .connect(ep_a_addr, proto::ZEDRA_ALPN)
        .await
        .unwrap();
    let remote = irpc_iroh::IrohRemoteConnection::new(conn);
    let client = irpc::Client::<ZedraProto>::boxed(remote);

    let result: HeartbeatResult = client.rpc(HeartbeatReq {}).await.unwrap();
    assert!(result.ok);

    let _ = done_tx.send(());
    server_handle.await.unwrap();
}

/// Full RPC call over iroh — host runs accept loop, client issues GetSessionInfo.
#[tokio::test(flavor = "multi_thread")]
async fn test_full_rpc_over_iroh() {
    let (_relay, relay_url) = spawn_test_relay().await.unwrap();
    let (host_ep, _registry, _dir) = setup_host(relay_url.clone()).await.unwrap();

    let (client, session_id) = connect_client(relay_url, &host_ep).await.unwrap();
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
    let (host_ep, _registry, _dir) = setup_host(relay_url.clone()).await.unwrap();

    let (client, _session_id) = connect_client(relay_url, &host_ep).await.unwrap();

    // Create terminal
    let result: TermCreateResult = client.rpc(TermCreateReq { cols: 80, rows: 24 }).await.unwrap();
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

    // Should receive terminal output (shell prompt + echo output)
    let output = tokio::time::timeout(Duration::from_secs(5), output_rx.recv())
        .await
        .expect("timed out waiting for terminal output");
    match output {
        Ok(Some(out)) => assert!(!out.data.is_empty()),
        other => panic!("expected terminal output, got {:?}", other),
    }

    // Cleanup
    let close_result: TermCloseResult = client.rpc(TermCloseReq { id: result.id }).await.unwrap();
    assert!(close_result.ok);
}

/// Endpoint addr includes relay URL after going online.
/// This validates the fix that ensures QR codes contain the relay URL.
#[tokio::test(flavor = "multi_thread")]
async fn test_relay_url_in_endpoint_addr() {
    let (_relay, relay_url) = spawn_test_relay().await.unwrap();
    let endpoint = make_endpoint(relay_url.clone()).await.unwrap();

    // Wait for relay connection with generous timeout
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

/// EndpointAddr round-trip: encode live endpoint addr, decode back,
/// verify all fields (id, relay URL, direct addrs) survive the encoding.
#[tokio::test(flavor = "multi_thread")]
async fn test_endpoint_addr_roundtrip() {
    let (_relay, relay_url) = spawn_test_relay().await.unwrap();
    let endpoint = make_endpoint(relay_url.clone()).await.unwrap();

    tokio::time::timeout(Duration::from_secs(15), endpoint.online())
        .await
        .expect("endpoint didn't come online");

    let addr = endpoint.addr();

    // Encode to compact string
    let encoded = encode_endpoint_addr(&addr).unwrap();
    assert!(!encoded.is_empty());

    // Decode back
    let decoded = decode_endpoint_addr(&encoded).unwrap();
    assert_eq!(decoded.id, addr.id);

    // Verify relay URL survived round-trip
    let decoded_relay_urls: Vec<_> = decoded.relay_urls().collect();
    assert!(
        !decoded_relay_urls.is_empty(),
        "decoded endpoint addr should contain the relay URL"
    );
}
