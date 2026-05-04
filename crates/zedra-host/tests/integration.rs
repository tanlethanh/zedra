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

#[cfg(unix)]
fn process_exists(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

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
/// registry, and runs Register → Connect → Challenge → AuthProve.
async fn register_client_for_session(
    relay_url: iroh::RelayUrl,
    host_endpoint: &iroh::Endpoint,
    registry: &Arc<SessionRegistry>,
    session_id: &str,
) -> anyhow::Result<(
    irpc::Client<ZedraProto>,
    ed25519_dalek::SigningKey,
    [u8; 32],
)> {
    use ed25519_dalek::SigningKey;
    use std::time::{SystemTime, UNIX_EPOCH};

    let client_signing_key = SigningKey::generate(&mut rand::thread_rng());
    let client_pubkey = client_signing_key.verifying_key().to_bytes();

    let handshake_key: [u8; 16] = rand::random();
    registry.add_pairing_slot(session_id, handshake_key).await;

    let client_endpoint = make_endpoint(relay_url).await?;
    wait_online(&client_endpoint).await;

    let conn = client_endpoint
        .connect(host_endpoint.addr(), proto::ZEDRA_ALPN)
        .await?;
    let remote = irpc_iroh::IrohRemoteConnection::new(conn);
    let client = irpc::Client::<ZedraProto>::boxed(remote);

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
            session_id: session_id.to_string(),
        })
        .await?;
    if !matches!(reg_result, RegisterResult::Ok) {
        anyhow::bail!("register failed: {:?}", reg_result);
    }

    Ok((client, client_signing_key, client_pubkey))
}

async fn prove_registered_client(
    client: &irpc::Client<ZedraProto>,
    client_signing_key: &ed25519_dalek::SigningKey,
    client_pubkey: [u8; 32],
    session_id: &str,
    host_identity: &Arc<HostIdentity>,
) -> anyhow::Result<AuthProveResult> {
    use ed25519_dalek::{Verifier, VerifyingKey};

    let connect_result: ConnectResult = client
        .rpc(ConnectReq {
            client_pubkey,
            session_id: session_id.to_string(),
            session_token: None,
        })
        .await?;

    let nonce = match connect_result {
        ConnectResult::Challenge {
            nonce,
            host_signature,
        } => {
            let host_pk_bytes = *host_identity.endpoint_id().as_bytes();
            let host_vk = VerifyingKey::from_bytes(&host_pk_bytes)?;
            let host_sig = ed25519_dalek::Signature::from_bytes(&host_signature);
            host_vk.verify(&nonce, &host_sig)?;
            nonce
        }
        other => anyhow::bail!("expected Challenge, got {:?}", other),
    };

    let client_signature = client_signing_key.sign(&nonce).to_bytes();
    Ok(client
        .rpc(AuthProveReq {
            nonce,
            client_signature,
            session_id: session_id.to_string(),
        })
        .await?)
}

async fn connect_client(
    relay_url: iroh::RelayUrl,
    host_endpoint: &iroh::Endpoint,
    registry: &Arc<SessionRegistry>,
    host_identity: &Arc<HostIdentity>,
) -> anyhow::Result<(
    irpc::Client<ZedraProto>,
    String,
    [u8; 32],
    SyncSessionResult,
)> {
    let session = registry
        .create_named("test", std::path::PathBuf::from("/tmp/test"))
        .await;

    let (client, client_signing_key, client_pubkey) =
        register_client_for_session(relay_url, host_endpoint, registry, &session.id).await?;

    let prove_result = prove_registered_client(
        &client,
        &client_signing_key,
        client_pubkey,
        &session.id,
        host_identity,
    )
    .await?;

    let sync = match prove_result {
        AuthProveResult::Ok(sync) => sync,
        other => anyhow::bail!("auth prove failed: {:?}", other),
    };

    Ok((client, session.id.clone(), client_pubkey, sync))
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

    let conn = ep_b.connect(ep_a_addr, proto::ZEDRA_ALPN).await.unwrap();
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

        let msg = irpc_iroh::read_request::<ZedraProto>(&conn).await.unwrap();

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
    let conn = ep_b.connect(ep_a_addr, proto::ZEDRA_ALPN).await.unwrap();
    let remote = irpc_iroh::IrohRemoteConnection::new(conn);
    let client = irpc::Client::<ZedraProto>::boxed(remote);

    let result: PongResult = client
        .rpc(PingReq {
            timestamp_ms: 12345,
        })
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

    let (client, session_id, _client_pubkey, sync) =
        connect_client(relay_url, &host_ep, &registry, &identity)
            .await
            .unwrap();
    assert!(!session_id.is_empty());
    assert_eq!(sync.session_id, session_id);
    assert_ne!(sync.session_token, [0u8; 32]);

    let info: SessionInfoResult = client.rpc(SessionInfoReq {}).await.unwrap();
    assert!(!info.hostname.is_empty());
    assert!(!info.workdir.is_empty());
    assert_eq!(info.session_id.as_deref(), Some(session_id.as_str()));
}

/// A new authorized client is still blocked while the current active client is live.
#[tokio::test(flavor = "multi_thread")]
async fn test_live_second_client_auth_returns_host_occupied() {
    let (_relay, relay_url) = spawn_test_relay().await.unwrap();
    let (host_ep, registry, identity, _dir) = setup_host(relay_url.clone()).await.unwrap();

    let (client_a, session_id, _client_a_pubkey, _sync) =
        connect_client(relay_url.clone(), &host_ep, &registry, &identity)
            .await
            .unwrap();

    let (client_b, signing_key_b, pubkey_b) =
        register_client_for_session(relay_url, &host_ep, &registry, &session_id)
            .await
            .unwrap();
    let result =
        prove_registered_client(&client_b, &signing_key_b, pubkey_b, &session_id, &identity)
            .await
            .unwrap();

    assert!(
        matches!(result, AuthProveResult::SessionOccupied),
        "expected SessionOccupied, got {:?}",
        result
    );

    let info: SessionInfoResult = client_a.rpc(SessionInfoReq {}).await.unwrap();
    assert_eq!(info.session_id.as_deref(), Some(session_id.as_str()));
}

/// SwitchSession is kept in the protocol, but the host cannot change the
/// per-connection dispatch session after authentication.
#[tokio::test(flavor = "multi_thread")]
async fn test_switch_session_returns_explicit_unsupported_result() {
    let (_relay, relay_url) = spawn_test_relay().await.unwrap();
    let (host_ep, registry, identity, _dir) = setup_host(relay_url.clone()).await.unwrap();

    let (client, _session_id, _client_pubkey, _sync) =
        connect_client(relay_url, &host_ep, &registry, &identity)
            .await
            .unwrap();

    let result: SessionSwitchResult = client
        .rpc(SessionSwitchReq {
            session_name: "test".to_string(),
            last_notif_seq: 0,
        })
        .await
        .unwrap();

    assert!(result.session_id.is_empty());
    assert!(result.workdir.is_none());
    assert!(
        result
            .error
            .as_deref()
            .is_some_and(|error| error.contains("unsupported")),
        "expected unsupported error, got {:?}",
        result.error
    );
}

/// Host info snapshots stream over a separate server-streaming subscription.
#[tokio::test(flavor = "multi_thread")]
async fn test_host_info_subscription_over_relay() {
    let (_relay, relay_url) = spawn_test_relay().await.unwrap();
    let (host_ep, registry, identity, _dir) = setup_host(relay_url.clone()).await.unwrap();

    let (client, _session_id, _client_pubkey, _sync) =
        connect_client(relay_url, &host_ep, &registry, &identity)
            .await
            .unwrap();

    let mut snapshots = client
        .server_streaming(SubscribeHostInfoReq {}, 4)
        .await
        .unwrap();

    let first = tokio::time::timeout(Duration::from_secs(8), snapshots.recv())
        .await
        .expect("timed out waiting for first host info snapshot")
        .unwrap()
        .expect("host info stream closed before first snapshot");
    assert!(first.captured_at_ms > 0);
    assert!(first.cpu_count > 0);
    assert!(first.memory_total_bytes > 0);
    assert!(first.memory_used_bytes <= first.memory_total_bytes);

    let second = tokio::time::timeout(Duration::from_secs(7), snapshots.recv())
        .await
        .expect("timed out waiting for second host info snapshot")
        .unwrap()
        .expect("host info stream closed before second snapshot");
    assert!(second.captured_at_ms >= first.captured_at_ms);
}

/// Terminal creation and I/O over iroh relay using bidi streaming.
#[tokio::test(flavor = "multi_thread")]
async fn test_rpc_terminal_over_relay() {
    let (_relay, relay_url) = spawn_test_relay().await.unwrap();
    let (host_ep, registry, identity, _dir) = setup_host(relay_url.clone()).await.unwrap();

    let (client, session_id, _client_pubkey, _sync) =
        connect_client(relay_url, &host_ep, &registry, &identity)
            .await
            .unwrap();

    // Create terminal
    let result: TermCreateResult = client
        .rpc(TermCreateReq {
            cols: 80,
            rows: 24,
            launch_cmd: None,
        })
        .await
        .unwrap();
    assert!(uuid::Uuid::parse_str(&result.id).is_ok());
    let terminal_id = result.id.clone();

    #[cfg(unix)]
    let child_pid = {
        let session = registry.get(&session_id).await.unwrap();
        let terminals = session.terminals.lock().await;
        terminals
            .get(&terminal_id)
            .and_then(|terminal| terminal.child.process_id())
            .expect("terminal child should have a pid")
    };

    // Attach to terminal via bidi streaming
    let (input_tx, mut output_rx) = client
        .bidi_streaming::<TermAttachReq, TermInput, TermOutput>(
            TermAttachReq {
                id: terminal_id.clone(),
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

    let close_result: TermCloseResult = client
        .rpc(TermCloseReq {
            id: terminal_id.clone(),
        })
        .await
        .unwrap();
    assert!(close_result.ok);

    let session = registry.get(&session_id).await.unwrap();
    assert!(!session.terminals.lock().await.contains_key(&terminal_id));

    #[cfg(unix)]
    assert!(!process_exists(child_pid));

    let close_again: TermCloseResult = client.rpc(TermCloseReq { id: terminal_id }).await.unwrap();
    assert!(!close_again.ok);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_terminal_reorder_updates_host_list_and_sync_order() {
    let (_relay, relay_url) = spawn_test_relay().await.unwrap();
    let (host_ep, registry, identity, _dir) = setup_host(relay_url.clone()).await.unwrap();

    let (client, _session_id, _client_pubkey, _sync) =
        connect_client(relay_url, &host_ep, &registry, &identity)
            .await
            .unwrap();

    let mut ids = Vec::new();
    for _ in 0..3 {
        let result: TermCreateResult = client
            .rpc(TermCreateReq {
                cols: 80,
                rows: 24,
                launch_cmd: None,
            })
            .await
            .unwrap();
        assert!(result.error.is_none());
        ids.push(result.id);
    }

    let initial_list: TermListResult = client.rpc(TermListReq {}).await.unwrap();
    assert_eq!(
        initial_list
            .terminals
            .iter()
            .map(|entry| entry.id.clone())
            .collect::<Vec<_>>(),
        ids
    );

    let reordered_ids = vec![ids[2].clone(), ids[0].clone(), ids[1].clone()];
    let reorder: TermReorderResult = client
        .rpc(TermReorderReq {
            ordered_ids: reordered_ids.clone(),
        })
        .await
        .unwrap();
    assert!(reorder.ok, "{:?}", reorder.error);

    let list: TermListResult = client.rpc(TermListReq {}).await.unwrap();
    assert_eq!(
        list.terminals
            .iter()
            .map(|entry| entry.id.clone())
            .collect::<Vec<_>>(),
        reordered_ids
    );
    assert_eq!(
        list.terminals
            .iter()
            .map(|entry| entry.position)
            .collect::<Vec<_>>(),
        vec![0, 1, 2]
    );

    let sync: SyncSessionResult = client.rpc(SyncSessionReq {}).await.unwrap();
    assert_eq!(
        sync.terminals
            .iter()
            .map(|entry| entry.id.clone())
            .collect::<Vec<_>>(),
        reordered_ids
    );
    assert_eq!(
        sync.terminals
            .iter()
            .map(|entry| entry.position)
            .collect::<Vec<_>>(),
        vec![0, 1, 2]
    );

    let duplicate: TermReorderResult = client
        .rpc(TermReorderReq {
            ordered_ids: vec![ids[0].clone(), ids[0].clone(), ids[1].clone()],
        })
        .await
        .unwrap();
    assert!(!duplicate.ok);

    for id in ids {
        let _ = client.rpc(TermCloseReq { id }).await.unwrap();
    }
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

    // Try to connect without registering (unknown client, no session token)
    let unknown_pubkey = [99u8; 32];
    let result: ConnectResult = client
        .rpc(ConnectReq {
            client_pubkey: unknown_pubkey,
            session_id: "nonexistent".to_string(),
            session_token: None,
        })
        .await
        .unwrap();

    assert!(
        matches!(result, ConnectResult::Unauthorized),
        "expected Unauthorized, got {:?}",
        result
    );
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
            session_id: session.id.clone(),
        })
        .await
        .unwrap();

    assert!(
        matches!(result, RegisterResult::InvalidHandshake),
        "expected InvalidHandshake, got {:?}",
        result
    );
}
