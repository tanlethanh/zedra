// RPC daemon: exposes filesystem, git, terminal, LSP, and AI operations over irpc.
//
// Connection lifecycle:
//   First pairing:  Register → Connect → Challenge → AuthProve → Ok(SyncSessionResult) → (RPC calls)
//   Token resume:   Connect(session_token) → Ok(SyncSessionResult) → (RPC calls)
//   PKI reconnect:  Connect(None) → Challenge → AuthProve → Ok(SyncSessionResult) → (RPC calls)
//   Health:         Ping (every 2s, foreground only, 5 misses = client reconnects)

use crate::fs::{Filesystem, LocalFs};
use crate::git::GitRepo;
use crate::identity::SharedIdentity;
use crate::pty::{ShellSession, SpawnOptions};
use crate::session_registry::{
    AttachResult, ConsumeSlotResult, HostTermMeta, OutputSenderSlot, ServerSession,
    SessionRegistry, TermBacklog, TermSession, MAX_WATCHED_PATHS_PER_SESSION,
};
use anyhow::Result;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use zedra_osc::OscEvent;
use zedra_rpc::proto::*;
use zedra_telemetry::Event;

struct HostEnvInfo {
    hostname: String,
    username: String,
    workdir: String,
    home_dir: Option<String>,
}

fn collect_host_env(workdir: &std::path::Path) -> HostEnvInfo {
    HostEnvInfo {
        hostname: hostname::get()
            .ok()
            .and_then(|h| h.into_string().ok())
            .unwrap_or_else(|| "unknown".to_string()),
        username: std::env::var("USER").unwrap_or_else(|_| "unknown".to_string()),
        workdir: workdir.to_string_lossy().into_owned(),
        home_dir: std::env::var("HOME").ok(),
    }
}

async fn build_sync_result(
    session: &Arc<ServerSession>,
    state: &DaemonState,
    session_token: [u8; 32],
) -> SyncSessionResult {
    let info = collect_host_env(&state.workdir);

    SyncSessionResult {
        session_id: session.id.clone(),
        session_token,
        hostname: info.hostname,
        workdir: info.workdir,
        username: info.username,
        home_dir: info.home_dir,
        os: Some(std::env::consts::OS.to_string()),
        arch: Some(std::env::consts::ARCH.to_string()),
        os_version: os_version_string(),
        host_version: Some(env!("CARGO_PKG_VERSION").to_string()),
        terminals: session.terminal_sync_entries().await,
    }
}

fn ts() -> String {
    let s = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!(
        "{:02}:{:02}:{:02}",
        (s % 86400) / 3600,
        (s % 3600) / 60,
        s % 60
    )
}

/// Build a synthetic OSC preamble encoding cached title/CWD.
/// Sent as seq=0 on TermAttach so the client seeds its meta from the PTY stream.
fn encode_meta_preamble(title: &Option<String>, cwd: &Option<String>) -> Vec<u8> {
    let mut out = Vec::new();
    if let Some(t) = title {
        out.extend_from_slice(b"\x1b]2;");
        out.extend_from_slice(t.as_bytes());
        out.push(0x07);
    }
    if let Some(c) = cwd {
        out.extend_from_slice(b"\x1b]7;file://");
        out.extend_from_slice(c.as_bytes());
        out.push(0x07);
    }
    out
}

fn short_key(key: &[u8; 32]) -> String {
    key[..4].iter().map(|b| format!("{b:02x}")).collect()
}

/// Snapshot the iroh connection path type at the current moment.
/// Returns "direct" for P2P, "relay" for relay-only, "unknown" if undetermined.
fn initial_path_type(conn: &iroh::endpoint::Connection) -> &'static str {
    use iroh::Watcher;
    let mut paths = conn.paths();
    let path_list = paths.get();
    let result = path_list
        .iter()
        .find(|p| p.is_selected())
        .map(|p| if p.is_ip() { "direct" } else { "relay" })
        .unwrap_or("unknown");
    drop(path_list);
    result
}

/// Resolve `user_path` relative to `workdir`, then verify the canonical path
/// stays inside `workdir`. Rejects absolute paths, `..` escapes, and symlinks
/// that point outside the jail.
fn resolve_path(workdir: &Path, user_path: &str) -> Result<PathBuf> {
    // Reject empty paths
    anyhow::ensure!(!user_path.is_empty(), "empty path");
    let joined = workdir.join(user_path);
    let resolved = joined.canonicalize().or_else(|_| {
        // File may not exist yet (e.g. FsWrite to a new path).
        // Walk up to the first existing ancestor and canonicalize that.
        let mut base = joined.as_path();
        while let Some(parent) = base.parent() {
            if parent.exists() {
                let canon = parent.canonicalize()?;
                // Reconstruct: canon + the non-existing tail
                let tail = joined.strip_prefix(parent).unwrap_or(base);
                return Ok(canon.join(tail));
            }
            base = parent;
        }
        anyhow::bail!("could not resolve path");
    })?;
    let jail = workdir.canonicalize()?;
    anyhow::ensure!(
        resolved.starts_with(&jail),
        "path {} escapes workspace {}",
        resolved.display(),
        jail.display(),
    );
    Ok(resolved)
}

/// Normalize a client-provided observer path into a canonical relative key.
/// Returns `None` for invalid input (absolute paths or parent traversal).
fn normalize_observer_path(path: &str) -> Option<String> {
    let raw = path.trim();
    if raw.is_empty() {
        return None;
    }
    let p = Path::new(raw);
    if p.is_absolute() {
        return None;
    }
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            std::path::Component::CurDir => {}
            std::path::Component::Normal(seg) => out.push(seg),
            _ => return None,
        }
    }
    if out.as_os_str().is_empty() {
        Some(".".to_string())
    } else {
        Some(out.to_string_lossy().into_owned())
    }
}

fn git_status_fingerprint(workdir: &Path) -> Option<u64> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(workdir)
        .arg("status")
        .arg("--porcelain=v1")
        .arg("--branch")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    output.stdout.hash(&mut hasher);
    Some(hasher.finish())
}

fn fs_dir_fingerprint(workdir: &Path, rel_path: &str) -> Option<u64> {
    let target = resolve_path(workdir, rel_path).ok()?;
    let entries = std::fs::read_dir(target).ok()?;
    // File explorer invalidation should track tree shape changes only.
    // Including mtime/size causes noisy false positives (for example `.git`
    // metadata churn) that collapse expanded directories via root reload.
    let mut rows: Vec<(String, bool)> = Vec::new();
    for entry in entries.flatten() {
        let meta = entry.metadata().ok()?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_dir = meta.is_dir();
        rows.push((name, is_dir));
    }
    rows.sort();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    rows.hash(&mut hasher);
    Some(hasher.finish())
}

async fn run_observer(session: Arc<ServerSession>, workdir: PathBuf, my_gen: u64) {
    let mut last_git: Option<u64> = None;
    let mut fs_snapshots: HashMap<String, u64> = HashMap::new();
    let mut tick_count: u64 = 0;
    loop {
        let current = session.observer_gen.load(Ordering::Acquire);
        if current != my_gen {
            break;
        }

        if let Ok(git_hash) = tokio::task::spawn_blocking({
            let workdir = workdir.clone();
            move || git_status_fingerprint(&workdir)
        })
        .await
        {
            if let Some(git_hash) = git_hash {
                if last_git.is_some() && last_git != Some(git_hash) {
                    let _ = session.push_event(HostEvent::GitChanged).await;
                }
                last_git = Some(git_hash);
            }
        }

        let watched: Vec<String> = {
            let set = session.fs_watched_paths.lock().await;
            set.iter().cloned().collect()
        };
        let watched_len = watched.len();

        let mut retained: HashMap<String, u64> = HashMap::new();
        for path in watched {
            let fingerprint = match tokio::task::spawn_blocking({
                let workdir = workdir.clone();
                let path_clone = path.clone();
                move || fs_dir_fingerprint(&workdir, &path_clone)
            })
            .await
            {
                Ok(v) => v,
                Err(_) => None,
            };
            let Some(next_hash) = fingerprint else {
                continue;
            };
            if let Some(prev_hash) = fs_snapshots.get(&path) {
                if *prev_hash != next_hash {
                    let _ = session
                        .push_event(HostEvent::FsChanged { path: path.clone() })
                        .await;
                }
            }
            retained.insert(path, next_hash);
        }
        fs_snapshots = retained;

        tick_count += 1;
        if tick_count % 30 == 0 {
            tracing::info!(
                "observer metrics: session={} watched={} sent={} dropped_full={} dropped_no_subscriber={} rate_limited={} quota_rejected={}",
                session.id,
                watched_len,
                session.observer_events_sent.load(Ordering::Relaxed),
                session.observer_events_dropped_full.load(Ordering::Relaxed),
                session
                    .observer_events_dropped_no_subscriber
                    .load(Ordering::Relaxed),
                session.fs_watch_rate_limited.load(Ordering::Relaxed),
                session.fs_watch_quota_rejected.load(Ordering::Relaxed),
            );
        }

        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

/// Shared state for RPC handlers.
pub struct DaemonState {
    pub fs: Arc<dyn Filesystem>,
    pub workdir: std::path::PathBuf,
    /// Host identity for signing challenges in the Authenticate step.
    pub identity: SharedIdentity,
    /// When the daemon started; used to compute uptime.
    pub started_at: std::time::Instant,
}

impl std::fmt::Debug for DaemonState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DaemonState")
            .field("workdir", &self.workdir)
            .finish_non_exhaustive()
    }
}

impl DaemonState {
    pub fn new(workdir: std::path::PathBuf, identity: SharedIdentity) -> Self {
        Self {
            fs: Arc::new(LocalFs),
            workdir,
            identity,
            started_at: std::time::Instant::now(),
        }
    }
}

// ---------------------------------------------------------------------------
// Connection handler
// ---------------------------------------------------------------------------

/// Handle a single iroh connection using the irpc protocol.
///
/// Auth phase: optional Register, then Authenticate → AuthProve.
/// After successful auth, enters the RPC dispatch loop.
pub async fn handle_connection(
    conn: iroh::endpoint::Connection,
    registry: Arc<SessionRegistry>,
    state: Arc<DaemonState>,
) -> Result<()> {
    let remote = conn.remote_id();
    tracing::info!("connection from {}", remote.fmt_short());

    // Auth phase: returns (session, client_pubkey, is_new_client) or closes connection
    let auth_start = std::time::Instant::now();
    let path_type = initial_path_type(&conn);
    let mut failure_reason: &'static str = "io_error";
    let mut failure_is_new_client = false;
    let (session, client_pubkey, is_new_client, auth_timing) = match auth_phase(
        &conn,
        &registry,
        &state,
        &mut failure_reason,
        &mut failure_is_new_client,
    )
    .await
    {
        Ok(quad) => quad,
        Err(e) => {
            zedra_telemetry::send(Event::AuthFailed {
                reason: failure_reason,
                elapsed_ms: auth_start.elapsed().as_millis() as u64,
                is_new_client: failure_is_new_client,
                path_type,
            });
            tracing::warn!("auth failed from {}: {}", remote.fmt_short(), e);
            // Wait for the client to close the connection (up to 500ms) so any
            // error response we sent has time to be delivered before CONNECTION_CLOSE.
            let _ =
                tokio::time::timeout(std::time::Duration::from_millis(500), conn.closed()).await;
            return Ok(());
        }
    };

    zedra_telemetry::send(Event::AuthSuccess {
        is_new_client,
        register_ms: auth_timing.register_ms,
        challenge_ms: auth_timing.challenge_ms,
        prove_ms: auth_timing.prove_ms,
        total_ms: auth_start.elapsed().as_millis() as u64,
        path_type,
    });

    tracing::info!(
        "Authenticated client {:?}... → session={}",
        &client_pubkey[..4],
        session.id,
    );
    eprintln!(
        "[{}] connected: {} → session {}",
        ts(),
        short_key(&client_pubkey),
        &session.id[..8.min(session.id.len())]
    );

    let session_start = std::time::Instant::now();

    // Spawn bandwidth sampler: reads iroh path stats every 60s while connected.
    {
        use iroh::Watcher;
        let conn_for_bw = conn.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            interval.tick().await; // skip immediate first tick
            let mut paths = conn_for_bw.paths(); // hold watcher for the lifetime of the task
            let mut prev_tx: u64 = 0;
            let mut prev_rx: u64 = 0;
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let path_list = paths.get();
                        let mut cur_tx = 0u64;
                        let mut cur_rx = 0u64;
                        let mut bw_found = false;
                        for p in path_list.iter() {
                            if p.is_selected() {
                                let s = p.stats();
                                cur_tx = s.udp_tx.bytes;
                                cur_rx = s.udp_rx.bytes;
                                bw_found = true;
                                break;
                            }
                        }
                        drop(path_list);
                        if bw_found {
                            let delta_tx = cur_tx.saturating_sub(prev_tx);
                            let delta_rx = cur_rx.saturating_sub(prev_rx);
                            prev_tx = cur_tx;
                            prev_rx = cur_rx;
                            zedra_telemetry::send(Event::BandwidthSample {
                                bytes_sent: delta_tx,
                                bytes_recv: delta_rx,
                                interval_secs: 60,
                            });
                        }
                    }
                    _ = conn_for_bw.closed() => break,
                }
            }
        });
    }

    // RPC dispatch loop
    loop {
        match irpc_iroh::read_request::<ZedraProto>(&conn).await {
            Ok(Some(msg)) => {
                let s = session.clone();
                let st = state.clone();
                let r = registry.clone();
                let cpk = client_pubkey;
                tokio::spawn(async move {
                    if let Err(e) = dispatch(msg, s, st, r, cpk).await {
                        tracing::warn!("dispatch error: {}", e);
                    }
                });
            }
            Ok(None) => break,
            Err(e) => {
                tracing::debug!("read_request error: {}", e);
                break;
            }
        }
    }

    // Cleanup on disconnect.
    // clear_output_senders() is intentionally NOT called here: the TermAttach
    // cleanup above guards its None-set with a generation check, and the PTY
    // reader task self-heals by clearing a dead sender on the next write attempt.
    // Calling it here would race with a concurrent new TermAttach and silence
    // the new client's output.
    let session_duration_ms = session_start.elapsed().as_millis() as u64;
    let terminal_count = session.terminals.lock().await.len() as u64;
    zedra_telemetry::send(Event::SessionEnd {
        duration_ms: session_duration_ms,
        terminal_count,
        path_type,
        fs_reads: session.rpc_fs_reads.load(Ordering::Relaxed),
        fs_writes: session.rpc_fs_writes.load(Ordering::Relaxed),
        git_ops: session.rpc_git_ops.load(Ordering::Relaxed),
        git_commits: session.rpc_git_commits.load(Ordering::Relaxed),
        ai_prompts: session.rpc_ai_prompts.load(Ordering::Relaxed),
    });

    registry.detach_client(&session.id, client_pubkey).await;

    tracing::info!(
        "Connection closed: session={} (session stays alive in registry)",
        session.id,
    );
    eprintln!(
        "[{}] disconn:   {} (session {})",
        ts(),
        short_key(&client_pubkey),
        &session.id[..8.min(session.id.len())]
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Auth phase
// ---------------------------------------------------------------------------

struct AuthTiming {
    register_ms: u64,
    challenge_ms: u64,
    prove_ms: u64,
}

/// Perform the full auth handshake for a new connection.
///
/// Flow:
///   1. Optional Register (first-time only, proves QR possession via HMAC)
///   2. Connect — universal initiator for all non-Register paths:
///      - session_token present and valid → Ok(SyncSessionResult) fast path
///      - otherwise → Challenge (nonce + host_sig embedded, saves Authenticate RTT)
///   3. AuthProve (client signs nonce, specifies session to attach)
///      → Ok(SyncSessionResult) (bootstrap data piggybacked, no SyncSession needed)
async fn auth_phase(
    conn: &iroh::endpoint::Connection,
    registry: &Arc<SessionRegistry>,
    state: &Arc<DaemonState>,
    failure_reason: &mut &'static str,
    failure_is_new_client: &mut bool,
) -> Result<(Arc<ServerSession>, [u8; 32], bool, AuthTiming)> {
    let first = irpc_iroh::read_request::<ZedraProto>(conn).await?;

    match first {
        Some(ZedraMessage::Register(msg)) => {
            // First pairing: verify HMAC, consume slot, add to ACL.
            // After success, expect Connect (which will always issue a Challenge
            // since no session_token exists yet for a brand-new client).
            *failure_is_new_client = true;
            let t = std::time::Instant::now();
            let result = handle_register(&msg, registry).await;
            let ok = matches!(result, RegisterResult::Ok);
            let register_ms = t.elapsed().as_millis() as u64;
            *failure_reason = match &result {
                RegisterResult::StaleTimestamp => "stale_timestamp",
                RegisterResult::InvalidHandshake => "bad_hmac",
                RegisterResult::HandshakeConsumed => "slot_consumed",
                RegisterResult::SlotNotFound => "slot_not_found",
                RegisterResult::Ok => "io_error",
            };
            let _ = msg.tx.send(result).await;
            if !ok {
                anyhow::bail!("register rejected");
            }
            let connect_msg = irpc_iroh::read_request::<ZedraProto>(conn).await?;
            match connect_msg {
                Some(ZedraMessage::Connect(msg)) => {
                    // is_new_client = true: came through the Register path
                    let t_connect = std::time::Instant::now();
                    let (session, pubkey, is_new, prove_ms) =
                        handle_connect(msg, conn, registry, state, true, failure_reason).await?;
                    Ok((
                        session,
                        pubkey,
                        is_new,
                        AuthTiming {
                            register_ms,
                            challenge_ms: t_connect.elapsed().as_millis() as u64,
                            prove_ms,
                        },
                    ))
                }
                _ => {
                    *failure_reason = "unexpected_message";
                    anyhow::bail!("expected Connect after Register")
                }
            }
        }
        Some(ZedraMessage::Connect(msg)) => {
            // PKI reconnect or token resume — is_new_client = false
            let t = std::time::Instant::now();
            let (session, pubkey, is_new, prove_ms) =
                handle_connect(msg, conn, registry, state, false, failure_reason).await?;
            Ok((
                session,
                pubkey,
                is_new,
                AuthTiming {
                    register_ms: 0,
                    challenge_ms: t.elapsed().as_millis() as u64,
                    prove_ms,
                },
            ))
        }
        _ => {
            *failure_reason = "unexpected_message";
            anyhow::bail!("expected Register or Connect as first message")
        }
    }
}

/// Process a `Connect` message. If the client presents a valid session_token,
/// attach immediately and return Ok with SyncSessionResult. Otherwise, issue a
/// Challenge (nonce + host_signature) and wait for AuthProve.
/// Returns (session, pubkey, is_new_client, prove_ms).
async fn handle_connect(
    msg: irpc::WithChannels<ConnectReq, ZedraProto>,
    conn: &iroh::endpoint::Connection,
    registry: &Arc<SessionRegistry>,
    state: &Arc<DaemonState>,
    is_new_client: bool,
    failure_reason: &mut &'static str,
) -> Result<(Arc<ServerSession>, [u8; 32], bool, u64)> {
    let pubkey = msg.client_pubkey;
    let session_id = msg.session_id.clone();

    // Fast path: try session token if client provided one.
    if let Some(token) = msg.session_token {
        let session = registry.get(&session_id).await;
        if let Some(ref session) = session {
            if session.validate_session_token(&pubkey, &token).await {
                match registry.attach_client(&session_id, pubkey).await {
                    AttachResult::Ok => {
                        let new_token = session.issue_session_token(pubkey).await;
                        let sync = build_sync_result(session, state, new_token).await;
                        let _ = msg.tx.send(ConnectResult::Ok(sync)).await;
                        // Token fast-path: no challenge/prove round trip, prove_ms=0
                        return Ok((session.clone(), pubkey, false, 0));
                    }
                    AttachResult::NotInSessionAcl => {
                        *failure_reason = "not_in_session_acl";
                        let _ = msg.tx.send(ConnectResult::NotInSessionAcl).await;
                        anyhow::bail!("client not in session ACL");
                    }
                    AttachResult::SessionOccupied => {
                        *failure_reason = "session_occupied";
                        let _ = msg.tx.send(ConnectResult::SessionOccupied).await;
                        anyhow::bail!("session {} is occupied", session_id);
                    }
                    AttachResult::SessionNotFound => {
                        // Fall through to PKI challenge below
                    }
                }
            }
        }
        // Token invalid/expired or session not found — fall through to challenge
    }

    // Check global authorization before issuing a challenge.
    if !is_new_client && !registry.is_globally_authorized(&pubkey).await {
        // Drop tx to signal error; don't send a challenge to unknown clients.
        *failure_reason = "not_authorized";
        drop(msg.tx);
        anyhow::bail!("client not globally authorized");
    }

    // Issue challenge (nonce + host signature) embedded in ConnectResult::Challenge,
    // saving the separate Authenticate round trip.
    let mut nonce = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut nonce);
    let host_signature = state.identity.sign_challenge(&nonce);
    let _ = msg
        .tx
        .send(ConnectResult::Challenge {
            nonce,
            host_signature,
        })
        .await;

    finish_auth(
        conn,
        registry,
        pubkey,
        nonce,
        state,
        is_new_client,
        failure_reason,
    )
    .await
}

/// Handle a Register request: verify HMAC, consume slot, add to ACL.
async fn handle_register(
    msg: &irpc::WithChannels<RegisterReq, ZedraProto>,
    registry: &Arc<SessionRegistry>,
) -> RegisterResult {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Check timestamp (±60s window)
    if now.abs_diff(msg.timestamp) > 60 {
        tracing::warn!(
            "Register: stale timestamp (now={}, ts={})",
            now,
            msg.timestamp
        );
        return RegisterResult::StaleTimestamp;
    }

    // Atomically consume the pairing slot
    match registry.consume_pairing_slot(&msg.session_id).await {
        ConsumeSlotResult::Active(slot) => {
            // Verify HMAC (slot is already consumed regardless of outcome)
            if !zedra_rpc::verify_registration_hmac(
                &slot.handshake_secret,
                &msg.client_pubkey,
                msg.timestamp,
                &msg.hmac,
            ) {
                tracing::warn!(
                    "Register: invalid HMAC from {:?}...",
                    &msg.client_pubkey[..4]
                );
                return RegisterResult::InvalidHandshake;
            }

            // Add to session ACL + global list
            registry
                .add_client_to_session(&slot.session_id, msg.client_pubkey)
                .await;

            tracing::info!(
                "Register: client {:?}... added to session {}",
                &msg.client_pubkey[..4],
                slot.session_id,
            );
            eprintln!(
                "[{}] paired:    {} → session {}",
                ts(),
                short_key(&msg.client_pubkey),
                &slot.session_id[..8.min(slot.session_id.len())]
            );
            zedra_telemetry::send(Event::ClientPaired);
            RegisterResult::Ok
        }
        ConsumeSlotResult::Consumed => {
            tracing::warn!("Register: slot for {} already consumed", msg.session_id);
            eprintln!(
                "[{}] pairing:   QR already used (session {}). Press 'r' in the host terminal to generate a new QR.",
                ts(),
                &msg.session_id[..8.min(msg.session_id.len())]
            );
            RegisterResult::HandshakeConsumed
        }
        ConsumeSlotResult::NotFound => {
            tracing::warn!("Register: no slot found for session {}", msg.session_id);
            RegisterResult::SlotNotFound
        }
    }
}

/// Read AuthProve, verify client signature, attach to session.
/// Returns (session, pubkey, is_new_client, prove_ms).
/// On success, sends `AuthProveResult::Ok(SyncSessionResult)` so the client
/// has everything it needs without a separate SyncSession round trip.
async fn finish_auth(
    conn: &iroh::endpoint::Connection,
    registry: &Arc<SessionRegistry>,
    client_pubkey: [u8; 32],
    nonce: [u8; 32],
    state: &Arc<DaemonState>,
    is_new_client: bool,
    failure_reason: &mut &'static str,
) -> Result<(Arc<ServerSession>, [u8; 32], bool, u64)> {
    let prove_start = std::time::Instant::now();
    let prove_msg = irpc_iroh::read_request::<ZedraProto>(conn).await?;

    let msg = match prove_msg {
        Some(ZedraMessage::AuthProve(m)) => m,
        _ => {
            *failure_reason = "unexpected_message";
            anyhow::bail!("expected AuthProve")
        }
    };

    // Extract fields before any moves
    let prove_nonce = msg.nonce;
    let prove_sig = msg.client_signature;
    let session_id = msg.session_id.clone();
    let tx = msg.tx;

    // Verify nonce echo
    if prove_nonce != nonce {
        *failure_reason = "nonce_mismatch";
        let _ = tx.send(AuthProveResult::InvalidSignature).await;
        anyhow::bail!("AuthProve: nonce mismatch");
    }

    // Verify client signature of the nonce using stored pubkey
    {
        use ed25519_dalek::{Verifier, VerifyingKey};
        let vk = VerifyingKey::from_bytes(&client_pubkey)
            .map_err(|e| anyhow::anyhow!("invalid client pubkey: {e}"))?;
        let sig = ed25519_dalek::Signature::from_bytes(&prove_sig);
        if vk.verify(&nonce, &sig).is_err() {
            *failure_reason = "invalid_signature";
            let _ = tx.send(AuthProveResult::InvalidSignature).await;
            anyhow::bail!("AuthProve: signature invalid");
        }
    }

    // Attach to the requested session, with fallback for stale session IDs
    // (e.g. after a daemon restart the client's stored session_id is gone).
    let (attach_result, resolved_session_id) =
        match registry.attach_client(&session_id, client_pubkey).await {
            AttachResult::SessionNotFound => {
                // Client is globally authorized but their session was lost.
                // Try to find another session they have ACL for, or create one.
                let fallback =
                    if let Some(s) = registry.find_session_for_client(&client_pubkey).await {
                        tracing::info!(
                            "finish_auth: session {} gone, falling back to session {}",
                            session_id,
                            s.id,
                        );
                        s
                    } else {
                        // No existing session — create a fresh default one.
                        let workdir = &state.workdir;
                        let name = workdir
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("default");
                        let s = registry.create_named(name, workdir.to_path_buf()).await;
                        registry.add_client_to_session(&s.id, client_pubkey).await;
                        tracing::info!(
                            "finish_auth: session {} gone, created new session {} ({})",
                            session_id,
                            s.id,
                            name,
                        );
                        s
                    };
                let new_id = fallback.id.clone();
                (registry.attach_client(&new_id, client_pubkey).await, new_id)
            }
            other => (other, session_id.clone()),
        };

    match attach_result {
        AttachResult::Ok => {
            let Some(session) = registry.get(&resolved_session_id).await else {
                let _ = tx.send(AuthProveResult::SessionNotFound).await;
                anyhow::bail!("session {} vanished after attach", resolved_session_id);
            };
            let session_token = session.issue_session_token(client_pubkey).await;
            let sync = build_sync_result(&session, state, session_token).await;
            let _ = tx.send(AuthProveResult::Ok(sync)).await;
            Ok((
                session,
                client_pubkey,
                is_new_client,
                prove_start.elapsed().as_millis() as u64,
            ))
        }
        AttachResult::SessionNotFound => {
            *failure_reason = "session_not_found";
            let _ = tx.send(AuthProveResult::SessionNotFound).await;
            anyhow::bail!("session {} not found", resolved_session_id)
        }
        AttachResult::NotInSessionAcl => {
            *failure_reason = "not_in_session_acl";
            let _ = tx.send(AuthProveResult::NotInSessionAcl).await;
            anyhow::bail!("client not in session ACL")
        }
        AttachResult::SessionOccupied => {
            *failure_reason = "session_occupied";
            let _ = tx.send(AuthProveResult::SessionOccupied).await;
            anyhow::bail!("session {} is occupied", resolved_session_id)
        }
    }
}

// ---------------------------------------------------------------------------
// Terminal creation (shared by RPC dispatch and REST API)
// ---------------------------------------------------------------------------

pub const MAX_TERMINALS_PER_SESSION: usize = 16;

/// Spawn a new PTY shell and register it in `session`.
///
/// Returns the new terminal ID on success. Used by both the `TermCreate` RPC
/// handler and the local REST API so both paths share identical behaviour.
pub async fn create_terminal(
    session: &Arc<ServerSession>,
    cols: u16,
    rows: u16,
    opts: SpawnOptions,
) -> Result<String> {
    if session.terminals.lock().await.len() >= MAX_TERMINALS_PER_SESSION {
        anyhow::bail!(
            "session {} already has {} terminals (limit {})",
            session.id,
            MAX_TERMINALS_PER_SESSION,
            MAX_TERMINALS_PER_SESSION,
        );
    }

    let shell = ShellSession::spawn(cols, rows, opts)?;
    let (pty_reader, pty_writer, master) = shell.take_reader();
    let id = session.next_terminal_id().await;

    tracing::info!(
        "create_terminal: id={} cols={} rows={} session={}",
        id,
        cols,
        rows,
        session.id,
    );

    let output_sender = Arc::new(std::sync::Mutex::new(OutputSenderSlot {
        gen: 0,
        sender: None,
    }));
    let host_meta = Arc::new(std::sync::Mutex::new(HostTermMeta::default()));
    let backlog = Arc::new(std::sync::Mutex::new(TermBacklog::new()));
    // Wrap the writer so TermAttach can hold a direct Arc clone and write
    // without locking session.terminals on every keystroke (Fix 3).
    let writer = Arc::new(std::sync::Mutex::new(pty_writer));

    session.terminals.lock().await.insert(
        id.clone(),
        TermSession {
            writer: writer.clone(),
            master,
            output_sender: output_sender.clone(),
            host_meta: host_meta.clone(),
            backlog: backlog.clone(),
        },
    );

    let term_id = id.clone();
    tokio::task::spawn_blocking(move || {
        let mut reader = pty_reader;
        let mut buf = [0u8; 8192];
        // Chunks that couldn't be sent (channel full) are held here and
        // coalesced with the next PTY read. This keeps the spawn_blocking
        // thread alive under QUIC back-pressure without blocking (Fix 2).
        let mut pending: Option<TermOutput> = None;
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let data = buf[..n].to_vec();

                    // Scan for OSC sequences to keep the per-terminal metadata
                    // cache (title, CWD) up to date. This runs on every PTY
                    // chunk so the host always has the latest values even after
                    // old backlog entries have been evicted.
                    if let Ok(mut m) = host_meta.lock() {
                        let events = m.scanner.feed(&data);
                        for ev in events {
                            match ev {
                                OscEvent::Title(t) => m.title = Some(t),
                                OscEvent::ResetTitle => m.title = None,
                                OscEvent::Cwd(c) => m.cwd = Some(c),
                                _ => {}
                            }
                        }
                    }

                    // Push to per-terminal backlog (Fix 1: sync, no rt.block_on).
                    let seq = backlog.lock().unwrap().push(term_id.clone(), data.clone());

                    let sender: Option<tokio::sync::mpsc::Sender<TermOutput>> =
                        output_sender.lock().unwrap().sender.clone();
                    if let Some(tx) = sender {
                        // Coalesce new data with any previously unsent chunk,
                        // then attempt a non-blocking send (Fix 2).
                        let out = match pending.take() {
                            Some(mut p) => {
                                p.data.extend_from_slice(&data);
                                p.seq = seq;
                                p
                            }
                            None => TermOutput { data, seq },
                        };
                        match tx.try_send(out) {
                            Ok(()) => {}
                            Err(tokio::sync::mpsc::error::TrySendError::Full(ret)) => {
                                // Channel full (QUIC congested): hold for next iteration.
                                pending = Some(ret);
                            }
                            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                                output_sender.lock().unwrap().sender = None;
                            }
                        }
                    }
                }
                Err(_) => break,
            }
        }
    });

    Ok(id)
}

// ---------------------------------------------------------------------------
// RPC dispatch
// ---------------------------------------------------------------------------

async fn dispatch(
    msg: ZedraMessage,
    session: Arc<ServerSession>,
    state: Arc<DaemonState>,
    registry: Arc<SessionRegistry>,
    client_pubkey: [u8; 32],
) -> Result<()> {
    match msg {
        // -- Auth / bootstrap (should not appear in dispatch loop) --
        ZedraMessage::Register(_)
        | ZedraMessage::Authenticate(_)
        | ZedraMessage::AuthProve(_)
        | ZedraMessage::Connect(_) => {
            tracing::warn!("auth message received in dispatch loop (ignored)");
        }

        // -- Health --
        ZedraMessage::Ping(msg) => {
            session.touch().await;
            let ts = msg.timestamp_ms;
            let _ = msg.tx.send(PongResult { timestamp_ms: ts }).await;
        }

        // -- Session --
        ZedraMessage::GetSessionInfo(msg) => {
            let info = collect_host_env(&state.workdir);
            let _ = msg
                .tx
                .send(SessionInfoResult {
                    hostname: info.hostname,
                    workdir: info.workdir,
                    username: info.username,
                    home_dir: info.home_dir,
                    session_id: Some(session.id.clone()),
                    os: Some(std::env::consts::OS.to_string()),
                    arch: Some(std::env::consts::ARCH.to_string()),
                    os_version: os_version_string(),
                    host_version: Some(env!("CARGO_PKG_VERSION").to_string()),
                })
                .await;
        }

        ZedraMessage::SyncSession(msg) => {
            let session_token = session.issue_session_token(client_pubkey).await;
            let _ = msg
                .tx
                .send(build_sync_result(&session, &state, session_token).await)
                .await;
        }

        ZedraMessage::ListSessions(msg) => {
            let list = registry.list_sessions().await;
            let sessions = list
                .into_iter()
                .map(|s| SessionListEntry {
                    id: s.id,
                    name: s.name,
                    workdir: s.workdir.map(|p| p.to_string_lossy().into_owned()),
                    terminal_count: s.terminal_count,
                    uptime_secs: s.created_at_elapsed_secs,
                    idle_secs: s.last_activity_elapsed_secs,
                    is_occupied: s.is_occupied,
                })
                .collect();
            let _ = msg.tx.send(SessionListResult { sessions }).await;
        }

        ZedraMessage::SwitchSession(msg) => {
            // Verify the client is authorized in the target session's ACL,
            // not just globally. This prevents a client from switching to a
            // session it was never paired with.
            match registry.get_by_name(&msg.session_name).await {
                Some(target) if target.acl.lock().await.contains(&client_pubkey) => {
                    target.touch().await;
                    let workdir = target
                        .workdir
                        .as_ref()
                        .map(|p| p.to_string_lossy().into_owned());
                    let _ = msg
                        .tx
                        .send(SessionSwitchResult {
                            session_id: target.id.clone(),
                            workdir,
                        })
                        .await;
                }
                _ => {
                    drop(msg.tx);
                }
            }
        }

        // -- Filesystem --
        ZedraMessage::FsList(msg) => {
            let path = match resolve_path(&state.workdir, &msg.path) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!("FsList: rejected path {:?}: {}", msg.path, e);
                    drop(msg.tx);
                    return Ok(());
                }
            };
            match state.fs.list(&path) {
                Ok(entries) => {
                    let total = entries.len() as u32;
                    let limit = if msg.limit == 0 {
                        FS_LIST_DEFAULT_LIMIT
                    } else {
                        msg.limit.min(FS_LIST_DEFAULT_LIMIT)
                    } as usize;
                    let offset = msg.offset as usize;
                    let page: Vec<FsEntry> = entries
                        .into_iter()
                        .skip(offset)
                        .take(limit)
                        .map(|e| FsEntry {
                            name: e.name,
                            path: e.path.to_string_lossy().into_owned(),
                            is_dir: e.is_dir,
                            size: e.size,
                        })
                        .collect();
                    let has_more = (offset + page.len()) < total as usize;
                    let _ = msg
                        .tx
                        .send(FsListResult {
                            entries: page,
                            total,
                            has_more,
                        })
                        .await;
                }
                Err(_) => drop(msg.tx),
            }
        }

        ZedraMessage::FsRead(msg) => {
            session.rpc_fs_reads.fetch_add(1, Ordering::Relaxed);
            let path = match resolve_path(&state.workdir, &msg.path) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!("FsRead: rejected path {:?}: {}", msg.path, e);
                    drop(msg.tx);
                    return Ok(());
                }
            };
            const MAX_FILE_SIZE: u64 = 500 * 1024;
            if std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) > MAX_FILE_SIZE {
                let _ = msg
                    .tx
                    .send(FsReadResult {
                        content: String::new(),
                        too_large: true,
                    })
                    .await;
                return Ok(());
            }
            match state.fs.read(&path) {
                Ok(content) => {
                    let _ = msg
                        .tx
                        .send(FsReadResult {
                            content,
                            too_large: false,
                        })
                        .await;
                }
                Err(_) => drop(msg.tx),
            }
        }

        ZedraMessage::FsWrite(msg) => {
            session.rpc_fs_writes.fetch_add(1, Ordering::Relaxed);
            let path = match resolve_path(&state.workdir, &msg.path) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!("FsWrite: rejected path {:?}: {}", msg.path, e);
                    let _ = msg.tx.send(FsWriteResult { ok: false }).await;
                    return Ok(());
                }
            };
            let ok = state.fs.write(&path, &msg.content).is_ok();
            let _ = msg.tx.send(FsWriteResult { ok }).await;
        }

        ZedraMessage::FsStat(msg) => {
            let path = match resolve_path(&state.workdir, &msg.path) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!("FsStat: rejected path {:?}: {}", msg.path, e);
                    drop(msg.tx);
                    return Ok(());
                }
            };
            match state.fs.stat(&path) {
                Ok(stat) => {
                    let _ = msg
                        .tx
                        .send(FsStatResult {
                            path: stat.path.to_string_lossy().into_owned(),
                            is_dir: stat.is_dir,
                            size: stat.size,
                            modified: stat.modified,
                        })
                        .await;
                }
                Err(_) => drop(msg.tx),
            }
        }

        ZedraMessage::FsWatch(msg) => {
            if !session.allow_fs_watch_rpc().await {
                session
                    .fs_watch_rate_limited
                    .fetch_add(1, Ordering::Relaxed);
                tracing::warn!("FsWatch rate limited: session={}", session.id);
                let _ = msg.tx.send(FsWatchResult::RateLimited).await;
                return Ok(());
            }
            let result = match normalize_observer_path(&msg.path) {
                Some(path) => {
                    if session.try_add_watched_path(path).await {
                        FsWatchResult::Ok
                    } else {
                        FsWatchResult::QuotaExceeded
                    }
                }
                None => FsWatchResult::InvalidPath,
            };
            if !matches!(result, FsWatchResult::Ok) {
                tracing::warn!(
                    "FsWatch rejected: session={} path={:?} quota={} max_watched_paths={}",
                    session.id,
                    msg.path,
                    session.fs_watch_quota_rejected.load(Ordering::Relaxed),
                    MAX_WATCHED_PATHS_PER_SESSION
                );
            }
            let _ = msg.tx.send(result).await;
        }

        ZedraMessage::FsUnwatch(msg) => {
            if !session.allow_fs_watch_rpc().await {
                session
                    .fs_watch_rate_limited
                    .fetch_add(1, Ordering::Relaxed);
                tracing::warn!("FsUnwatch rate limited: session={}", session.id);
                let _ = msg.tx.send(FsUnwatchResult::RateLimited).await;
                return Ok(());
            }
            let result = match normalize_observer_path(&msg.path) {
                Some(path) => {
                    if session.remove_watched_path(&path).await {
                        FsUnwatchResult::Ok
                    } else {
                        FsUnwatchResult::NotWatched
                    }
                }
                None => FsUnwatchResult::InvalidPath,
            };
            let _ = msg.tx.send(result).await;
        }

        // -- Terminal --
        ZedraMessage::TermCreate(msg) => {
            session.touch().await;
            let workdir = session
                .workdir
                .clone()
                .or_else(|| Some(state.workdir.clone()));
            let has_launch_cmd = msg.launch_cmd.is_some();
            let launch_cmd = msg.launch_cmd.clone();
            match create_terminal(
                &session,
                msg.cols,
                msg.rows,
                SpawnOptions {
                    workdir,
                    launch_cmd,
                },
            )
            .await
            {
                Ok(id) => {
                    zedra_telemetry::send(Event::HostTerminalOpen { has_launch_cmd });
                    let _ = msg.tx.send(TermCreateResult { id }).await;
                }
                Err(e) => {
                    tracing::warn!("TermCreate failed: {}", e);
                    drop(msg.tx);
                }
            }
        }

        ZedraMessage::Subscribe(msg) => {
            session.touch().await;
            // Bridge: store a regular tokio sender in the session; spawn a task
            // that forwards events from it to the irpc channel toward the client.
            let (bridge_tx, mut bridge_rx) = tokio::sync::mpsc::channel::<HostEvent>(32);
            *session.event_tx.lock().await = Some(bridge_tx);
            let irpc_tx = msg.tx;
            {
                let mut watched = session.fs_watched_paths.lock().await;
                watched.insert(".".to_string());
            }
            let my_gen = session.observer_gen.fetch_add(1, Ordering::AcqRel) + 1;
            let observer_session = session.clone();
            let observer_workdir = state.workdir.clone();
            tokio::spawn(async move {
                run_observer(observer_session, observer_workdir, my_gen).await;
            });
            tokio::spawn(async move {
                while let Some(event) = bridge_rx.recv().await {
                    if irpc_tx.send(event).await.is_err() {
                        break;
                    }
                }
            });
        }

        ZedraMessage::TermAttach(msg) => {
            session.touch().await;

            let term_id = msg.id.clone();
            let last_seq = msg.last_seq;
            let irpc_tx = msg.tx;
            let mut irpc_rx = msg.rx;

            {
                let terms = session.terminals.lock().await;
                if !terms.contains_key(&term_id) {
                    tracing::warn!("TermAttach: unknown terminal {}", term_id);
                    return Ok(());
                }
            }

            // Synthetic metadata preamble (seq=0): inject cached title/CWD so
            // the client seeds TerminalMeta even when those OSC sequences were
            // evicted from the backlog. seq=0 is a reserved marker; the client
            // pump processes its data but skips seq tracking and gap detection.
            // Extract the preamble bytes while holding the sync lock, then send
            // after releasing it to avoid holding a MutexGuard across an await.
            let preamble: Option<Vec<u8>> = {
                let terms = session.terminals.lock().await;
                terms.get(&term_id).and_then(|term| {
                    term.host_meta.lock().ok().and_then(|meta| {
                        let p = encode_meta_preamble(&meta.title, &meta.cwd);
                        if p.is_empty() {
                            None
                        } else {
                            Some(p)
                        }
                    })
                })
            };
            if let Some(p) = preamble {
                tracing::debug!(
                    "TermAttach: sending meta preamble ({} bytes) for {}",
                    p.len(),
                    term_id
                );
                if irpc_tx.send(TermOutput { data: p, seq: 0 }).await.is_err() {
                    return Ok(());
                }
            }

            // Replay backlog
            let backlog = session.backlog_after(&term_id, last_seq).await;
            tracing::info!(
                "TermAttach: id={} last_seq={} backlog_entries={} session={}",
                term_id,
                last_seq,
                backlog.len(),
                session.id,
            );
            for entry in backlog {
                if irpc_tx
                    .send(TermOutput {
                        data: entry.data,
                        seq: entry.seq,
                    })
                    .await
                    .is_err()
                {
                    return Ok(());
                }
            }

            // Extract the writer Arc at setup so the input loop can write
            // directly without re-acquiring session.terminals on every keystroke
            // (Fix 3). The Arc stays valid even if the terminal is removed from
            // the map; writes will simply fail harmlessly against the closed PTY.
            let pty_writer = {
                let terms = session.terminals.lock().await;
                terms.get(&term_id).map(|t| t.writer.clone())
            };
            let Some(pty_writer) = pty_writer else {
                tracing::warn!(
                    "TermAttach: terminal {} vanished before writer extract",
                    term_id
                );
                return Ok(());
            };

            // Set up bridge.
            // Capture the generation we install so cleanup can guard against
            // clobbering a sender installed by a concurrent newer TermAttach.
            let (bridge_tx, mut bridge_rx) = tokio::sync::mpsc::channel::<TermOutput>(256);
            let my_sender_gen: u64 = {
                let terms = session.terminals.lock().await;
                if let Some(term) = terms.get(&term_id) {
                    let mut slot = term.output_sender.lock().unwrap();
                    slot.gen = slot.gen.wrapping_add(1);
                    slot.sender = Some(bridge_tx);
                    slot.gen
                } else {
                    0
                }
            };

            // Separate output task so slow relay sends don't block input processing.
            // With high-latency connections (e.g. relay RTT ~300ms), irpc_tx.send().await
            // can stall waiting for QUIC flow control acks. If input and output share a
            // single select! loop, that stall prevents keystrokes from reaching the PTY.
            let output_task = tokio::spawn(async move {
                while let Some(mut term_output) = bridge_rx.recv().await {
                    // Coalesce any chunks that arrived while the previous send was in
                    // flight. Under relay congestion the channel can accumulate many
                    // small PTY reads; merging them reduces irpc framing overhead and
                    // the number of QUIC stream writes without adding any extra delay
                    // for interactive typing (single-byte keystrokes never accumulate).
                    while let Ok(next) = bridge_rx.try_recv() {
                        term_output.data.extend_from_slice(&next.data);
                        term_output.seq = next.seq;
                    }
                    if irpc_tx.send(term_output).await.is_err() {
                        break;
                    }
                }
            });

            loop {
                match irpc_rx.recv().await {
                    Ok(Some(term_input)) => {
                        // Write directly via the pre-captured writer Arc —
                        // no session.terminals lock needed per keystroke (Fix 3).
                        if let Ok(mut w) = pty_writer.lock() {
                            let _ = w.write_all(&term_input.data);
                            let _ = w.flush();
                        }
                    }
                    Ok(None) | Err(_) => break,
                }
            }

            output_task.abort();

            // Only clear output_sender if it still belongs to this TermAttach.
            // A concurrent newer TermAttach may have already replaced the sender;
            // clearing it unconditionally would silence that client's output.
            {
                let terms = session.terminals.lock().await;
                if let Some(term) = terms.get(&term_id) {
                    let mut slot = term.output_sender.lock().unwrap();
                    if slot.gen == my_sender_gen {
                        slot.sender = None;
                    }
                }
            }
        }

        ZedraMessage::TermResize(msg) => {
            let terms = session.terminals.lock().await;
            let ok = if let Some(term) = terms.get(&msg.id) {
                term.master
                    .resize(portable_pty::PtySize {
                        rows: msg.rows,
                        cols: msg.cols,
                        pixel_width: 0,
                        pixel_height: 0,
                    })
                    .is_ok()
            } else {
                false
            };
            let _ = msg.tx.send(TermResizeResult { ok }).await;
        }

        ZedraMessage::TermClose(msg) => {
            session.terminals.lock().await.remove(&msg.id);
            let _ = msg.tx.send(TermCloseResult { ok: true }).await;
        }

        ZedraMessage::TermList(msg) => {
            let terms = session.terminals.lock().await;
            let terminals = terms
                .keys()
                .map(|id| TermListEntry { id: id.clone() })
                .collect();
            let _ = msg.tx.send(TermListResult { terminals }).await;
        }

        // -- Git --
        ZedraMessage::GitStatus(msg) => {
            session.rpc_git_ops.fetch_add(1, Ordering::Relaxed);
            match GitRepo::open(&state.workdir) {
                Ok(repo) => {
                    let branch = repo.branch().unwrap_or_default();
                    let entries = repo
                        .status()
                        .unwrap_or_default()
                        .into_iter()
                        .map(|e| GitStatusEntry {
                            path: e.path,
                            staged_status: e
                                .staged_status
                                .map(|status| format!("{:?}", status).to_lowercase()),
                            unstaged_status: e
                                .unstaged_status
                                .map(|status| format!("{:?}", status).to_lowercase()),
                        })
                        .collect();
                    let _ = msg.tx.send(GitStatusResult { branch, entries }).await;
                }
                Err(_) => drop(msg.tx),
            }
        }

        ZedraMessage::GitDiff(msg) => {
            session.rpc_git_ops.fetch_add(1, Ordering::Relaxed);
            match GitRepo::open(&state.workdir) {
                Ok(repo) => {
                    let diff = repo
                        .diff(msg.path.as_deref(), msg.staged)
                        .unwrap_or_default();
                    let _ = msg.tx.send(GitDiffResult { diff }).await;
                }
                Err(_) => drop(msg.tx),
            }
        }

        ZedraMessage::GitLog(msg) => {
            session.rpc_git_ops.fetch_add(1, Ordering::Relaxed);
            match GitRepo::open(&state.workdir) {
                Ok(repo) => {
                    let entries = repo
                        .log(msg.limit.unwrap_or(20).min(500))
                        .unwrap_or_default()
                        .into_iter()
                        .map(|e| GitLogEntry {
                            id: e.id,
                            message: e.message,
                            author: e.author,
                            timestamp: e.timestamp,
                        })
                        .collect();
                    let _ = msg.tx.send(GitLogResult { entries }).await;
                }
                Err(_) => drop(msg.tx),
            }
        }

        ZedraMessage::GitCommit(msg) => {
            let files_staged = msg.paths.len();
            match GitRepo::open(&state.workdir) {
                Ok(repo) => match repo.commit(&msg.message, &msg.paths) {
                    Ok(hash) => {
                        session.rpc_git_commits.fetch_add(1, Ordering::Relaxed);
                        zedra_telemetry::send(Event::GitCommitMade {
                            files_staged,
                            success: true,
                        });
                        let _ = msg.tx.send(GitCommitResult { hash }).await;
                    }
                    Err(_) => {
                        zedra_telemetry::send(Event::GitCommitMade {
                            files_staged,
                            success: false,
                        });
                        drop(msg.tx);
                    }
                },
                Err(_) => {
                    zedra_telemetry::send(Event::GitCommitMade {
                        files_staged,
                        success: false,
                    });
                    drop(msg.tx);
                }
            }
        }

        ZedraMessage::GitStage(msg) => {
            session.rpc_git_ops.fetch_add(1, Ordering::Relaxed);
            match GitRepo::open(&state.workdir) {
                Ok(repo) => match repo.stage(&msg.paths) {
                    Ok(()) => {
                        let _ = msg.tx.send(GitStageResult {}).await;
                    }
                    Err(_) => drop(msg.tx),
                },
                Err(_) => drop(msg.tx),
            }
        }

        ZedraMessage::GitUnstage(msg) => {
            session.rpc_git_ops.fetch_add(1, Ordering::Relaxed);
            match GitRepo::open(&state.workdir) {
                Ok(repo) => match repo.unstage(&msg.paths) {
                    Ok(()) => {
                        let _ = msg.tx.send(GitUnstageResult {}).await;
                    }
                    Err(_) => drop(msg.tx),
                },
                Err(_) => drop(msg.tx),
            }
        }

        ZedraMessage::GitBranches(msg) => {
            session.rpc_git_ops.fetch_add(1, Ordering::Relaxed);
            match GitRepo::open(&state.workdir) {
                Ok(repo) => {
                    let branches = repo
                        .branches()
                        .unwrap_or_default()
                        .into_iter()
                        .map(|b| GitBranchEntry {
                            name: b.name,
                            is_head: b.is_head,
                        })
                        .collect();
                    let _ = msg.tx.send(GitBranchesResult { branches }).await;
                }
                Err(_) => drop(msg.tx),
            }
        }

        ZedraMessage::GitCheckout(msg) => {
            let ok = GitRepo::open(&state.workdir)
                .and_then(|repo| repo.checkout(&msg.branch))
                .is_ok();
            let _ = msg.tx.send(GitCheckoutResult { ok }).await;
        }

        // -- AI --
        ZedraMessage::AiPrompt(msg) => {
            // Resolve the Claude binary path. Prefer an explicit absolute path
            // from the environment to avoid executing a malicious `claude` binary
            // that might appear earlier in $PATH.
            let claude_bin =
                std::env::var("ZEDRA_CLAUDE_BIN").unwrap_or_else(|_| "claude".to_string());
            let prompt_bytes = msg.prompt.len();
            let ai_start = std::time::Instant::now();
            let output = std::process::Command::new(&claude_bin)
                .args(["--print", &msg.prompt])
                .current_dir(&state.workdir)
                .output();
            let duration_ms = ai_start.elapsed().as_millis() as u64;

            let (text, done, success) = match output {
                Ok(out) if out.status.success() => {
                    (String::from_utf8_lossy(&out.stdout).into_owned(), true, true)
                }
                Ok(out) => {
                    let err = String::from_utf8_lossy(&out.stderr).into_owned();
                    (format!("Error: {}", err), true, false)
                }
                Err(_) => (
                    format!(
                        "[Claude Code not found on host. Install with: npm i -g @anthropic-ai/claude-code]\n\nPrompt was: {}",
                        msg.prompt
                    ),
                    true,
                    false,
                ),
            };
            session.rpc_ai_prompts.fetch_add(1, Ordering::Relaxed);
            zedra_telemetry::send(Event::AiPromptSent {
                success,
                duration_ms,
                prompt_bytes,
                response_bytes: text.len(),
            });
            let _ = msg.tx.send(AiPromptResult { text, done }).await;
        }

        // -- LSP --
        ZedraMessage::LspDiagnostics(msg) => {
            let full_path = match resolve_path(&state.workdir, &msg.path) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!("LspDiagnostics: rejected path {:?}: {}", msg.path, e);
                    drop(msg.tx);
                    return Ok(());
                }
            };
            let diagnostics = run_lsp_check(&full_path)
                .into_iter()
                .map(|d| LspDiagnostic {
                    message: d.message,
                    severity: d.severity,
                })
                .collect();
            let _ = msg.tx.send(LspDiagnosticsResult { diagnostics }).await;
        }

        ZedraMessage::LspHover(msg) => {
            let _ = msg
                .tx
                .send(LspHoverResult {
                    contents: "LSP hover not yet connected to a language server.".to_string(),
                })
                .await;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct DiagnosticEntry {
    message: String,
    severity: String,
}

fn run_lsp_check(path: &std::path::Path) -> Vec<DiagnosticEntry> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

    let (cmd, args): (&str, Vec<&str>) = match ext {
        "rs" => ("cargo", vec!["check", "--message-format=json"]),
        "ts" | "tsx" | "js" | "jsx" => ("npx", vec!["tsc", "--noEmit"]),
        "py" => (
            "python3",
            vec!["-m", "py_compile", path.to_str().unwrap_or("")],
        ),
        _ => return vec![],
    };

    let output = std::process::Command::new(cmd)
        .args(&args)
        .current_dir(path.parent().unwrap_or(std::path::Path::new(".")))
        .output();

    match output {
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            if stderr.is_empty() && out.status.success() {
                vec![]
            } else {
                stderr
                    .lines()
                    .take(10)
                    .filter(|l| !l.is_empty())
                    .map(|line| DiagnosticEntry {
                        message: line.to_string(),
                        severity: "error".into(),
                    })
                    .collect()
            }
        }
        Err(_) => vec![],
    }
}

fn os_version_string() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        if let Ok(content) = std::fs::read_to_string("/etc/os-release") {
            for line in content.lines() {
                if let Some(pretty) = line.strip_prefix("PRETTY_NAME=") {
                    return Some(pretty.trim_matches('"').to_string());
                }
            }
        }
        let output = std::process::Command::new("uname")
            .arg("-r")
            .output()
            .ok()?;
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("sw_vers")
            .arg("-productVersion")
            .output()
            .ok()?;
        Some(format!(
            "macOS {}",
            String::from_utf8_lossy(&output.stdout).trim()
        ))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}
