// RPC daemon: exposes filesystem, git, terminal, LSP, and AI operations over JSON-RPC.
//
// Runs alongside the SSH server. Mobile clients connect via TCP and issue
// JSON-RPC requests for file browsing, editing, git operations, terminal
// sessions, LSP queries, and Claude Code AI integration.

use anyhow::Result;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use zedra_fs::{Filesystem, LocalFs};
use zedra_git::GitRepo;
use zedra_rpc::methods;
use zedra_rpc::{FsListParams, FsReadParams, FsStatParams, FsWriteParams};
use zedra_rpc::{GitCommitParams, GitDiffParams, GitLogParams};
use zedra_rpc::{TermCreateParams, TermDataParams, TermResizeParams};
use zedra_rpc::RpcServer;

use crate::pty::ShellSession;

/// A live terminal session managed by the daemon.
struct TermSession {
    reader: Box<dyn Read + Send>,
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
pub async fn start_on_listener(
    listener: &TcpListener,
    state: Arc<DaemonState>,
) -> Result<()> {
    let (stream, addr) = listener.accept().await?;
    tracing::info!("RPC connection from {}", addr);
    handle_connection(stream, state).await
}

async fn handle_connection(
    stream: tokio::net::TcpStream,
    state: Arc<DaemonState>,
) -> Result<()> {
    let (reader, writer) = tokio::io::split(stream);
    let server = build_server(state);
    server.serve(reader, writer).await
}

fn build_server(state: Arc<DaemonState>) -> RpcServer {
    let mut server = RpcServer::new();

    // fs/list
    let s = state.clone();
    server.register(methods::FS_LIST, move |params| {
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
    server.register(methods::FS_READ, move |params| {
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
    server.register(methods::FS_WRITE, move |params| {
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
    server.register(methods::FS_STAT, move |params| {
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

    // git/status
    let s = state.clone();
    server.register(methods::GIT_STATUS, move |_params| {
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
    server.register(methods::GIT_DIFF, move |params| {
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
    server.register(methods::GIT_LOG, move |params| {
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
    server.register(methods::GIT_COMMIT, move |params| {
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
    server.register(methods::GIT_BRANCH_LIST, move |_params| {
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
    });

    // git/checkout
    let s = state.clone();
    server.register(methods::GIT_CHECKOUT, move |params| {
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

    // -----------------------------------------------------------------------
    // Terminal handlers
    // -----------------------------------------------------------------------

    // terminal/create — spawn a new PTY session
    let s = state.clone();
    server.register(methods::TERM_CREATE, move |params| {
        let s = s.clone();
        Box::pin(async move {
            let p: TermCreateParams = serde_json::from_value(params)?;
            let shell = ShellSession::spawn(p.cols, p.rows)?;
            let (reader, writer, master) = shell.take_reader();
            let id = s.next_terminal_id().await;
            s.terminals.lock().await.insert(
                id.clone(),
                TermSession {
                    reader,
                    writer,
                    master,
                },
            );
            Ok(serde_json::to_value(zedra_rpc::TermCreateResult { id })?)
        })
    });

    // terminal/data — write input to a terminal
    let s = state.clone();
    server.register(methods::TERM_DATA, move |params| {
        let s = s.clone();
        Box::pin(async move {
            let p: TermDataParams = serde_json::from_value(params)?;
            let data = base64_url::decode(&p.data)
                .map_err(|e| anyhow::anyhow!("bad base64: {}", e))?;
            let mut terms = s.terminals.lock().await;
            let term = terms
                .get_mut(&p.id)
                .ok_or_else(|| anyhow::anyhow!("unknown terminal: {}", p.id))?;
            term.writer.write_all(&data)?;
            term.writer.flush()?;

            // Read any available output
            let mut buf = [0u8; 8192];
            // Non-blocking: try to read what's available
            std::thread::sleep(std::time::Duration::from_millis(10));
            let n = match term.reader.read(&mut buf) {
                Ok(n) => n,
                Err(_) => 0,
            };
            let output = if n > 0 {
                base64_url::encode(&buf[..n])
            } else {
                String::new()
            };
            Ok(serde_json::json!({"output": output}))
        })
    });

    // terminal/resize — resize a terminal
    let s = state.clone();
    server.register(methods::TERM_RESIZE, move |params| {
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

    // terminal/close — close a terminal session
    let s = state.clone();
    server.register(methods::TERM_CLOSE, move |params| {
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

    // -----------------------------------------------------------------------
    // AI / Claude Code handlers
    // -----------------------------------------------------------------------

    // ai/prompt — execute a command or prompt via subprocess
    // Minimal integration: runs `claude` CLI if available, otherwise echoes.
    let s = state.clone();
    server.register(methods::AI_PROMPT, move |params| {
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
                    // claude CLI not available — echo back the prompt
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

    // -----------------------------------------------------------------------
    // LSP proxy handlers (minimal: diagnostics from file extension heuristics)
    // -----------------------------------------------------------------------

    // lsp/diagnostics — run basic checks on a file
    let s = state.clone();
    server.register("lsp/diagnostics", move |params| {
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

    // lsp/hover — placeholder for LSP hover info
    server.register("lsp/hover", |params| {
        Box::pin(async move {
            let _path: String = serde_json::from_value::<serde_json::Value>(params)?
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Ok(serde_json::json!({"contents": "LSP hover not yet connected to a language server."}))
        })
    });

    server
}

/// Run basic diagnostics on a file using available tooling.
fn run_lsp_check(path: &std::path::Path) -> Vec<LspDiagnostic> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    let (cmd, args): (&str, Vec<&str>) = match ext {
        "rs" => ("cargo", vec!["check", "--message-format=json"]),
        "ts" | "tsx" | "js" | "jsx" => ("npx", vec!["tsc", "--noEmit"]),
        "py" => ("python3", vec!["-m", "py_compile", path.to_str().unwrap_or("")]),
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
        let result: zedra_rpc::FsReadResult =
            serde_json::from_value(resp.result.unwrap()).unwrap();
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
            .call(methods::TERM_CREATE, serde_json::json!({"cols": 80, "rows": 24}))
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
        let chunk: zedra_rpc::AiStreamChunk =
            serde_json::from_value(resp.result.unwrap()).unwrap();
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
