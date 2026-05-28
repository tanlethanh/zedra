//! One running language server child.
//!
//! Scope is process supervision only: spawn, hold PID + state, drain stdout
//! and stderr so the child does not block on a full pipe, report RSS, and
//! shut down on demand. The actual LSP JSON-RPC protocol (initialize,
//! publishDiagnostics, ...) lands in a follow-up commit.
//!
//! The reason we spawn early without speaking the protocol is the
//! non-functional safety contract: the resource guard must enforce caps on a
//! real child process before we wire any LSP wire surface that might pin the
//! server in a request loop. This file is what gives the guard something to
//! supervise.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use anyhow::{Context, Result, anyhow};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, mpsc};
use zedra_rpc::proto::{LspDiagnosticV2, LspKillReason, LspLanguage, LspServerState};

use crate::policy::language_binary;
use crate::protocol::{self, DiagnosticStore};

/// One diagnostic-set update from a language server. Wholesale replacement
/// for `(language, path)`. Empty `diagnostics` means "clear the file".
#[derive(Debug, Clone)]
pub struct DiagnosticUpdate {
    pub language: LspLanguage,
    pub path: PathBuf,
    pub diagnostics: Vec<LspDiagnosticV2>,
}

/// Telemetry-stable language label. Kept here (mirrored in `zedra-host`) so
/// `zedra-lsp` can emit telemetry without a back-dependency.
pub fn language_label(language: LspLanguage) -> &'static str {
    match language {
        LspLanguage::Rust => "rust",
        LspLanguage::Go => "go",
        LspLanguage::TypeScript => "typescript",
        LspLanguage::JavaScript => "javascript",
        LspLanguage::Python => "python",
    }
}

/// A single supervised language server.
pub struct LspServer {
    language: LspLanguage,
    state: Mutex<LspServerState>,
    child: Mutex<Option<Child>>,
    pid: AtomicU32,
    started_at: Mutex<Option<Instant>>,
    peak_rss_bytes: AtomicRss,
    last_kill_reason: Mutex<Option<LspKillReason>>,
    diagnostics: Arc<Mutex<DiagnosticStore>>,
}

impl LspServer {
    pub fn new(language: LspLanguage) -> Arc<Self> {
        Arc::new(Self {
            language,
            state: Mutex::new(LspServerState::Idle),
            child: Mutex::new(None),
            pid: AtomicU32::new(0),
            started_at: Mutex::new(None),
            peak_rss_bytes: AtomicRss::new(),
            last_kill_reason: Mutex::new(None),
            diagnostics: Arc::new(Mutex::new(DiagnosticStore::new())),
        })
    }

    /// Snapshot of the diagnostic store for this server.
    pub async fn diagnostics_snapshot(&self) -> DiagnosticStore {
        self.diagnostics.lock().await.clone()
    }

    /// Aggregate diagnostic counts across all files. `(errors, warnings)`.
    pub async fn diagnostic_counts(&self) -> (u32, u32) {
        let mut errors = 0u32;
        let mut warnings = 0u32;
        for ds in self.diagnostics.lock().await.values() {
            for d in ds {
                match d.severity {
                    zedra_rpc::proto::LspSeverity::Error => errors += 1,
                    zedra_rpc::proto::LspSeverity::Warning => warnings += 1,
                    _ => {}
                }
            }
        }
        (errors, warnings)
    }

    pub fn language(&self) -> LspLanguage {
        self.language
    }

    pub fn pid(&self) -> Option<u32> {
        let pid = self.pid.load(Ordering::Relaxed);
        if pid == 0 { None } else { Some(pid) }
    }

    pub async fn state(&self) -> LspServerState {
        *self.state.lock().await
    }

    pub async fn uptime_secs(&self) -> u64 {
        self.started_at
            .lock()
            .await
            .map(|t| t.elapsed().as_secs())
            .unwrap_or(0)
    }

    pub fn peak_rss_bytes(&self) -> u64 {
        self.peak_rss_bytes.get()
    }

    pub fn record_rss(&self, bytes: u64) {
        self.peak_rss_bytes.maybe_set(bytes);
    }

    pub async fn last_kill_reason(&self) -> Option<LspKillReason> {
        *self.last_kill_reason.lock().await
    }

    /// Spawn the child and drive the LSP initialize handshake. After
    /// `initialized`, a background task reads `publishDiagnostics` and
    /// forwards each update to `diag_tx`. `workspace_root` is the absolute
    /// path the language server should index.
    pub async fn spawn(
        self: &Arc<Self>,
        workspace_root: &Path,
        diag_tx: mpsc::Sender<DiagnosticUpdate>,
    ) -> Result<u64> {
        {
            let state = self.state.lock().await;
            if matches!(*state, LspServerState::Starting | LspServerState::Ready) {
                return Ok(0);
            }
        }

        let bin = language_binary(self.language)
            .ok_or_else(|| anyhow!("no language server registered for {:?}", self.language))?;

        let started = Instant::now();
        *self.state.lock().await = LspServerState::Starting;

        let mut cmd = Command::new(bin.command);
        cmd.args(bin.args)
            .current_dir(workspace_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd
            .spawn()
            .with_context(|| format!("failed to spawn {}", bin.command))?;

        let pid = child.id().unwrap_or(0);
        self.pid.store(pid, Ordering::Relaxed);

        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("child stdin missing"))?;
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("child stdout missing"))?;
        // Drain stderr so the child does not block on a full pipe.
        if let Some(stderr) = child.stderr.take() {
            let lang = self.language;
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if !line.is_empty() {
                        tracing::debug!(language = ?lang, "lsp_stderr: {}", line);
                    }
                }
            });
        }

        *self.child.lock().await = Some(child);
        *self.started_at.lock().await = Some(started);
        *self.last_kill_reason.lock().await = None;
        self.peak_rss_bytes.reset();

        let cold_start_ms = started.elapsed().as_millis() as u64;
        zedra_telemetry::send(zedra_telemetry::Event::LspSpawn {
            language: language_label(self.language),
            cold_start_ms,
        });
        tracing::info!(
            language = ?self.language,
            pid = pid,
            cold_start_ms = cold_start_ms,
            "LSP server spawned",
        );

        // Drive the handshake on a background task so spawn() returns quickly.
        let me = Arc::clone(self);
        let workspace_root = workspace_root.to_path_buf();
        tokio::spawn(async move {
            if let Err(e) = protocol::handshake(&mut stdin, &mut stdout, &workspace_root).await {
                tracing::warn!(language = ?me.language, "LSP handshake failed: {}", e);
                *me.state.lock().await = LspServerState::Failed;
                return;
            }
            let init_ms = me
                .started_at
                .lock()
                .await
                .map(|t| t.elapsed().as_millis() as u64)
                .unwrap_or(0);
            *me.state.lock().await = LspServerState::Ready;
            zedra_telemetry::send(zedra_telemetry::Event::LspReady {
                language: language_label(me.language),
                init_ms,
            });
            tracing::info!(language = ?me.language, init_ms, "LSP server ready");
            // Reader loop owns stdout for the rest of the server's lifetime.
            let store = Arc::clone(&me.diagnostics);
            let lang = me.language;
            protocol::run_reader(stdout, store, move |path, diagnostics| {
                let _ = diag_tx.try_send(DiagnosticUpdate {
                    language: lang,
                    path,
                    diagnostics,
                });
            })
            .await;
        });

        Ok(cold_start_ms)
    }

    /// Terminate the child. `reason` is recorded for the next status snapshot
    /// and surfaced in `lsp_killed` telemetry.
    pub async fn shutdown(&self, reason: LspKillReason) {
        let mut child_guard = self.child.lock().await;
        let Some(mut child) = child_guard.take() else {
            *self.state.lock().await = LspServerState::Idle;
            return;
        };
        let uptime = self
            .started_at
            .lock()
            .await
            .map(|t| t.elapsed().as_secs())
            .unwrap_or(0);
        let peak_mb = self.peak_rss_bytes.get() / (1024 * 1024);

        // Best-effort SIGTERM first; fall back to SIGKILL after the grace.
        #[cfg(unix)]
        {
            if let Some(pid) = child.id() {
                // SAFETY: pid is positive and obtained from a live child handle.
                unsafe {
                    libc::kill(pid as libc::pid_t, libc::SIGTERM);
                }
            }
        }
        let grace = std::time::Duration::from_secs(3);
        let waited = tokio::time::timeout(grace, child.wait()).await;
        if waited.is_err() {
            let _ = child.kill().await;
        }
        self.pid.store(0, Ordering::Relaxed);
        *self.started_at.lock().await = None;
        *self.state.lock().await = LspServerState::Killed;
        *self.last_kill_reason.lock().await = Some(reason);

        zedra_telemetry::send(zedra_telemetry::Event::LspKilled {
            language: language_label(self.language),
            reason: kill_reason_label(reason),
            uptime_secs: uptime,
            peak_rss_mb: peak_mb,
        });
        tracing::info!(
            language = ?self.language,
            reason = ?reason,
            uptime_secs = uptime,
            peak_rss_mb = peak_mb,
            "LSP server shut down",
        );
    }

    /// Detect crashes (child exited on its own). Returns `true` and updates
    /// state to `Failed` when the child has exited; otherwise `false`.
    pub async fn poll_crash(&self) -> bool {
        let mut guard = self.child.lock().await;
        let Some(child) = guard.as_mut() else {
            return false;
        };
        match child.try_wait() {
            Ok(Some(status)) => {
                let uptime = self
                    .started_at
                    .lock()
                    .await
                    .map(|t| t.elapsed().as_secs())
                    .unwrap_or(0);
                let peak_mb = self.peak_rss_bytes.get() / (1024 * 1024);
                let exit_code = status.code().unwrap_or(-1);
                *guard = None;
                self.pid.store(0, Ordering::Relaxed);
                *self.state.lock().await = LspServerState::Failed;
                *self.last_kill_reason.lock().await = Some(LspKillReason::Crash);
                zedra_telemetry::send(zedra_telemetry::Event::LspKilled {
                    language: language_label(self.language),
                    reason: kill_reason_label(LspKillReason::Crash),
                    uptime_secs: uptime,
                    peak_rss_mb: peak_mb,
                });
                tracing::warn!(
                    language = ?self.language,
                    exit_code = exit_code,
                    "LSP server crashed",
                );
                true
            }
            Ok(None) => false,
            Err(e) => {
                tracing::warn!(language = ?self.language, "try_wait failed: {}", e);
                false
            }
        }
    }
}

fn kill_reason_label(reason: LspKillReason) -> &'static str {
    use LspKillReason::*;
    match reason {
        Oom => "oom",
        AggregateOom => "aggregate_oom",
        Cpu => "cpu",
        Idle => "idle",
        Manual => "manual",
        Crash => "crash",
    }
}

/// Cheap atomic peak tracker. Avoids holding a mutex on the hot RSS-poll
/// path.
struct AtomicRss(std::sync::atomic::AtomicU64);

impl AtomicRss {
    const fn new() -> Self {
        Self(std::sync::atomic::AtomicU64::new(0))
    }
    fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
    fn reset(&self) {
        self.0.store(0, Ordering::Relaxed);
    }
    fn maybe_set(&self, candidate: u64) {
        let mut cur = self.0.load(Ordering::Relaxed);
        while candidate > cur {
            match self
                .0
                .compare_exchange_weak(cur, candidate, Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(_) => return,
                Err(observed) => cur = observed,
            }
        }
    }
}
