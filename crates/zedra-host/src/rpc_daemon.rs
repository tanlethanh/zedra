// RPC daemon: exposes filesystem, git, terminal, LSP, and AI operations over JSON-RPC.
//
// Mobile clients connect via TCP or relay and issue JSON-RPC requests for file
// browsing, editing, git operations, terminal sessions, LSP queries, and Claude
// Code AI integration.
//
// Architecture: manual dispatch loop with shared writer channel for bidirectional
// notifications. Terminal output is streamed as `terminal/output` notifications
// from a blocking PTY reader task through the shared writer.

use anyhow::Result;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::Arc;
use tokio::net::TcpListener;
use zedra_fs::{Filesystem, LocalFs};
use zedra_git::GitRepo;
use zedra_rpc::methods;
use zedra_rpc::{FsListParams, FsReadParams, FsStatParams, FsWriteParams};
use zedra_rpc::{GitCommitParams, GitDiffParams, GitLogParams};
use zedra_rpc::{SessionResumeParams, TermCreateParams, TermDataParams, TermResizeParams};
use zedra_rpc::{TcpTransport, Transport};

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
}

impl DaemonState {
    pub fn new(workdir: std::path::PathBuf) -> Self {
        Self {
            fs: Arc::new(LocalFs),
            workdir,
        }
    }
}

/// Start the RPC daemon TCP listener.
pub async fn run_daemon(
    bind: &str,
    port: u16,
    registry: Arc<SessionRegistry>,
    state: Arc<DaemonState>,
) -> Result<()> {
    let listener = TcpListener::bind(format!("{}:{}", bind, port)).await?;
    tracing::info!("RPC daemon listening on {}:{}", bind, port);

    loop {
        let (stream, addr) = listener.accept().await?;
        tracing::info!("RPC connection from {}", addr);
        let state = state.clone();
        let registry = registry.clone();
        tokio::spawn(async move {
            let transport = TcpTransport::new(stream);
            if let Err(e) = handle_transport_connection(Box::new(transport), registry, state).await
            {
                tracing::error!("RPC connection error from {}: {}", addr, e);
            }
        });
    }
}

/// Start the RPC daemon on a pre-bound listener (for tests).
pub async fn start_on_listener(listener: &TcpListener, state: Arc<DaemonState>) -> Result<()> {
    let registry = Arc::new(SessionRegistry::new());
    let (stream, addr) = listener.accept().await?;
    tracing::info!("RPC connection from {}", addr);
    let transport = TcpTransport::new(stream);
    handle_transport_connection(Box::new(transport), registry, state).await
}

/// Handle a connection using the Transport trait.
///
/// The first RPC call should be `session/resume_or_create` to establish or
/// resume a session. If the client skips this, a default session is created
/// automatically on the first terminal request.
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

    // Session state — lazily initialized on first session/resume_or_create call,
    // or auto-created on first terminal request.
    let session: Arc<tokio::sync::Mutex<Option<Arc<ServerSession>>>> =
        Arc::new(tokio::sync::Mutex::new(None));

    // Build handler map using session-aware terminal handlers
    let handlers = build_session_handlers(
        daemon_state.clone(),
        registry.clone(),
        session.clone(),
        write_tx.clone(),
    );

    // Dispatch loop
    while let Some(payload) = recv_rx.recv().await {
        let msg: zedra_rpc::Message = match serde_json::from_slice(&payload) {
            Ok(m) => m,
            Err(_) => continue,
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

    // Clean up: abort transport I/O task
    io_handle.abort();

    tracing::info!(
        "Transport connection closed via {} (session stays alive in registry)",
        transport_name
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
    session: Arc<tokio::sync::Mutex<Option<Arc<ServerSession>>>>,
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

                // Store the session
                *sess.lock().await = Some(server_session);

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
                if let Some(s) = sess.lock().await.as_ref() {
                    s.touch().await;
                }
                Ok(serde_json::json!({"ok": true}))
            })
        }
    );

    // session/ping — lightweight RTT probe (no session touch, no side effects)
    register!(methods::SESSION_PING, move |_params: serde_json::Value| {
        Box::pin(async move { Ok(serde_json::to_value(zedra_rpc::PingResult { pong: true })?) })
    });

    // session/info — return hostname, workdir, username
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
            })?)
        })
    });

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

    // Helper: ensure session exists (auto-create if client skipped resume_or_create)
    let ensure_session = {
        let reg = registry.clone();
        let sess = session.clone();
        move || {
            let reg = reg.clone();
            let sess = sess.clone();
            async move {
                let mut guard = sess.lock().await;
                if guard.is_none() {
                    let s = reg.resume_or_create(None, "auto").await;
                    *guard = Some(s.clone());
                    s
                } else {
                    guard.as_ref().unwrap().clone()
                }
            }
        }
    };

    // terminal/create — spawn PTY, store in session
    let es = ensure_session.clone();
    let notif_tx = write_tx.clone();
    register!(methods::TERM_CREATE, move |params: serde_json::Value| {
        let es = es.clone();
        let notif_tx = notif_tx.clone();
        Box::pin(async move {
            let session = es().await;
            session.touch().await;

            let p: TermCreateParams = serde_json::from_value(params)?;
            let shell = ShellSession::spawn(p.cols, p.rows)?;
            let (pty_reader, pty_writer, master) = shell.take_reader();
            let id = session.next_terminal_id().await;

            session.terminals.lock().await.insert(
                id.clone(),
                SessionTermSession {
                    writer: pty_writer,
                    master,
                },
            );

            // Spawn PTY reader that sends terminal/output notifications
            // and also stores them in the session's notification backlog.
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
                                // Use a runtime handle since we're in spawn_blocking
                                let rt = tokio::runtime::Handle::current();
                                rt.block_on(async {
                                    sess.push_notification(payload_clone).await;
                                });

                                if notif_tx.blocking_send(payload).is_err() {
                                    break;
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
    let es = ensure_session.clone();
    register!(methods::TERM_DATA, move |params: serde_json::Value| {
        let es = es.clone();
        Box::pin(async move {
            let session = es().await;
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
    let es = ensure_session.clone();
    register!(methods::TERM_RESIZE, move |params: serde_json::Value| {
        let es = es.clone();
        Box::pin(async move {
            let session = es().await;
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
    let es = ensure_session.clone();
    register!(methods::TERM_CLOSE, move |params: serde_json::Value| {
        let es = es.clone();
        Box::pin(async move {
            let session = es().await;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tokio::net::TcpListener;
    use zedra_rpc::RpcClient;

    async fn setup() -> (tempfile::TempDir, Arc<DaemonState>, TcpListener) {
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
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        (dir, state, listener)
    }

    #[tokio::test]
    async fn rpc_fs_list() {
        let (_dir, state, listener) = setup().await;
        let addr = listener.local_addr().unwrap();

        let state_clone = state.clone();
        tokio::spawn(async move {
            let _ = start_on_listener(&listener, state_clone).await;
        });

        let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (r, w) = tokio::io::split(stream);
        let (client, _notifs) = RpcClient::spawn(r, w);

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
        let (_dir, state, listener) = setup().await;
        let addr = listener.local_addr().unwrap();

        let state_clone = state.clone();
        tokio::spawn(async move {
            let _ = start_on_listener(&listener, state_clone).await;
        });

        let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (r, w) = tokio::io::split(stream);
        let (client, _notifs) = RpcClient::spawn(r, w);

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
        let (_dir, state, listener) = setup().await;
        let addr = listener.local_addr().unwrap();

        let state_clone = state.clone();
        tokio::spawn(async move {
            let _ = start_on_listener(&listener, state_clone).await;
        });

        let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (r, w) = tokio::io::split(stream);
        let (client, _notifs) = RpcClient::spawn(r, w);

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
        let (_dir, state, listener) = setup().await;
        let addr = listener.local_addr().unwrap();

        let state_clone = state.clone();
        tokio::spawn(async move {
            let _ = start_on_listener(&listener, state_clone).await;
        });

        let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (r, w) = tokio::io::split(stream);
        let (client, _notifs) = RpcClient::spawn(r, w);

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
        let (_dir, state, listener) = setup().await;
        let addr = listener.local_addr().unwrap();

        let state_clone = state.clone();
        tokio::spawn(async move {
            let _ = start_on_listener(&listener, state_clone).await;
        });

        let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (r, w) = tokio::io::split(stream);
        let (client, mut notifs) = RpcClient::spawn(r, w);

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
        let (_dir, state, listener) = setup().await;
        let addr = listener.local_addr().unwrap();

        let state_clone = state.clone();
        tokio::spawn(async move {
            let _ = start_on_listener(&listener, state_clone).await;
        });

        let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (r, w) = tokio::io::split(stream);
        let (client, _notifs) = RpcClient::spawn(r, w);

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
        let (_dir, state, listener) = setup().await;
        let addr = listener.local_addr().unwrap();

        let state_clone = state.clone();
        tokio::spawn(async move {
            let _ = start_on_listener(&listener, state_clone).await;
        });

        let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (r, w) = tokio::io::split(stream);
        let (client, _notifs) = RpcClient::spawn(r, w);

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
        let (_dir, state, listener) = setup().await;
        let addr = listener.local_addr().unwrap();

        let state_clone = state.clone();
        tokio::spawn(async move {
            let _ = start_on_listener(&listener, state_clone).await;
        });

        let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (r, w) = tokio::io::split(stream);
        let (client, _notifs) = RpcClient::spawn(r, w);

        let resp = client
            .call("lsp/hover", serde_json::json!({"path": "hello.txt"}))
            .await
            .unwrap();
        assert!(resp.error.is_none());
    }
}
