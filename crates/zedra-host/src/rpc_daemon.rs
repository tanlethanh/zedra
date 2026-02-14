// RPC daemon: exposes filesystem, git, terminal, LSP, and AI operations over JSON-RPC.
//
// Runs alongside the SSH server. Mobile clients connect via TCP and issue
// JSON-RPC requests for file browsing, editing, git operations, terminal
// sessions, LSP queries, and Claude Code AI integration.
//
// Architecture: manual dispatch loop with shared writer channel for bidirectional
// notifications. Terminal output is streamed as `terminal/output` notifications
// from a blocking PTY reader task through the shared writer.

use anyhow::Result;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use zedra_fs::{Filesystem, LocalFs};
use zedra_git::GitRepo;
use zedra_rpc::methods;
use zedra_rpc::{FsListParams, FsReadParams, FsStatParams, FsWriteParams};
use zedra_rpc::{GitCommitParams, GitDiffParams, GitLogParams};
use zedra_rpc::{TermCreateParams, TermDataParams, TermResizeParams};

use crate::pty::ShellSession;

/// Handler function type: takes params, returns result or error.
type HandlerFn = Arc<
    dyn Fn(serde_json::Value) -> futures::future::BoxFuture<'static, Result<serde_json::Value>>
        + Send
        + Sync,
>;

/// A live terminal session managed by the daemon.
/// The reader is split off and owned by a background streaming task.
struct TermSession {
    writer: Box<dyn Write + Send>,
    master: Box<dyn portable_pty::MasterPty + Send>,
}

/// Shared state for RPC handlers.
pub struct DaemonState {
    pub fs: Arc<dyn Filesystem>,
    pub workdir: std::path::PathBuf,
    terminals: Mutex<HashMap<String, TermSession>>,
    next_term_id: Mutex<u64>,
}

impl DaemonState {
    pub fn new(workdir: std::path::PathBuf) -> Self {
        Self {
            fs: Arc::new(LocalFs),
            workdir,
            terminals: Mutex::new(HashMap::new()),
            next_term_id: Mutex::new(1),
        }
    }

    async fn next_terminal_id(&self) -> String {
        let mut id = self.next_term_id.lock().await;
        let current = *id;
        *id += 1;
        format!("term-{}", current)
    }
}

/// Start the RPC daemon on the given port.
pub async fn run_daemon(bind: &str, port: u16, workdir: std::path::PathBuf) -> Result<()> {
    let listener = TcpListener::bind(format!("{}:{}", bind, port)).await?;
    tracing::info!("RPC daemon listening on {}:{}", bind, port);

    let state = Arc::new(DaemonState::new(workdir));

    loop {
        let (stream, addr) = listener.accept().await?;
        tracing::info!("RPC connection from {}", addr);
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, state).await {
                tracing::error!("RPC connection error: {}", e);
            }
        });
    }
}

/// Start the RPC daemon on a pre-bound listener (for tests).
pub async fn start_on_listener(listener: &TcpListener, state: Arc<DaemonState>) -> Result<()> {
    let (stream, addr) = listener.accept().await?;
    tracing::info!("RPC connection from {}", addr);
    handle_connection(stream, state).await
}

async fn handle_connection(stream: tokio::net::TcpStream, state: Arc<DaemonState>) -> Result<()> {
    let (reader, writer) = tokio::io::split(stream);

    // Create shared writer channel — responses and notifications both go through here.
    let (write_tx, mut write_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);

    // Writer task: drains channel and writes length-delimited frames to TCP.
    tokio::spawn(async move {
        let mut writer = writer;
        while let Some(payload) = write_rx.recv().await {
            let len = (payload.len() as u32).to_be_bytes();
            if writer.write_all(&len).await.is_err() {
                break;
            }
            if writer.write_all(&payload).await.is_err() {
                break;
            }
            let _ = writer.flush().await;
        }
    });

    // Build handler map (terminal/create needs write_tx for streaming notifications)
    let handlers = build_handlers(state.clone(), write_tx.clone());

    // Read loop: dispatch incoming requests to handlers
    let mut reader = reader;
    loop {
        let msg = match zedra_rpc::read_message(&mut reader).await {
            Ok(msg) => msg,
            Err(_) => break,
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
    Ok(())
}

/// Build the handler dispatch map. All handlers are registered here.
///
/// The `write_tx` channel is shared so that `terminal/create` can pass it to
/// the background PTY reader task for streaming `terminal/output` notifications.
fn build_handlers(
    state: Arc<DaemonState>,
    write_tx: tokio::sync::mpsc::Sender<Vec<u8>>,
) -> HashMap<String, HandlerFn> {
    let mut handlers: HashMap<String, HandlerFn> = HashMap::new();

    // Helper macro to reduce boilerplate for registering handlers.
    macro_rules! register {
        ($method:expr, $handler:expr) => {
            handlers.insert($method.to_string(), Arc::new($handler));
        };
    }

    // -------------------------------------------------------------------
    // Filesystem handlers
    // -------------------------------------------------------------------

    // fs/list
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

    // fs/read
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

    // fs/write
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

    // fs/stat
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
    // Git handlers
    // -------------------------------------------------------------------

    // git/status
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

    // git/diff
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

    // git/log
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

    // git/commit
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

    // git/branches
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

    // git/checkout
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
    // Terminal handlers
    // -------------------------------------------------------------------

    // terminal/create -- spawn a new PTY session and start streaming output
    let s = state.clone();
    let notif_tx = write_tx.clone();
    register!(methods::TERM_CREATE, move |params: serde_json::Value| {
        let s = s.clone();
        let notif_tx = notif_tx.clone();
        Box::pin(async move {
            let p: TermCreateParams = serde_json::from_value(params)?;
            let shell = ShellSession::spawn(p.cols, p.rows)?;
            let (pty_reader, pty_writer, master) = shell.take_reader();
            let id = s.next_terminal_id().await;

            // Store only writer + master (reader is owned by the streaming task)
            s.terminals.lock().await.insert(
                id.clone(),
                TermSession {
                    writer: pty_writer,
                    master,
                },
            );

            // Spawn blocking PTY reader task that sends terminal/output notifications
            let term_id = id.clone();
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
                                // blocking_send because we are in a spawn_blocking context
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

    // terminal/data -- write input to a terminal (input-only, no output polling)
    let s = state.clone();
    register!(methods::TERM_DATA, move |params: serde_json::Value| {
        let s = s.clone();
        Box::pin(async move {
            let p: TermDataParams = serde_json::from_value(params)?;
            let data =
                base64_url::decode(&p.data).map_err(|e| anyhow::anyhow!("bad base64: {}", e))?;
            let mut terms = s.terminals.lock().await;
            let term = terms
                .get_mut(&p.id)
                .ok_or_else(|| anyhow::anyhow!("unknown terminal: {}", p.id))?;
            term.writer.write_all(&data)?;
            term.writer.flush()?;
            Ok(serde_json::json!({"ok": true}))
        })
    });

    // terminal/resize -- resize a terminal
    let s = state.clone();
    register!(methods::TERM_RESIZE, move |params: serde_json::Value| {
        let s = s.clone();
        Box::pin(async move {
            let p: TermResizeParams = serde_json::from_value(params)?;
            let terms = s.terminals.lock().await;
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

    // terminal/close -- close a terminal session
    let s = state.clone();
    register!(methods::TERM_CLOSE, move |params: serde_json::Value| {
        let s = s.clone();
        Box::pin(async move {
            let id: String = serde_json::from_value::<serde_json::Value>(params)?
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing id"))?
                .to_string();
            s.terminals.lock().await.remove(&id);
            Ok(serde_json::json!({"ok": true}))
        })
    });

    // -------------------------------------------------------------------
    // Session handlers
    // -------------------------------------------------------------------

    // session/info -- return hostname, workdir, username
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
            })?)
        })
    });

    // -------------------------------------------------------------------
    // AI / Claude Code handlers
    // -------------------------------------------------------------------

    // ai/prompt -- execute a command or prompt via subprocess
    // Minimal integration: runs `claude` CLI if available, otherwise echoes.
    let s = state.clone();
    register!(methods::AI_PROMPT, move |params: serde_json::Value| {
        let s = s.clone();
        Box::pin(async move {
            let p: zedra_rpc::AiPromptParams = serde_json::from_value(params)?;

            // Try running `claude` CLI with the prompt
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
                Err(_) => {
                    // claude CLI not available -- echo back the prompt
                    Ok(serde_json::to_value(zedra_rpc::AiStreamChunk {
                        text: format!(
                            "[Claude Code not found on host. Install with: npm i -g @anthropic-ai/claude-code]\n\nPrompt was: {}",
                            p.prompt
                        ),
                        done: true,
                    })?)
                }
            }
        })
    });

    // -------------------------------------------------------------------
    // LSP proxy handlers (minimal: diagnostics from file extension heuristics)
    // -------------------------------------------------------------------

    // lsp/diagnostics -- run basic checks on a file
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

            // Try to run a language-specific linter
            let diagnostics = run_lsp_check(&full_path);
            Ok(serde_json::to_value(diagnostics)?)
        })
    });

    // lsp/hover -- placeholder for LSP hover info
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
