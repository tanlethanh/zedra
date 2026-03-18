// RPC daemon: exposes filesystem, git, terminal, LSP, and AI operations over irpc.
//
// Connection lifecycle (Phase 1 PKI):
//   First pairing:  Register → Authenticate → AuthProve → (RPC calls)
//   Reconnect:      Authenticate → AuthProve → (RPC calls)
//   Health:         Ping (every 2s, foreground only, 5 misses = client reconnects)

use crate::fs::{Filesystem, LocalFs};
use crate::git::GitRepo;
use crate::identity::SharedIdentity;
use crate::pty::{ShellSession, SpawnOptions};
use crate::session_registry::{AttachResult, ConsumeSlotResult, OutputSenderSlot, ServerSession, SessionRegistry, TermSession};
use anyhow::Result;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use zedra_rpc::proto::*;

fn ts() -> String {
    let s = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    format!("{:02}:{:02}:{:02}", (s % 86400) / 3600, (s % 3600) / 60, s % 60)
}

fn short_key(key: &[u8; 32]) -> String {
    key[..4].iter().map(|b| format!("{b:02x}")).collect()
}

/// Resolve `user_path` relative to `workdir`, then verify the canonical path
/// stays inside `workdir`. Rejects absolute paths, `..` escapes, and symlinks
/// that point outside the jail.
fn resolve_path(workdir: &Path, user_path: &str) -> Result<PathBuf> {
    // Reject empty paths
    anyhow::ensure!(!user_path.is_empty(), "empty path");
    let joined = workdir.join(user_path);
    let resolved = joined
        .canonicalize()
        .or_else(|_| {
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

/// Shared state for RPC handlers.
pub struct DaemonState {
    pub fs: Arc<dyn Filesystem>,
    pub workdir: std::path::PathBuf,
    /// Host identity for signing challenges in the Authenticate step.
    pub identity: SharedIdentity,
    /// Default command injected into every new terminal on startup (e.g. "claude --resume").
    /// Can be overridden per-terminal via `TermCreateReq::launch_cmd`.
    pub default_launch_cmd: Option<String>,
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
            default_launch_cmd: None,
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

    // Auth phase: returns (session, client_pubkey) or closes connection
    let (session, client_pubkey) =
        match auth_phase(&conn, &registry, &state.identity, &state.workdir).await {
            Ok(pair) => pair,
            Err(e) => {
                tracing::warn!("auth failed from {}: {}", remote.fmt_short(), e);
                // Wait for the client to close the connection (up to 500ms) so any
                // error response we sent has time to be delivered before CONNECTION_CLOSE.
                let _ = tokio::time::timeout(
                    std::time::Duration::from_millis(500),
                    conn.closed(),
                )
                .await;
                return Ok(());
            }
        };

    tracing::info!(
        "Authenticated client {:?}... → session={}",
        &client_pubkey[..4],
        session.id,
    );
    eprintln!("[{}] connected: {} → session {}", ts(), short_key(&client_pubkey), &session.id[..8.min(session.id.len())]);

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
    registry.detach_client(&session.id, client_pubkey).await;

    tracing::info!(
        "Connection closed: session={} (session stays alive in registry)",
        session.id,
    );
    eprintln!("[{}] disconn:   {} (session {})", ts(), short_key(&client_pubkey), &session.id[..8.min(session.id.len())]);
    Ok(())
}

// ---------------------------------------------------------------------------
// Auth phase
// ---------------------------------------------------------------------------

/// Perform the full auth handshake for a new connection.
///
/// Flow:
///   1. Optional Register (first-time only, proves QR possession via HMAC)
///   2. Authenticate (get nonce + host signature from us)
///   3. AuthProve (client signs nonce, specifies session to attach)
async fn auth_phase(
    conn: &iroh::endpoint::Connection,
    registry: &Arc<SessionRegistry>,
    identity: &SharedIdentity,
    workdir: &std::path::Path,
) -> Result<(Arc<ServerSession>, [u8; 32])> {
    // Step 1: Optional Register
    let first = irpc_iroh::read_request::<ZedraProto>(conn).await?;

    let client_pubkey: [u8; 32] = match first {
        Some(ZedraMessage::Register(msg)) => {
            let pubkey = msg.client_pubkey;
            let result = handle_register(&msg, registry).await;
            let ok = matches!(result, RegisterResult::Ok);
            let _ = msg.tx.send(result).await;
            if !ok {
                anyhow::bail!("register rejected");
            }
            // Now expect Authenticate
            pubkey
        }
        Some(ZedraMessage::Authenticate(msg)) => {
            // Reconnect path: skip register, issue challenge directly
            let pubkey = msg.client_pubkey;
            // Check global auth first
            if !registry.is_globally_authorized(&pubkey).await {
                drop(msg.tx); // signal error by dropping
                anyhow::bail!("client not authorized");
            }
            let nonce = issue_challenge(msg.tx, identity).await?;
            return finish_auth(conn, registry, pubkey, nonce, workdir).await;
        }
        _ => anyhow::bail!("expected Register or Authenticate as first message"),
    };

    // After Register: expect Authenticate
    let auth_msg = irpc_iroh::read_request::<ZedraProto>(conn).await?;
    match auth_msg {
        Some(ZedraMessage::Authenticate(msg)) => {
            // After fresh registration, client is authorized
            let nonce = issue_challenge(msg.tx, identity).await?;
            finish_auth(conn, registry, client_pubkey, nonce, workdir).await
        }
        _ => anyhow::bail!("expected Authenticate after Register"),
    }
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
        tracing::warn!("Register: stale timestamp (now={}, ts={})", now, msg.timestamp);
        return RegisterResult::StaleTimestamp;
    }

    // Atomically consume the pairing slot
    match registry.consume_pairing_slot(&msg.slot_session_id).await {
        ConsumeSlotResult::Active(slot) => {
            // Verify HMAC (slot is already consumed regardless of outcome)
            if !zedra_rpc::verify_registration_hmac(
                &slot.handshake_secret,
                &msg.client_pubkey,
                msg.timestamp,
                &msg.hmac,
            ) {
                tracing::warn!("Register: invalid HMAC from {:?}...", &msg.client_pubkey[..4]);
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
            eprintln!("[{}] paired:    {} → session {}", ts(), short_key(&msg.client_pubkey), &slot.session_id[..8.min(slot.session_id.len())]);
            RegisterResult::Ok
        }
        ConsumeSlotResult::Consumed => {
            tracing::warn!("Register: slot for {} already consumed", msg.slot_session_id);
            RegisterResult::HandshakeConsumed
        }
        ConsumeSlotResult::NotFound => {
            tracing::warn!("Register: no slot found for session {}", msg.slot_session_id);
            RegisterResult::SlotNotFound
        }
    }
}

/// Generate a fresh nonce, sign it with the host key, send to client.
/// Returns the nonce for later verification in AuthProve.
async fn issue_challenge(
    tx: irpc::channel::oneshot::Sender<AuthChallengeResult>,
    identity: &SharedIdentity,
) -> Result<[u8; 32]> {
    let mut nonce = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut nonce);
    let host_signature = identity.sign_challenge(&nonce);
    let _ = tx.send(AuthChallengeResult { nonce, host_signature }).await;
    Ok(nonce)
}

/// Read AuthProve, verify client signature, attach to session.
async fn finish_auth(
    conn: &iroh::endpoint::Connection,
    registry: &Arc<SessionRegistry>,
    client_pubkey: [u8; 32],
    nonce: [u8; 32],
    workdir: &std::path::Path,
) -> Result<(Arc<ServerSession>, [u8; 32])> {
    let prove_msg = irpc_iroh::read_request::<ZedraProto>(conn).await?;

    let msg = match prove_msg {
        Some(ZedraMessage::AuthProve(m)) => m,
        _ => anyhow::bail!("expected AuthProve"),
    };

    // Extract fields before any moves
    let prove_nonce = msg.nonce;
    let prove_sig = msg.client_signature;
    let session_id = msg.session_id.clone();
    let tx = msg.tx;

    // Verify nonce echo
    if prove_nonce != nonce {
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
                let fallback = if let Some(s) =
                    registry.find_session_for_client(&client_pubkey).await
                {
                    tracing::info!(
                        "finish_auth: session {} gone, falling back to session {}",
                        session_id,
                        s.id,
                    );
                    s
                } else {
                    // No existing session — create a fresh default one.
                    let name = workdir
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("default");
                    let s = registry.create_named(name, workdir.to_path_buf()).await;
                    registry
                        .add_client_to_session(&s.id, client_pubkey)
                        .await;
                    tracing::info!(
                        "finish_auth: session {} gone, created new session {} ({})",
                        session_id,
                        s.id,
                        name,
                    );
                    s
                };
                let new_id = fallback.id.clone();
                (
                    registry.attach_client(&new_id, client_pubkey).await,
                    new_id,
                )
            }
            other => (other, session_id.clone()),
        };

    match attach_result {
        AttachResult::Ok => {
            let Some(session) = registry.get(&resolved_session_id).await else {
                let _ = tx.send(AuthProveResult::SessionNotFound).await;
                anyhow::bail!("session {} vanished after attach", resolved_session_id);
            };
            let _ = tx.send(AuthProveResult::Ok).await;
            Ok((session, client_pubkey))
        }
        AttachResult::SessionNotFound => {
            let _ = tx.send(AuthProveResult::SessionNotFound).await;
            anyhow::bail!("session {} not found", resolved_session_id)
        }
        AttachResult::NotInSessionAcl => {
            let _ = tx.send(AuthProveResult::NotInSessionAcl).await;
            anyhow::bail!("client not in session ACL")
        }
        AttachResult::SessionOccupied => {
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
            session.id, MAX_TERMINALS_PER_SESSION, MAX_TERMINALS_PER_SESSION,
        );
    }

    let shell = ShellSession::spawn(cols, rows, opts)?;
    let (pty_reader, pty_writer, master) = shell.take_reader();
    let id = session.next_terminal_id().await;

    tracing::info!(
        "create_terminal: id={} cols={} rows={} session={}",
        id, cols, rows, session.id,
    );

    let output_sender = Arc::new(std::sync::Mutex::new(
        OutputSenderSlot { gen: 0, sender: None },
    ));

    session.terminals.lock().await.insert(
        id.clone(),
        TermSession {
            writer: pty_writer,
            master,
            output_sender: output_sender.clone(),
        },
    );

    let term_id = id.clone();
    let sess_for_reader = session.clone();
    tokio::task::spawn_blocking(move || {
        let mut reader = pty_reader;
        let mut buf = [0u8; 8192];
        let rt = tokio::runtime::Handle::current();
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let data = buf[..n].to_vec();
                    let seq = rt.block_on(async {
                        let s = sess_for_reader.next_backlog_seq().await;
                        sess_for_reader.push_backlog_entry(BacklogEntry {
                            seq: s,
                            terminal_id: term_id.clone(),
                            data: data.clone(),
                        }).await;
                        s
                    });

                    let sender: Option<tokio::sync::mpsc::Sender<TermOutput>> =
                        output_sender.lock().unwrap().sender.clone();
                    if let Some(tx) = sender {
                        if rt.block_on(tx.send(TermOutput { data, seq })).is_err() {
                            output_sender.lock().unwrap().sender = None;
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
        // -- Auth (should not appear in dispatch loop) --
        ZedraMessage::Register(_) | ZedraMessage::Authenticate(_) | ZedraMessage::AuthProve(_) => {
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
            let hostname = hostname::get()
                .ok()
                .and_then(|h| h.into_string().ok())
                .unwrap_or_else(|| "unknown".to_string());
            let username = std::env::var("USER").unwrap_or_else(|_| "unknown".to_string());
            let workdir = state.workdir.to_string_lossy().into_owned();
            let home_dir = std::env::var("HOME").ok();
            let _ = msg.tx.send(SessionInfoResult {
                hostname,
                workdir,
                username,
                home_dir,
                session_id: Some(session.id.clone()),
                os: Some(std::env::consts::OS.to_string()),
                arch: Some(std::env::consts::ARCH.to_string()),
                os_version: os_version_string(),
                host_version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }).await;
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
                Some(target)
                    if target.acl.lock().await.contains(&client_pubkey) =>
                {
                    target.touch().await;
                    let workdir = target
                        .workdir
                        .as_ref()
                        .map(|p| p.to_string_lossy().into_owned());
                    let _ = msg.tx.send(SessionSwitchResult {
                        session_id: target.id.clone(),
                        workdir,
                    }).await;
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
                    let entries = entries
                        .into_iter()
                        .map(|e| FsEntry {
                            name: e.name,
                            path: e.path.to_string_lossy().into_owned(),
                            is_dir: e.is_dir,
                            size: e.size,
                        })
                        .collect();
                    let _ = msg.tx.send(FsListResult { entries }).await;
                }
                Err(_) => drop(msg.tx),
            }
        }

        ZedraMessage::FsRead(msg) => {
            let path = match resolve_path(&state.workdir, &msg.path) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!("FsRead: rejected path {:?}: {}", msg.path, e);
                    drop(msg.tx);
                    return Ok(());
                }
            };
            match state.fs.read(&path) {
                Ok(content) => {
                    let _ = msg.tx.send(FsReadResult { content }).await;
                }
                Err(_) => drop(msg.tx),
            }
        }

        ZedraMessage::FsWrite(msg) => {
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
                    let _ = msg.tx.send(FsStatResult {
                        path: stat.path.to_string_lossy().into_owned(),
                        is_dir: stat.is_dir,
                        size: stat.size,
                        modified: stat.modified,
                    }).await;
                }
                Err(_) => drop(msg.tx),
            }
        }

        // -- Terminal --
        ZedraMessage::TermCreate(msg) => {
            session.touch().await;
            let workdir = session.workdir.clone().or_else(|| Some(state.workdir.clone()));
            let launch_cmd = msg.launch_cmd.clone().or_else(|| state.default_launch_cmd.clone());
            match create_terminal(&session, msg.cols, msg.rows, SpawnOptions { workdir, launch_cmd }).await {
                Ok(id) => { let _ = msg.tx.send(TermCreateResult { id }).await; }
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

            // Replay backlog
            let backlog = session.backlog_after(&term_id, last_seq).await;
            tracing::info!(
                "TermAttach: id={} last_seq={} backlog_entries={} session={}",
                term_id, last_seq, backlog.len(), session.id,
            );
            for entry in backlog {
                if irpc_tx
                    .send(TermOutput { data: entry.data, seq: entry.seq })
                    .await
                    .is_err()
                {
                    return Ok(());
                }
            }

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

            let session_for_input = session.clone();
            let term_id_for_input = term_id.clone();

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
                        let mut terms = session_for_input.terminals.lock().await;
                        if let Some(term) = terms.get_mut(&term_id_for_input) {
                            let _ = term.writer.write_all(&term_input.data);
                            let _ = term.writer.flush();
                        } else {
                            break;
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
            match GitRepo::open(&state.workdir) {
                Ok(repo) => {
                    let branch = repo.branch().unwrap_or_default();
                    let entries = repo
                        .status()
                        .unwrap_or_default()
                        .into_iter()
                        .map(|e| GitStatusEntry {
                            path: e.path,
                            status: format!("{:?}", e.status).to_lowercase(),
                        })
                        .collect();
                    let _ = msg.tx.send(GitStatusResult { branch, entries }).await;
                }
                Err(_) => drop(msg.tx),
            }
        }

        ZedraMessage::GitDiff(msg) => {
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
            match GitRepo::open(&state.workdir) {
                Ok(repo) => match repo.commit(&msg.message, &msg.paths) {
                    Ok(hash) => {
                        let _ = msg.tx.send(GitCommitResult { hash }).await;
                    }
                    Err(_) => drop(msg.tx),
                },
                Err(_) => drop(msg.tx),
            }
        }

        ZedraMessage::GitBranches(msg) => {
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
            let claude_bin = std::env::var("ZEDRA_CLAUDE_BIN")
                .unwrap_or_else(|_| "claude".to_string());
            let output = std::process::Command::new(&claude_bin)
                .args(["--print", &msg.prompt])
                .current_dir(&state.workdir)
                .output();

            let (text, done) = match output {
                Ok(out) if out.status.success() => {
                    (String::from_utf8_lossy(&out.stdout).into_owned(), true)
                }
                Ok(out) => {
                    let err = String::from_utf8_lossy(&out.stderr).into_owned();
                    (format!("Error: {}", err), true)
                }
                Err(_) => (
                    format!(
                        "[Claude Code not found on host. Install with: npm i -g @anthropic-ai/claude-code]\n\nPrompt was: {}",
                        msg.prompt
                    ),
                    true,
                ),
            };
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
            let _ = msg.tx.send(LspHoverResult {
                contents: "LSP hover not yet connected to a language server.".to_string(),
            }).await;
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
