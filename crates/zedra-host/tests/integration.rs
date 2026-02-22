// Integration tests for iroh transport and relay connectivity.
//
// Each test spawns a localhost iroh-relay server (TLS with self-signed certs,
// ephemeral port), creates endpoints pointed at it, and validates the
// connection + RPC flow. This exercises the full stack: endpoint binding →
// relay negotiation → QUIC stream → IrohTransport → RPC dispatch.

use std::sync::Arc;
use std::time::Duration;

use zedra_host::iroh_listener;
use zedra_host::rpc_daemon::DaemonState;
use zedra_host::session_registry::SessionRegistry;
use zedra_rpc::{methods, RpcClient, Transport};
use zedra_transport::{parse_pairing_uri, IrohTransport, PairingPayload};

const ZEDRA_ALPN: &[u8] = b"zedra/rpc/1";

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
        .alpns(vec![ZEDRA_ALPN.to_vec()])
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

/// Connect a client to the host via iroh, returning an RPC client.
async fn connect_client(
    relay_url: iroh::RelayUrl,
    host_endpoint: &iroh::Endpoint,
) -> anyhow::Result<(
    RpcClient,
    tokio::sync::mpsc::Receiver<zedra_rpc::Notification>,
)> {
    let client_endpoint = make_endpoint(relay_url).await?;
    wait_online(&client_endpoint).await;

    let host_addr = host_endpoint.addr();
    let conn = client_endpoint.connect(host_addr, ZEDRA_ALPN).await?;
    let (send, recv) = conn.open_bi().await?;

    let transport = IrohTransport::new(send, recv);
    let (incoming_rx, outgoing_tx) = transport.into_rpc_channels();
    let (client, notifs) = RpcClient::spawn_from_channels(incoming_rx, outgoing_tx);

    Ok((client, notifs))
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
    let conn = ep_b.connect(ep_a_addr, ZEDRA_ALPN).await.unwrap();
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

/// IrohTransport correctly frames messages over real QUIC streams.
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

    // Server side: accept and echo via IrohTransport
    let server_handle = tokio::spawn(async move {
        let incoming = ep_a_clone.accept().await.expect("no incoming");
        let conn = incoming.accept().unwrap().await.unwrap();
        let (send, recv) = conn.accept_bi().await.unwrap();

        let mut transport = IrohTransport::new(send, recv);

        // Receive a framed message
        let msg = transport.recv().await.unwrap();
        assert_eq!(msg, b"hello iroh transport");

        // Send a framed response
        transport.send(b"echo: hello iroh transport").await.unwrap();

        // Wait for client to finish reading
        let _ = done_rx.await;
    });

    // Client side: connect and send via IrohTransport
    let conn = ep_b.connect(ep_a_addr, ZEDRA_ALPN).await.unwrap();
    let (send, recv) = conn.open_bi().await.unwrap();

    let mut transport = IrohTransport::new(send, recv);
    transport.send(b"hello iroh transport").await.unwrap();

    let response = transport.recv().await.unwrap();
    assert_eq!(response, b"echo: hello iroh transport");

    let _ = done_tx.send(());
    server_handle.await.unwrap();
}

/// Full RPC call over iroh — host runs accept loop, client issues session/info.
#[tokio::test(flavor = "multi_thread")]
async fn test_full_rpc_over_iroh() {
    let (_relay, relay_url) = spawn_test_relay().await.unwrap();
    let (host_ep, _registry, _dir) = setup_host(relay_url.clone()).await.unwrap();

    let (client, _notifs) = connect_client(relay_url, &host_ep).await.unwrap();

    let resp = client
        .call(methods::SESSION_INFO, serde_json::json!({}))
        .await
        .unwrap();

    assert!(
        resp.error.is_none(),
        "session/info failed: {:?}",
        resp.error
    );

    let info: zedra_rpc::SessionInfoResult = serde_json::from_value(resp.result.unwrap()).unwrap();
    assert!(!info.hostname.is_empty());
    assert!(!info.workdir.is_empty());
}

/// Terminal creation and output notification over iroh relay.
#[tokio::test(flavor = "multi_thread")]
async fn test_rpc_terminal_over_relay() {
    let (_relay, relay_url) = spawn_test_relay().await.unwrap();
    let (host_ep, _registry, _dir) = setup_host(relay_url.clone()).await.unwrap();

    let (client, mut notifs) = connect_client(relay_url, &host_ep).await.unwrap();

    // Create terminal
    let resp = client
        .call(
            methods::TERM_CREATE,
            serde_json::json!({"cols": 80, "rows": 24}),
        )
        .await
        .unwrap();
    assert!(resp.error.is_none(), "term/create failed: {:?}", resp.error);

    let result: zedra_rpc::TermCreateResult = serde_json::from_value(resp.result.unwrap()).unwrap();
    assert!(result.id.starts_with("term-"));

    // Send a command
    let input = base64_url::encode(b"echo test123\n");
    let resp = client
        .call(
            methods::TERM_DATA,
            serde_json::json!({"id": result.id, "data": input}),
        )
        .await
        .unwrap();
    assert!(resp.error.is_none(), "term/data failed: {:?}", resp.error);

    // Should receive terminal/output notification
    let notif = tokio::time::timeout(Duration::from_secs(5), notifs.recv())
        .await
        .expect("timed out waiting for terminal output")
        .expect("notification channel closed");
    assert_eq!(notif.method, methods::TERM_OUTPUT);

    let output: zedra_rpc::TermOutputNotification = serde_json::from_value(notif.params).unwrap();
    assert_eq!(output.id, result.id);
    assert!(!output.data.is_empty());

    // Cleanup
    let resp = client
        .call(methods::TERM_CLOSE, serde_json::json!({"id": result.id}))
        .await
        .unwrap();
    assert!(resp.error.is_none());
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

/// PairingPayload round-trip: build from live endpoint, serialize to URI,
/// parse back, and verify all fields survive the encoding.
#[tokio::test(flavor = "multi_thread")]
async fn test_pairing_payload_roundtrip() {
    let (_relay, relay_url) = spawn_test_relay().await.unwrap();
    let endpoint = make_endpoint(relay_url.clone()).await.unwrap();

    tokio::time::timeout(Duration::from_secs(15), endpoint.online())
        .await
        .expect("endpoint didn't come online");

    let addr = endpoint.addr();
    let endpoint_id = endpoint.id().to_string();

    // Build PairingPayload from live endpoint info
    let payload = PairingPayload {
        v: 1,
        endpoint_id: endpoint_id.clone(),
        name: "test-host".to_string(),
        relay_url: addr.relay_urls().next().map(|u| u.to_string()),
        addrs: addr.ip_addrs().map(|a| a.to_string()).collect(),
    };

    // Serialize to URI
    let json = serde_json::to_string(&payload).unwrap();
    let encoded = base64_url::encode(&json);
    let uri = format!("zedra://pair?d={}", encoded);

    // Parse back
    let parsed = parse_pairing_uri(&uri).unwrap();
    assert_eq!(parsed.v, 1);
    assert_eq!(parsed.endpoint_id, endpoint_id);
    assert_eq!(parsed.name, "test-host");
    assert_eq!(parsed.relay_url, payload.relay_url);

    // Convert to EndpointAddr and verify it contains the relay URL
    let parsed_addr = parsed.to_endpoint_addr().unwrap();
    let parsed_relay_urls: Vec<_> = parsed_addr.relay_urls().collect();

    assert!(
        !parsed_relay_urls.is_empty(),
        "parsed endpoint addr should contain the relay URL"
    );
}
