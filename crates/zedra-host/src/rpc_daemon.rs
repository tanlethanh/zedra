// RPC daemon: exposes filesystem, git, terminal, LSP, and AI operations over irpc.
//
// Mobile clients connect via iroh (QUIC/TLS 1.3) and issue typed RPC requests
// for file browsing, editing, git operations, terminal sessions, LSP queries,
// and Claude Code AI integration.
//
// Architecture: irpc read_request loop with typed dispatch on ZedraMessage.
// Terminal I/O uses bidi streaming via TermAttach (raw bytes, no base64/JSON).

use crate::fs::{Filesystem, LocalFs};
use crate::git::GitRepo;
use crate::pty::ShellSession;
use crate::session_registry::{ServerSession, SessionRegistry, TermSession};
use anyhow::Result;
use std::io::{Read, Write};
use std::sync::Arc;
use zedra_rpc::proto::*;

/// Shared state for RPC handlers.
pub struct DaemonState {
    pub fs: Arc<dyn Filesystem>,
    pub workdir: std::path::PathBuf,
}

impl std::fmt::Debug for DaemonState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DaemonState")
            .field("workdir", &self.workdir)
            .finish_non_exhaustive()
    }
}

impl DaemonState {
    pub fn new(workdir: std::path::PathBuf) -> Self {
        Self {
            fs: Arc::new(LocalFs),
            workdir,
        }
    }
}

/// Handle a single iroh connection using the irpc protocol.
///
/// Session binding happens upfront: the first message must be ResumeOrCreate.
/// All subsequent messages are dispatched to typed handlers.
///
/// On disconnect the session remains alive in the registry for the grace
/// period, allowing the client to reconnect and resume.
pub async fn handle_connection(
    conn: iroh::endpoint::Connection,
    registry: Arc<SessionRegistry>,
    state: Arc<DaemonState>,
) -> Result<()> {
    let remote = conn.remote_id();
    tracing::info!("irpc connection from {}", remote.fmt_short());

    // 1. First message must be ResumeOrCreate
    let session = match irpc_iroh::read_request::<ZedraProto>(&conn).await? {
        Some(ZedraMessage::ResumeOrCreate(msg)) => {
            let existing_id = msg.session_id.as_deref();
            let server_session = registry
                .resume_or_create(existing_id, &msg.auth_token)
                .await;
            let resumed = existing_id.is_some_and(|id| id == server_session.id);
            let session_id = server_session.id.clone();

            let _ = msg.tx.send(ResumeResult {
                session_id,
                resumed,
            }).await;

            tracing::info!(
                "Session {}: {} (id={})",
                if resumed { "resumed" } else { "created" },
                server_session.id,
                server_session.id,
            );

            server_session
        }
        Some(_) => return Err(anyhow::anyhow!("first message must be ResumeOrCreate")),
        None => return Ok(()),
    };

    // 2. Dispatch loop
    loop {
        match irpc_iroh::read_request::<ZedraProto>(&conn).await {
            Ok(Some(msg)) => {
                let s = session.clone();
                let st = state.clone();
                let r = registry.clone();
                tokio::spawn(async move {
                    if let Err(e) = dispatch(msg, s, st, r).await {
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

    // 3. Cleanup: clear output senders so PTY readers don't send on dead channels
    session.clear_output_senders().await;

    tracing::info!(
        "Connection closed: session={} (session stays alive in registry)",
        session.id,
    );
    Ok(())
}

/// Dispatch a single typed RPC message to its handler.
async fn dispatch(
    msg: ZedraMessage,
    session: Arc<ServerSession>,
    state: Arc<DaemonState>,
    registry: Arc<SessionRegistry>,
) -> Result<()> {
    match msg {
        // -- Session --
        ZedraMessage::ResumeOrCreate(msg) => {
            // Re-bind: client may re-send ResumeOrCreate mid-connection
            let existing_id = msg.session_id.as_deref();
            let server_session = registry
                .resume_or_create(existing_id, &msg.auth_token)
                .await;
            let resumed = existing_id.is_some_and(|id| id == server_session.id);
            let _ = msg.tx.send(ResumeResult {
                session_id: server_session.id.clone(),
                resumed,
            }).await;
        }

        ZedraMessage::Heartbeat(msg) => {
            session.touch().await;
            let _ = msg.tx.send(HeartbeatResult { ok: true }).await;
        }

        ZedraMessage::GetSessionInfo(msg) => {
            let hostname = hostname::get()
                .ok()
                .and_then(|h| h.into_string().ok())
                .unwrap_or_else(|| "unknown".to_string());
            let username =
                std::env::var("USER").unwrap_or_else(|_| "unknown".to_string());
            let workdir = state.workdir.to_string_lossy().into_owned();
            let _ = msg.tx.send(SessionInfoResult {
                hostname,
                workdir,
                username,
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
                })
                .collect();
            let _ = msg.tx.send(SessionListResult { sessions }).await;
        }

        ZedraMessage::SwitchSession(msg) => {
            let target = registry.get_by_name(&msg.session_name).await;
            match target {
                Some(t) if t.auth_token == msg.auth_token => {
                    t.touch().await;
                    let workdir = t
                        .workdir
                        .as_ref()
                        .map(|p| p.to_string_lossy().into_owned());
                    let _ = msg.tx.send(SessionSwitchResult {
                        session_id: t.id.clone(),
                        workdir,
                    }).await;
                }
                _ => {
                    // Drop sender to signal error
                    drop(msg.tx);
                }
            }
        }

        // -- Filesystem --
        ZedraMessage::FsList(msg) => {
            let path = state.workdir.join(&msg.path);
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
            let path = state.workdir.join(&msg.path);
            match state.fs.read(&path) {
                Ok(content) => {
                    let _ = msg.tx.send(FsReadResult { content }).await;
                }
                Err(_) => drop(msg.tx),
            }
        }

        ZedraMessage::FsWrite(msg) => {
            let path = state.workdir.join(&msg.path);
            let ok = state.fs.write(&path, &msg.content).is_ok();
            let _ = msg.tx.send(FsWriteResult { ok }).await;
        }

        ZedraMessage::FsStat(msg) => {
            let path = state.workdir.join(&msg.path);
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

            let shell = ShellSession::spawn(msg.cols, msg.rows)?;
            let (pty_reader, pty_writer, master) = shell.take_reader();
            let id = session.next_terminal_id().await;

            let output_sender = Arc::new(std::sync::Mutex::new(
                None::<tokio::sync::mpsc::Sender<TermOutput>>,
            ));

            session.terminals.lock().await.insert(
                id.clone(),
                TermSession {
                    writer: pty_writer,
                    master,
                    output_sender: output_sender.clone(),
                },
            );

            // Spawn PTY reader: reads raw bytes, stores in backlog, sends
            // to current TermAttach stream. Survives reconnections.
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
                            let seq = rt.block_on(sess_for_reader.next_backlog_seq());
                            rt.block_on(sess_for_reader.push_backlog_entry(BacklogEntry {
                                seq,
                                terminal_id: term_id.clone(),
                                data: data.clone(),
                            }));

                            let sender = output_sender.lock().unwrap().clone();
                            if let Some(tx) = sender {
                                let term_output = TermOutput { data, seq };
                                match tx.try_send(term_output) {
                                    Ok(()) => {}
                                    Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                                        *output_sender.lock().unwrap() = None;
                                    }
                                    Err(_) => {} // Full: output is in backlog
                                }
                            }
                        }
                        Err(_) => break,
                    }
                }
            });

            let _ = msg.tx.send(TermCreateResult { id }).await;
        }

        ZedraMessage::TermAttach(msg) => {
            session.touch().await;

            let term_id = msg.id.clone();
            let last_seq = msg.last_seq;
            let irpc_tx = msg.tx;
            let mut irpc_rx = msg.rx;

            // Check terminal exists
            {
                let terms = session.terminals.lock().await;
                if !terms.contains_key(&term_id) {
                    tracing::warn!("TermAttach: unknown terminal {}", term_id);
                    return Ok(());
                }
            }

            // Replay backlog
            let backlog = session.backlog_after(&term_id, last_seq).await;
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

            // Set up bridge: tokio channel → irpc sender
            let (bridge_tx, mut bridge_rx) = tokio::sync::mpsc::channel::<TermOutput>(256);
            {
                let terms = session.terminals.lock().await;
                if let Some(term) = terms.get(&term_id) {
                    *term.output_sender.lock().unwrap() = Some(bridge_tx);
                }
            }

            // Bridge loop: forward PTY output to client, and client input to PTY
            let session_for_input = session.clone();
            let term_id_for_input = term_id.clone();

            loop {
                tokio::select! {
                    output = bridge_rx.recv() => {
                        match output {
                            Some(term_output) => {
                                if irpc_tx.send(term_output).await.is_err() {
                                    break;
                                }
                            }
                            None => break,
                        }
                    }
                    input = irpc_rx.recv() => {
                        match input {
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
                }
            }

            // Cleanup: clear output sender
            {
                let terms = session.terminals.lock().await;
                if let Some(term) = terms.get(&term_id) {
                    *term.output_sender.lock().unwrap() = None;
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
                        .log(msg.limit.unwrap_or(20))
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
            let output = std::process::Command::new("claude")
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
            let full_path = state.workdir.join(&msg.path);
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

/// Run basic diagnostics on a file using available tooling.
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

/// Get a human-readable OS version string.
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
