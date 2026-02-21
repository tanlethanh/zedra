// RPC daemon: exposes filesystem, git, terminal, LSP, and AI operations over JSON-RPC.
//
// Mobile clients connect via iroh (QUIC/TLS 1.3) and issue JSON-RPC requests
// for file browsing, editing, git operations, terminal sessions, LSP queries,
// and Claude Code AI integration.
//
// Architecture: manual dispatch loop with shared writer channel for bidirectional
// notifications. Terminal output is streamed as `terminal/output` notifications
// from a blocking PTY reader task through the shared writer.

use anyhow::Result;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::Arc;
use crate::fs::{Filesystem, LocalFs};
use crate::git::GitRepo;
use crate::identity::SharedIdentity;
use zedra_rpc::methods;
use zedra_rpc::{FsListParams, FsReadParams, FsStatParams, FsWriteParams};
use zedra_rpc::{GitCommitParams, GitDiffParams, GitLogParams};
use zedra_rpc::{SessionAttachParams, SessionResumeParams, TermCreateParams, TermDataParams, TermResizeParams};
use zedra_rpc::Transport;

use crate::pty::ShellSession;
use crate::session_registry::{ServerSession, SessionRegistry, TermSession as SessionTermSession};

/// Handler function type: takes params, returns result or error.
type HandlerFn = Arc<
    dyn Fn(serde_json::Value) -> futures::future::BoxFuture<'static, Result<serde_json::Value>>
        + Send
        + Sync,
>;

/// Shared state for RPC handlers.
pub struct DaemonState {
    pub fs: Arc<dyn Filesystem>,
    pub workdir: std::path::PathBuf,
    /// Host identity (used by iroh listener for endpoint key).
    pub identity: Option<SharedIdentity>,
}

impl DaemonState {
    pub fn new(workdir: std::path::PathBuf) -> Self {
        Self {
            fs: Arc::new(LocalFs),
            workdir,
            identity: None,
        }
    }

    pub fn with_identity(mut self, identity: SharedIdentity) -> Self {
        self.identity = Some(identity);
        self
    }
}

/// Read the first application-level message and bind the session before
/// entering the dispatch loop. This ensures handlers always have a bound
/// session — no window of unbound state during which replayed RPCs could
/// arrive before the session is set.
///
/// Returns the bound session and optionally the first message's raw bytes
/// if it wasn't a session-management message (needs replay through dispatch).
async fn bind_session(
    recv_rx: &mut tokio::sync::mpsc::Receiver<Vec<u8>>,
    write_tx: &tokio::sync::mpsc::Sender<Vec<u8>>,
    registry: &SessionRegistry,
) -> Result<(Arc<ServerSession>, Option<Vec<u8>>)> {
    let payload = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        recv_rx.recv(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("timed out waiting for first message"))?
    .ok_or_else(|| anyhow::anyhow!("connection closed before first message"))?;

    let msg: zedra_rpc::Message = match serde_json::from_slice(&payload) {
        Ok(m) => m,
        Err(_) => {
            let session = registry.resume_or_create(None, "auto").await;
            session.update_notif_senders(write_tx.clone()).await;
            return Ok((session, Some(payload)));
        }
    };

    match msg {
        zedra_rpc::Message::Notification(ref notif)
            if notif.method == methods::SESSION_ATTACH =>
        {
            let p: SessionAttachParams = serde_json::from_value(notif.params.clone())?;
            let target = registry.get_by_id(&p.session_id).await;
            match target {
                Some(server_session) if server_session.auth_token == p.auth_token => {
                    tracing::info!(
                        "bind_session: attached session {} via session/attach",
                        p.session_id,
                    );
                    server_session.update_notif_senders(write_tx.clone()).await;
                    server_session.touch().await;
                    Ok((server_session, None))
                }
                Some(_) => {
                    tracing::warn!(
                        "bind_session: auth token mismatch for session {}",
                        p.session_id,
                    );
                    anyhow::bail!("session/attach: auth token mismatch for {}", p.session_id)
                }
                None => {
                    tracing::warn!(
                        "bind_session: session {} not found, creating new",
                        p.session_id,
                    );
                    let session = registry.resume_or_create(None, &p.auth_token).await;
                    session.update_notif_senders(write_tx.clone()).await;
                    Ok((session, None))
                }
            }
        }
        zedra_rpc::Message::Request(ref req)
            if req.method == methods::SESSION_RESUME_OR_CREATE =>
        {
            let p: SessionResumeParams = serde_json::from_value(req.params.clone())?;
            let existing_id = p.session_id.as_deref();
            let server_session = registry.resume_or_create(existing_id, &p.auth_token).await;
            let resumed = existing_id.is_some_and(|id| id == server_session.id);
            let session_id = server_session.id.clone();

            let missed = server_session.notifications_after(p.last_notif_seq).await;
            let backlog: Vec<zedra_rpc::SessionBacklogEntry> = missed
                .iter()
                .map(|(seq, payload)| zedra_rpc::SessionBacklogEntry {
                    seq: *seq,
                    payload: base64_url::encode(payload),
                })
                .collect();

            for (_, notif_payload) in &missed {
                let _ = write_tx.send(notif_payload.clone()).await;
            }

            server_session.update_notif_senders(write_tx.clone()).await;

            let resp = zedra_rpc::Response::ok(
                req.id,
                serde_json::to_value(zedra_rpc::SessionResumeResult {
                    session_id,
                    resumed,
                    backlog,
                })?,
            );
            let _ = write_tx.send(serde_json::to_vec(&resp)?).await;

            tracing::info!(
                "bind_session: {} session {} via session/resume_or_create (backlog={})",
                if resumed { "resumed" } else { "created" },
                server_session.id,
                missed.len(),
            );

            Ok((server_session, None))
        }
        _ => {
            tracing::debug!("bind_session: auto-creating session for non-session first message");
            let session = registry.resume_or_create(None, "auto").await;
            session.update_notif_senders(write_tx.clone()).await;
            Ok((session, Some(payload)))
        }
    }
}

/// Dispatch a single message payload through the handler map.
async fn dispatch_payload(
    payload: &[u8],
    handlers: &HashMap<String, HandlerFn>,
    write_tx: &tokio::sync::mpsc::Sender<Vec<u8>>,
) {
    let msg: zedra_rpc::Message = match serde_json::from_slice(payload) {
        Ok(m) => m,
        Err(_) => return,
    };

    match msg {
        zedra_rpc::Message::Request(req) => {
            let write_tx = write_tx.clone();
            if let Some(handler) = handlers.get(&req.method) {
                let handler = handler.clone();
                tokio::spawn(async move {
                    let resp = match handler(req.params).await {
                        Ok(result) => zedra_rpc::Response::ok(req.id, result),
                        Err(e) => zedra_rpc::Response::err(
                            req.id,
                            zedra_rpc::INTERNAL_ERROR,
                            e.to_string(),
                        ),
                    };
                    let payload = serde_json::to_vec(&resp).unwrap_or_default();
                    let _ = write_tx.send(payload).await;
                });
            } else {
                let resp = zedra_rpc::Response::err(
                    req.id,
                    zedra_rpc::METHOD_NOT_FOUND,
                    format!("unknown method: {}", req.method),
                );
                let payload = serde_json::to_vec(&resp).unwrap_or_default();
                let _ = write_tx.send(payload).await;
            }
        }
        _ => {} // Ignore notifications and responses from client
    }
}

/// Handle a connection using the Transport trait.
///
/// Session binding happens upfront via `bind_session()` before any RPCs
/// are processed, ensuring handlers always have access to a bound session.
///
/// On disconnect the session remains alive in the registry for the grace
/// period, allowing the client to reconnect and resume.
pub async fn handle_transport_connection(
    transport: Box<dyn Transport>,
    registry: Arc<SessionRegistry>,
    daemon_state: Arc<DaemonState>,
) -> Result<()> {
    let transport_name = transport.name().to_string();
    tracing::info!("Transport connection established via {}", transport_name);

    // Create channels for bidirectional communication between the transport
    // and the dispatch loop.
    let (write_tx, mut write_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);
    let (recv_tx, mut recv_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);

    // Single transport I/O task: owns the transport and uses select! to
    // interleave reads and writes without mutex contention.
    let io_handle = tokio::spawn(async move {
        let mut transport = transport;
        loop {
            tokio::select! {
                result = transport.recv() => {
                    match result {
                        Ok(payload) => {
                            if recv_tx.send(payload).await.is_err() {
                                break;
                            }
                        }
                        Err(e) => {
                            tracing::debug!("Transport recv ended: {}", e);
                            break;
                        }
                    }
                }
                msg = write_rx.recv() => {
                    match msg {
                        Some(payload) => {
                            if let Err(e) = transport.send(&payload).await {
                                tracing::debug!("Transport send failed: {}", e);
                                break;
                            }
                        }
                        None => break,
                    }
                }
            }
        }
    });

    // Bind session upfront: read the first application message and resolve
    // the session before entering the dispatch loop.
    let (bound_session, replay_msg) = bind_session(&mut recv_rx, &write_tx, &registry).await?;
    let session_id = bound_session.id.clone();
    tracing::info!(
        "Session bound: id={}, transport={}, has_replay={}",
        session_id,
        transport_name,
        replay_msg.is_some(),
    );
    let session: Arc<tokio::sync::Mutex<Arc<ServerSession>>> =
        Arc::new(tokio::sync::Mutex::new(bound_session));

    // Build handler map using session-aware terminal handlers
    let handlers = build_session_handlers(
        daemon_state.clone(),
        registry.clone(),
        session.clone(),
        write_tx.clone(),
    );

    // If bind_session returned a non-session first message, dispatch it now.
    if let Some(payload) = replay_msg {
        dispatch_payload(&payload, &handlers, &write_tx).await;
    }

    // Dispatch loop
    while let Some(payload) = recv_rx.recv().await {
        dispatch_payload(&payload, &handlers, &write_tx).await;
    }

    // Clear notification senders so PTY readers don't send on a dead channel
    session.lock().await.clear_notif_senders().await;

    // Clean up: abort transport I/O task
    io_handle.abort();

    tracing::info!(
        "Transport connection closed: session={}, transport={} (session stays alive in registry)",
        session_id,
        transport_name,
    );
    Ok(())
}

/// Build handlers that use a ServerSession for terminal state (instead of DaemonState).
///
/// This allows terminals to persist across transport reconnections. Filesystem,
/// git, AI, and LSP handlers still use DaemonState directly since they're stateless.
fn build_session_handlers(
    state: Arc<DaemonState>,
    registry: Arc<SessionRegistry>,
    session: Arc<tokio::sync::Mutex<Arc<ServerSession>>>,
    write_tx: tokio::sync::mpsc::Sender<Vec<u8>>,
) -> HashMap<String, HandlerFn> {
    let mut handlers: HashMap<String, HandlerFn> = HashMap::new();

    macro_rules! register {
        ($method:expr, $handler:expr) => {
            handlers.insert($method.to_string(), Arc::new($handler));
        };
    }

    // -------------------------------------------------------------------
    // Session handlers
    // -------------------------------------------------------------------

    // session/resume_or_create — establish or resume a session
    let reg = registry.clone();
    let sess = session.clone();
    let wtx = write_tx.clone();
    register!(
        methods::SESSION_RESUME_OR_CREATE,
        move |params: serde_json::Value| {
            let reg = reg.clone();
            let sess = sess.clone();
            let wtx = wtx.clone();
            Box::pin(async move {
                let p: SessionResumeParams = serde_json::from_value(params)?;
                let existing_id = p.session_id.as_deref();
                let server_session = reg.resume_or_create(existing_id, &p.auth_token).await;
                let resumed = existing_id.is_some_and(|id| id == server_session.id);
                let session_id = server_session.id.clone();

                // Replay notification backlog
                let missed = server_session.notifications_after(p.last_notif_seq).await;
                let backlog: Vec<zedra_rpc::SessionBacklogEntry> = missed
                    .iter()
                    .map(|(seq, payload)| zedra_rpc::SessionBacklogEntry {
                        seq: *seq,
                        payload: base64_url::encode(payload),
                    })
                    .collect();

                // Also replay missed notifications through the write channel
                for (_, payload) in &missed {
                    let _ = wtx.send(payload.clone()).await;
                }

                // Update all terminal notification senders to point to this
                // connection's write channel so PTY output flows to the client.
                server_session.update_notif_senders(wtx.clone()).await;

                // Update bound session (idempotent re-call from durable queue replay)
                *sess.lock().await = server_session;

                Ok(serde_json::to_value(zedra_rpc::SessionResumeResult {
                    session_id,
                    resumed,
                    backlog,
                })?)
            })
        }
    );

    // session/heartbeat — keep session alive
    let sess = session.clone();
    register!(
        methods::SESSION_HEARTBEAT,
        move |_params: serde_json::Value| {
            let sess = sess.clone();
            Box::pin(async move {
                sess.lock().await.touch().await;
                Ok(serde_json::json!({"ok": true}))
            })
        }
    );

    // session/ping — lightweight RTT probe (no session touch, no side effects)
    register!(methods::SESSION_PING, move |_params: serde_json::Value| {
        Box::pin(async move { Ok(serde_json::to_value(zedra_rpc::PingResult { pong: true })?) })
    });

    // session/info — return hostname, workdir, username, OS, arch, version
    let s = state.clone();
    register!(methods::SESSION_INFO, move |_params: serde_json::Value| {
        let s = s.clone();
        Box::pin(async move {
            let hostname = hostname::get()
                .ok()
                .and_then(|h| h.into_string().ok())
                .unwrap_or_else(|| "unknown".to_string());
            let username = std::env::var("USER").unwrap_or_else(|_| "unknown".to_string());
            let workdir = s.workdir.to_string_lossy().into_owned();
            Ok(serde_json::to_value(zedra_rpc::SessionInfoResult {
                hostname,
                workdir,
                username,
                session_id: None,
                os: Some(std::env::consts::OS.to_string()),
                arch: Some(std::env::consts::ARCH.to_string()),
                os_version: os_version_string(),
                host_version: Some(env!("CARGO_PKG_VERSION").to_string()),
            })?)
        })
    });

    // session/list — list all available named sessions
    let reg = registry.clone();
    register!(
        methods::SESSION_LIST,
        move |_params: serde_json::Value| {
            let reg = reg.clone();
            Box::pin(async move {
                let list = reg.list_sessions().await;
                let entries: Vec<zedra_rpc::SessionListEntry> = list
                    .into_iter()
                    .map(|s| zedra_rpc::SessionListEntry {
                        id: s.id,
                        name: s.name,
                        workdir: s.workdir.map(|p| p.to_string_lossy().into_owned()),
                        terminal_count: s.terminal_count,
                        uptime_secs: s.created_at_elapsed_secs,
                        idle_secs: s.last_activity_elapsed_secs,
                    })
                    .collect();
                Ok(serde_json::to_value(zedra_rpc::SessionListResult {
                    sessions: entries,
                })?)
            })
        }
    );

    // session/switch — switch to a different named session
    let reg = registry.clone();
    let sess = session.clone();
    let wtx = write_tx.clone();
    register!(
        methods::SESSION_SWITCH,
        move |params: serde_json::Value| {
            let reg = reg.clone();
            let sess = sess.clone();
            let wtx = wtx.clone();
            Box::pin(async move {
                let p: zedra_rpc::SessionSwitchParams = serde_json::from_value(params)?;

                let target = reg
                    .get_by_name(&p.session_name)
                    .await
                    .ok_or_else(|| anyhow::anyhow!("Session '{}' not found", p.session_name))?;

                // Verify auth token matches.
                if target.auth_token != p.auth_token {
                    anyhow::bail!("Invalid auth token for session '{}'", p.session_name);
                }

                target.touch().await;

                // Replay notification backlog for the new session.
                let missed = target.notifications_after(p.last_notif_seq).await;
                let backlog: Vec<zedra_rpc::SessionBacklogEntry> = missed
                    .iter()
                    .map(|(seq, payload)| zedra_rpc::SessionBacklogEntry {
                        seq: *seq,
                        payload: base64_url::encode(payload),
                    })
                    .collect();

                for (_, payload) in &missed {
                    let _ = wtx.send(payload.clone()).await;
                }

                // Update terminal senders to point to this connection.
                target.update_notif_senders(wtx.clone()).await;

                let workdir = target.workdir.as_ref().map(|p| p.to_string_lossy().into_owned());
                let session_id = target.id.clone();

                // Switch the active session.
                *sess.lock().await = target;

                Ok(serde_json::to_value(zedra_rpc::SessionSwitchResult {
                    session_id,
                    workdir,
                    backlog,
                })?)
            })
        }
    );

    // -------------------------------------------------------------------
    // Filesystem handlers (stateless, use DaemonState)
    // -------------------------------------------------------------------

    let s = state.clone();
    register!(methods::FS_LIST, move |params: serde_json::Value| {
        let s = s.clone();
        Box::pin(async move {
            let p: FsListParams = serde_json::from_value(params)?;
            let path = s.workdir.join(&p.path);
            let entries = s.fs.list(&path)?;
            let rpc_entries: Vec<zedra_rpc::FsEntry> = entries
                .into_iter()
                .map(|e| zedra_rpc::FsEntry {
                    name: e.name,
                    path: e.path.to_string_lossy().into_owned(),
                    is_dir: e.is_dir,
                    size: e.size,
                })
                .collect();
            Ok(serde_json::to_value(rpc_entries)?)
        })
    });

    let s = state.clone();
    register!(methods::FS_READ, move |params: serde_json::Value| {
        let s = s.clone();
        Box::pin(async move {
            let p: FsReadParams = serde_json::from_value(params)?;
            let path = s.workdir.join(&p.path);
            let content = s.fs.read(&path)?;
            Ok(serde_json::to_value(zedra_rpc::FsReadResult { content })?)
        })
    });

    let s = state.clone();
    register!(methods::FS_WRITE, move |params: serde_json::Value| {
        let s = s.clone();
        Box::pin(async move {
            let p: FsWriteParams = serde_json::from_value(params)?;
            let path = s.workdir.join(&p.path);
            s.fs.write(&path, &p.content)?;
            Ok(serde_json::json!({"ok": true}))
        })
    });

    let s = state.clone();
    register!(methods::FS_STAT, move |params: serde_json::Value| {
        let s = s.clone();
        Box::pin(async move {
            let p: FsStatParams = serde_json::from_value(params)?;
            let path = s.workdir.join(&p.path);
            let stat = s.fs.stat(&path)?;
            Ok(serde_json::to_value(zedra_rpc::FsStatResult {
                path: stat.path.to_string_lossy().into_owned(),
                is_dir: stat.is_dir,
                size: stat.size,
                modified: stat.modified,
            })?)
        })
    });

    // -------------------------------------------------------------------
    // Git handlers (stateless, use DaemonState)
    // -------------------------------------------------------------------

    let s = state.clone();
    register!(methods::GIT_STATUS, move |_params: serde_json::Value| {
        let s = s.clone();
        Box::pin(async move {
            let repo = GitRepo::open(&s.workdir)?;
            let branch = repo.branch().unwrap_or_default();
            let entries = repo.status()?;
            let rpc_entries: Vec<zedra_rpc::GitStatusEntry> = entries
                .into_iter()
                .map(|e| zedra_rpc::GitStatusEntry {
                    path: e.path,
                    status: format!("{:?}", e.status).to_lowercase(),
                })
                .collect();
            Ok(serde_json::to_value(zedra_rpc::GitStatusResult {
                branch,
                entries: rpc_entries,
            })?)
        })
    });

    let s = state.clone();
    register!(methods::GIT_DIFF, move |params: serde_json::Value| {
        let s = s.clone();
        Box::pin(async move {
            let p: GitDiffParams = serde_json::from_value(params)?;
            let repo = GitRepo::open(&s.workdir)?;
            let diff = repo.diff(p.path.as_deref(), p.staged)?;
            Ok(serde_json::to_value(zedra_rpc::GitDiffResult { diff })?)
        })
    });

    let s = state.clone();
    register!(methods::GIT_LOG, move |params: serde_json::Value| {
        let s = s.clone();
        Box::pin(async move {
            let p: GitLogParams = serde_json::from_value(params)?;
            let repo = GitRepo::open(&s.workdir)?;
            let entries = repo.log(p.limit.unwrap_or(20))?;
            let rpc_entries: Vec<zedra_rpc::GitLogEntry> = entries
                .into_iter()
                .map(|e| zedra_rpc::GitLogEntry {
                    id: e.id,
                    message: e.message,
                    author: e.author,
                    timestamp: e.timestamp,
                })
                .collect();
            Ok(serde_json::to_value(rpc_entries)?)
        })
    });

    let s = state.clone();
    register!(methods::GIT_COMMIT, move |params: serde_json::Value| {
        let s = s.clone();
        Box::pin(async move {
            let p: GitCommitParams = serde_json::from_value(params)?;
            let repo = GitRepo::open(&s.workdir)?;
            let hash = repo.commit(&p.message, &p.paths)?;
            Ok(serde_json::json!({"hash": hash}))
        })
    });

    let s = state.clone();
    register!(
        methods::GIT_BRANCH_LIST,
        move |_params: serde_json::Value| {
            let s = s.clone();
            Box::pin(async move {
                let repo = GitRepo::open(&s.workdir)?;
                let branches = repo.branches()?;
                let rpc: Vec<zedra_rpc::GitBranchEntry> = branches
                    .into_iter()
                    .map(|b| zedra_rpc::GitBranchEntry {
                        name: b.name,
                        is_head: b.is_head,
                    })
                    .collect();
                Ok(serde_json::to_value(rpc)?)
            })
        }
    );

    let s = state.clone();
    register!(methods::GIT_CHECKOUT, move |params: serde_json::Value| {
        let s = s.clone();
        Box::pin(async move {
            let branch: String = serde_json::from_value::<serde_json::Value>(params)?
                .get("branch")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing branch"))?
                .to_string();
            let repo = GitRepo::open(&s.workdir)?;
            repo.checkout(&branch)?;
            Ok(serde_json::json!({"ok": true}))
        })
    });

    // -------------------------------------------------------------------
    // Terminal handlers (session-aware: terminals live in ServerSession)
    // -------------------------------------------------------------------

    // terminal/create — spawn PTY, store in session
    let sess = session.clone();
    let notif_tx = write_tx.clone();
    register!(methods::TERM_CREATE, move |params: serde_json::Value| {
        let sess = sess.clone();
        let notif_tx = notif_tx.clone();
        Box::pin(async move {
            let session = sess.lock().await.clone();
            session.touch().await;

            let p: TermCreateParams = serde_json::from_value(params)?;
            let shell = ShellSession::spawn(p.cols, p.rows)?;
            let (pty_reader, pty_writer, master) = shell.take_reader();
            let id = session.next_terminal_id().await;

            let notif_sender = Arc::new(std::sync::Mutex::new(Some(notif_tx.clone())));

            session.terminals.lock().await.insert(
                id.clone(),
                SessionTermSession {
                    writer: pty_writer,
                    master,
                    notif_sender: notif_sender.clone(),
                },
            );

            // Spawn PTY reader that sends terminal/output notifications
            // and also stores them in the session's notification backlog.
            // The reader survives transport reconnections: it never exits on
            // send failure, only when the PTY process itself closes.
            let term_id = id.clone();
            let sess_for_reader = session.clone();
            tokio::task::spawn_blocking(move || {
                let mut reader = pty_reader;
                let mut buf = [0u8; 8192];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            let data = base64_url::encode(&buf[..n]);
                            let notif = zedra_rpc::Notification::new(
                                methods::TERM_OUTPUT,
                                serde_json::json!({"id": term_id, "data": data}),
                            );
                            if let Ok(payload) = serde_json::to_vec(&notif) {
                                // Store in backlog for replay on reconnect
                                let sess = sess_for_reader.clone();
                                let payload_clone = payload.clone();
                                let rt = tokio::runtime::Handle::current();
                                rt.block_on(async {
                                    sess.push_notification(payload_clone).await;
                                });

                                // Try to send to current connection. If no
                                // connection is active (sender is None or
                                // channel closed), output is still in the
                                // backlog for replay on reconnect.
                                let sender = notif_sender.lock().unwrap().clone();
                                if let Some(tx) = sender {
                                    let _ = tx.blocking_send(payload);
                                }
                            }
                        }
                        Err(_) => break,
                    }
                }
            });

            Ok(serde_json::to_value(zedra_rpc::TermCreateResult { id })?)
        })
    });

    // terminal/data — write input to a terminal
    let sess = session.clone();
    register!(methods::TERM_DATA, move |params: serde_json::Value| {
        let sess = sess.clone();
        Box::pin(async move {
            let session = sess.lock().await.clone();
            session.touch().await;

            let p: TermDataParams = serde_json::from_value(params)?;
            let data =
                base64_url::decode(&p.data).map_err(|e| anyhow::anyhow!("bad base64: {}", e))?;
            let mut terms = session.terminals.lock().await;
            let term = terms
                .get_mut(&p.id)
                .ok_or_else(|| anyhow::anyhow!("unknown terminal: {}", p.id))?;
            term.writer.write_all(&data)?;
            term.writer.flush()?;
            Ok(serde_json::json!({"ok": true}))
        })
    });

    // terminal/resize
    let sess = session.clone();
    register!(methods::TERM_RESIZE, move |params: serde_json::Value| {
        let sess = sess.clone();
        Box::pin(async move {
            let session = sess.lock().await.clone();
            session.touch().await;

            let p: TermResizeParams = serde_json::from_value(params)?;
            let terms = session.terminals.lock().await;
            let term = terms
                .get(&p.id)
                .ok_or_else(|| anyhow::anyhow!("unknown terminal: {}", p.id))?;
            term.master
                .resize(portable_pty::PtySize {
                    rows: p.rows,
                    cols: p.cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(|e| anyhow::anyhow!("resize failed: {}", e))?;
            Ok(serde_json::json!({"ok": true}))
        })
    });

    // terminal/close
    let sess = session.clone();
    register!(methods::TERM_CLOSE, move |params: serde_json::Value| {
        let sess = sess.clone();
        Box::pin(async move {
            let session = sess.lock().await.clone();
            session.touch().await;

            let id: String = serde_json::from_value::<serde_json::Value>(params)?
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing id"))?
                .to_string();
            session.terminals.lock().await.remove(&id);
            Ok(serde_json::json!({"ok": true}))
        })
    });

    // terminal/list — list active terminal IDs
    let sess = session.clone();
    register!(methods::TERM_LIST, move |_params: serde_json::Value| {
        let sess = sess.clone();
        Box::pin(async move {
            let session = sess.lock().await.clone();
            let terms = session.terminals.lock().await;
            let entries: Vec<zedra_rpc::TermListEntry> = terms
                .keys()
                .map(|id| zedra_rpc::TermListEntry { id: id.clone() })
                .collect();
            Ok(serde_json::to_value(zedra_rpc::TermListResult {
                terminals: entries,
            })?)
        })
    });

    // -------------------------------------------------------------------
    // AI / Claude Code handlers
    // -------------------------------------------------------------------

    let s = state.clone();
    register!(methods::AI_PROMPT, move |params: serde_json::Value| {
        let s = s.clone();
        Box::pin(async move {
            let p: zedra_rpc::AiPromptParams = serde_json::from_value(params)?;
            let output = std::process::Command::new("claude")
                .args(["--print", &p.prompt])
                .current_dir(&s.workdir)
                .output();

            match output {
                Ok(out) if out.status.success() => {
                    let text = String::from_utf8_lossy(&out.stdout).into_owned();
                    Ok(serde_json::to_value(zedra_rpc::AiStreamChunk {
                        text,
                        done: true,
                    })?)
                }
                Ok(out) => {
                    let err = String::from_utf8_lossy(&out.stderr).into_owned();
                    Ok(serde_json::to_value(zedra_rpc::AiStreamChunk {
                        text: format!("Error: {}", err),
                        done: true,
                    })?)
                }
                Err(_) => Ok(serde_json::to_value(zedra_rpc::AiStreamChunk {
                    text: format!(
                        "[Claude Code not found on host. Install with: npm i -g @anthropic-ai/claude-code]\n\nPrompt was: {}",
                        p.prompt
                    ),
                    done: true,
                })?),
            }
        })
    });

    // -------------------------------------------------------------------
    // LSP proxy handlers
    // -------------------------------------------------------------------

    let s = state.clone();
    register!("lsp/diagnostics", move |params: serde_json::Value| {
        let s = s.clone();
        Box::pin(async move {
            let path: String = serde_json::from_value::<serde_json::Value>(params)?
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing path"))?
                .to_string();
            let full_path = s.workdir.join(&path);
            let diagnostics = run_lsp_check(&full_path);
            Ok(serde_json::to_value(diagnostics)?)
        })
    });

    register!("lsp/hover", |params: serde_json::Value| {
        Box::pin(async move {
            let _path: String = serde_json::from_value::<serde_json::Value>(params)?
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Ok(serde_json::json!({"contents": "LSP hover not yet connected to a language server."}))
        })
    });

    handlers
}

/// Run basic diagnostics on a file using available tooling.
fn run_lsp_check(path: &std::path::Path) -> Vec<LspDiagnostic> {
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
                // Return first few lines as diagnostics
                stderr
                    .lines()
                    .take(10)
                    .filter(|l| !l.is_empty())
                    .map(|line| LspDiagnostic {
                        message: line.to_string(),
                        severity: "error".into(),
                    })
                    .collect()
            }
        }
        Err(_) => vec![],
    }
}

#[derive(serde::Serialize)]
struct LspDiagnostic {
    message: String,
    severity: String,
}

/// Get a human-readable OS version string.
fn os_version_string() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        // Try /etc/os-release first (e.g. "Ubuntu 22.04.3 LTS")
        if let Ok(content) = std::fs::read_to_string("/etc/os-release") {
            for line in content.lines() {
                if let Some(pretty) = line.strip_prefix("PRETTY_NAME=") {
                    return Some(pretty.trim_matches('"').to_string());
                }
            }
        }
        // Fallback to kernel version
        let output = std::process::Command::new("uname").arg("-r").output().ok()?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use zedra_rpc::RpcClient;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Length-delimited transport over a tokio duplex stream (for tests).
    struct DuplexTransport {
        reader: tokio::io::ReadHalf<tokio::io::DuplexStream>,
        writer: tokio::io::WriteHalf<tokio::io::DuplexStream>,
    }

    impl DuplexTransport {
        fn new(stream: tokio::io::DuplexStream) -> Self {
            let (reader, writer) = tokio::io::split(stream);
            Self { reader, writer }
        }
    }

    #[async_trait::async_trait]
    impl Transport for DuplexTransport {
        async fn send(&mut self, payload: &[u8]) -> anyhow::Result<()> {
            let len = (payload.len() as u32).to_be_bytes();
            self.writer.write_all(&len).await?;
            self.writer.write_all(payload).await?;
            self.writer.flush().await?;
            Ok(())
        }

        async fn recv(&mut self) -> anyhow::Result<Vec<u8>> {
            let mut len_buf = [0u8; 4];
            self.reader.read_exact(&mut len_buf).await?;
            let len = u32::from_be_bytes(len_buf) as usize;
            if len > 16 * 1024 * 1024 {
                anyhow::bail!("message too large: {} bytes", len);
            }
            let mut payload = vec![0u8; len];
            self.reader.read_exact(&mut payload).await?;
            Ok(payload)
        }

        fn name(&self) -> &str {
            "duplex"
        }
    }

    async fn setup() -> (tempfile::TempDir, Arc<DaemonState>) {
        let dir = tempfile::tempdir().unwrap();
        // Init git repo
        Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        // Create a file
        std::fs::write(dir.path().join("hello.txt"), "hello world").unwrap();

        let state = Arc::new(DaemonState::new(dir.path().to_path_buf()));
        (dir, state)
    }

    /// Create a connected (client, server) pair over duplex streams.
    /// Server side runs handle_transport_connection in a background task.
    /// Client side returns an RpcClient + notification receiver.
    async fn setup_rpc_pair(
        state: Arc<DaemonState>,
    ) -> (RpcClient, tokio::sync::mpsc::Receiver<zedra_rpc::Notification>) {
        let (client_stream, server_stream) = tokio::io::duplex(64 * 1024);

        let registry = Arc::new(SessionRegistry::new());
        let server_transport = DuplexTransport::new(server_stream);
        tokio::spawn(async move {
            let _ = handle_transport_connection(
                Box::new(server_transport),
                registry,
                state,
            )
            .await;
        });

        let (cr, cw) = tokio::io::split(client_stream);
        RpcClient::spawn(cr, cw)
    }

    #[tokio::test]
    async fn rpc_fs_list() {
        let (_dir, state) = setup().await;
        let (client, _notifs) = setup_rpc_pair(state).await;

        let resp = client
            .call(methods::FS_LIST, serde_json::json!({"path": "."}))
            .await
            .unwrap();
        assert!(resp.error.is_none());
        let entries: Vec<zedra_rpc::FsEntry> =
            serde_json::from_value(resp.result.unwrap()).unwrap();
        assert!(entries.iter().any(|e| e.name == "hello.txt"));
    }

    #[tokio::test]
    async fn rpc_fs_read_write() {
        let (_dir, state) = setup().await;
        let (client, _notifs) = setup_rpc_pair(state).await;

        // Write
        let resp = client
            .call(
                methods::FS_WRITE,
                serde_json::json!({"path": "new.txt", "content": "new content"}),
            )
            .await
            .unwrap();
        assert!(resp.error.is_none());

        // Read back
        let resp = client
            .call(methods::FS_READ, serde_json::json!({"path": "new.txt"}))
            .await
            .unwrap();
        let result: zedra_rpc::FsReadResult = serde_json::from_value(resp.result.unwrap()).unwrap();
        assert_eq!(result.content, "new content");
    }

    #[tokio::test]
    async fn rpc_git_status() {
        let (_dir, state) = setup().await;
        let (client, _notifs) = setup_rpc_pair(state).await;

        let resp = client
            .call(methods::GIT_STATUS, serde_json::json!({}))
            .await
            .unwrap();
        assert!(resp.error.is_none());
        let status: zedra_rpc::GitStatusResult =
            serde_json::from_value(resp.result.unwrap()).unwrap();
        assert!(status.entries.iter().any(|e| e.path == "hello.txt"));
    }

    #[tokio::test]
    async fn rpc_terminal_lifecycle() {
        let (_dir, state) = setup().await;
        let (client, _notifs) = setup_rpc_pair(state).await;

        // Create terminal
        let resp = client
            .call(
                methods::TERM_CREATE,
                serde_json::json!({"cols": 80, "rows": 24}),
            )
            .await
            .unwrap();
        assert!(resp.error.is_none(), "create failed: {:?}", resp.error);
        let result: zedra_rpc::TermCreateResult =
            serde_json::from_value(resp.result.unwrap()).unwrap();
        assert!(result.id.starts_with("term-"));

        // Resize terminal
        let resp = client
            .call(
                methods::TERM_RESIZE,
                serde_json::json!({"id": result.id, "cols": 120, "rows": 40}),
            )
            .await
            .unwrap();
        assert!(resp.error.is_none(), "resize failed: {:?}", resp.error);

        // Close terminal
        let resp = client
            .call(methods::TERM_CLOSE, serde_json::json!({"id": result.id}))
            .await
            .unwrap();
        assert!(resp.error.is_none(), "close failed: {:?}", resp.error);
    }

    #[tokio::test]
    async fn rpc_terminal_output_streaming() {
        let (_dir, state) = setup().await;
        let (client, mut notifs) = setup_rpc_pair(state).await;

        // Create terminal
        let resp = client
            .call(
                methods::TERM_CREATE,
                serde_json::json!({"cols": 80, "rows": 24}),
            )
            .await
            .unwrap();
        assert!(resp.error.is_none(), "create failed: {:?}", resp.error);
        let result: zedra_rpc::TermCreateResult =
            serde_json::from_value(resp.result.unwrap()).unwrap();

        // Send a command
        let input = base64_url::encode(b"echo hello\n");
        let resp = client
            .call(
                methods::TERM_DATA,
                serde_json::json!({"id": result.id, "data": input}),
            )
            .await
            .unwrap();
        assert!(resp.error.is_none(), "data failed: {:?}", resp.error);

        // We should receive terminal/output notifications
        let notif = tokio::time::timeout(std::time::Duration::from_secs(5), notifs.recv())
            .await
            .expect("timed out waiting for terminal output")
            .expect("notification channel closed");
        assert_eq!(notif.method, methods::TERM_OUTPUT);
        let output: zedra_rpc::TermOutputNotification =
            serde_json::from_value(notif.params).unwrap();
        assert_eq!(output.id, result.id);
        assert!(!output.data.is_empty());

        // Close terminal
        let resp = client
            .call(methods::TERM_CLOSE, serde_json::json!({"id": result.id}))
            .await
            .unwrap();
        assert!(resp.error.is_none(), "close failed: {:?}", resp.error);
    }

    #[tokio::test]
    async fn rpc_session_info() {
        let (_dir, state) = setup().await;
        let (client, _notifs) = setup_rpc_pair(state).await;

        let resp = client
            .call(methods::SESSION_INFO, serde_json::json!({}))
            .await
            .unwrap();
        assert!(
            resp.error.is_none(),
            "session/info failed: {:?}",
            resp.error
        );
        let info: zedra_rpc::SessionInfoResult =
            serde_json::from_value(resp.result.unwrap()).unwrap();
        assert!(!info.hostname.is_empty());
        assert!(!info.workdir.is_empty());
        assert!(!info.username.is_empty());
    }

    #[tokio::test]
    async fn rpc_ai_prompt_fallback() {
        let (_dir, state) = setup().await;
        let (client, _notifs) = setup_rpc_pair(state).await;

        // AI prompt — should at least return something (fallback if no claude CLI)
        let resp = client
            .call(methods::AI_PROMPT, serde_json::json!({"prompt": "hello"}))
            .await
            .unwrap();
        assert!(resp.error.is_none(), "ai/prompt failed: {:?}", resp.error);
        let chunk: zedra_rpc::AiStreamChunk = serde_json::from_value(resp.result.unwrap()).unwrap();
        assert!(chunk.done);
        assert!(!chunk.text.is_empty());
    }

    #[tokio::test]
    async fn rpc_lsp_hover() {
        let (_dir, state) = setup().await;
        let (client, _notifs) = setup_rpc_pair(state).await;

        let resp = client
            .call("lsp/hover", serde_json::json!({"path": "hello.txt"}))
            .await
            .unwrap();
        assert!(resp.error.is_none());
    }
}
