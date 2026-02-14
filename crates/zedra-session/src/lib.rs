// zedra-session: RemoteSession client library for connecting to a zedra-host RPC daemon.
//
// Bridges async RPC calls to the GPUI main thread using the same global-state pattern
// as zedra-ssh (OutputBuffer, AtomicBool signaling, OnceLock singletons).
//
// Usage:
//   1. Call RemoteSession::connect(host, port) on the session runtime
//   2. Store the result via set_active_session()
//   3. Main thread polls check_and_clear_terminal_data() each frame
//   4. Main thread drains drain_callbacks() each frame for deferred work

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::Result;
use tokio::net::TcpStream;

use zedra_rpc::{
    methods, FsEntry, FsListParams, FsReadParams, FsReadResult, FsStatParams, FsStatResult,
    FsWriteParams, GitBranchEntry, GitCommitParams, GitDiffParams, GitDiffResult, GitLogEntry,
    GitLogParams, GitStatusResult, Response, RpcClient, SessionInfoResult, TermCreateParams,
    TermCreateResult, TermDataParams, TermOutputNotification, TermResizeParams,
};
use zedra_transport::{PeerInfo, TransportManager, TransportState};

// ---------------------------------------------------------------------------
// Type aliases
// ---------------------------------------------------------------------------

/// Thread-safe buffer for receiving terminal output from the remote host.
pub type OutputBuffer = Arc<Mutex<VecDeque<Vec<u8>>>>;

/// A boxed callback to run on the main thread.
pub type MainCallback = Box<dyn FnOnce() + Send + 'static>;

// ---------------------------------------------------------------------------
// Session state
// ---------------------------------------------------------------------------

/// Represents the current state of the remote session.
#[derive(Clone, Debug)]
pub enum SessionState {
    Disconnected,
    Connecting,
    Connected { hostname: String, workdir: String },
    Error(String),
}

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

/// Atomic flag: set by the notification listener when terminal/output arrives.
/// Polled by the main-thread frame loop to trigger terminal refreshes.
pub static TERMINAL_DATA_PENDING: AtomicBool = AtomicBool::new(false);

/// Dedicated tokio runtime for session I/O (2 worker threads).
static SESSION_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

/// Singleton slot for the currently-active remote session.
static ACTIVE_SESSION: OnceLock<Mutex<Option<Arc<RemoteSession>>>> = OnceLock::new();

/// Queue of callbacks to be drained and executed on the main thread.
static MAIN_THREAD_CALLBACKS: OnceLock<Mutex<VecDeque<MainCallback>>> = OnceLock::new();

/// Get (or lazily create) the session tokio runtime.
pub fn session_runtime() -> &'static tokio::runtime::Runtime {
    SESSION_RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("zedra-session")
            .build()
            .expect("failed to create session runtime")
    })
}

fn active_session_slot() -> &'static Mutex<Option<Arc<RemoteSession>>> {
    ACTIVE_SESSION.get_or_init(|| Mutex::new(None))
}

fn callback_queue() -> &'static Mutex<VecDeque<MainCallback>> {
    MAIN_THREAD_CALLBACKS.get_or_init(|| Mutex::new(VecDeque::new()))
}

/// Store a newly-connected session as the global active session.
pub fn set_active_session(session: Arc<RemoteSession>) {
    if let Ok(mut slot) = active_session_slot().lock() {
        *slot = Some(session);
        log::info!("Active remote session set");
    }
}

/// Retrieve the active session (if any).
pub fn active_session() -> Option<Arc<RemoteSession>> {
    if let Ok(slot) = active_session_slot().lock() {
        return slot.clone();
    }
    None
}

/// Clear the active session.
pub fn clear_active_session() {
    if let Ok(mut slot) = active_session_slot().lock() {
        *slot = None;
        log::info!("Active remote session cleared");
    }
}

/// Signal that terminal data is available (called from notification listener).
pub fn signal_terminal_data() {
    TERMINAL_DATA_PENDING.store(true, Ordering::Release);
}

/// Check and atomically clear the terminal-data-pending flag (called from main thread).
pub fn check_and_clear_terminal_data() -> bool {
    TERMINAL_DATA_PENDING.swap(false, Ordering::AcqRel)
}

/// Enqueue a callback to be executed on the main thread.
pub fn push_callback(cb: MainCallback) {
    if let Ok(mut queue) = callback_queue().lock() {
        queue.push_back(cb);
    }
}

/// Drain all pending main-thread callbacks. Call this from the frame loop.
pub fn drain_callbacks() -> VecDeque<MainCallback> {
    if let Ok(mut queue) = callback_queue().lock() {
        std::mem::take(&mut *queue)
    } else {
        VecDeque::new()
    }
}

/// Send terminal input to the remote host via the active session.
///
/// Encodes `data` as base64 and dispatches a `terminal/data` RPC call on the
/// session runtime. Returns `true` if the call was successfully enqueued.
pub fn send_terminal_input(data: Vec<u8>) -> bool {
    let session = match active_session() {
        Some(s) => s,
        None => return false,
    };

    let term_id = match session.terminal_id() {
        Some(id) => id,
        None => return false,
    };

    let encoded = base64_url::encode(&data);
    let client = session.client.clone();

    session_runtime().spawn(async move {
        let params = TermDataParams {
            id: term_id,
            data: encoded,
        };
        if let Err(e) = client
            .call(methods::TERM_DATA, serde_json::to_value(&params).unwrap())
            .await
        {
            log::error!("terminal_write RPC failed: {e}");
        }
    });

    true
}

// ---------------------------------------------------------------------------
// RemoteSession
// ---------------------------------------------------------------------------

/// Client-side handle to a remote zedra-host daemon.
///
/// Wraps an `RpcClient` and provides typed accessors for every RPC method.
/// The notification listener task pushes terminal output into `terminal_output`
/// and signals the main thread via `signal_terminal_data()`.
pub struct RemoteSession {
    client: Arc<RpcClient>,
    state: Arc<Mutex<SessionState>>,
    terminal_output: OutputBuffer,
    terminal_id: Arc<Mutex<Option<String>>>,
    session_id: Arc<Mutex<Option<String>>>,
    /// Transport state from TransportManager (None for direct TCP connections).
    transport_state: Option<Arc<Mutex<TransportState>>>,
    /// Latest ping RTT in milliseconds (0 = not yet measured).
    latency_ms: Arc<AtomicU64>,
}

impl RemoteSession {
    /// Connect to a zedra-host RPC daemon over TCP (direct connection).
    ///
    /// 1. Opens a TCP connection to `host:port`.
    /// 2. Spawns the RPC client reader/writer tasks.
    /// 3. Spawns a notification listener for `terminal/output`.
    /// 4. Fetches `session/info` to populate the Connected state.
    pub async fn connect(host: &str, port: u16) -> Result<Arc<Self>> {
        let addr = format!("{host}:{port}");
        log::info!("RemoteSession: connecting to {addr}");

        let stream = TcpStream::connect(&addr).await?;
        let (reader, writer) = tokio::io::split(stream);

        let (rpc_client, notif_rx) = RpcClient::spawn(reader, writer);
        let client = Arc::new(rpc_client);

        let terminal_output: OutputBuffer = Arc::new(Mutex::new(VecDeque::new()));
        let state = Arc::new(Mutex::new(SessionState::Connecting));

        let latency_ms = Arc::new(AtomicU64::new(0));

        let session = Arc::new(Self {
            client,
            state,
            terminal_output,
            terminal_id: Arc::new(Mutex::new(None)),
            session_id: Arc::new(Mutex::new(None)),
            transport_state: None,
            latency_ms: latency_ms.clone(),
        });

        // Spawn notification listener
        Self::spawn_notification_listener(&session, notif_rx);

        // Fetch session info to populate state
        Self::fetch_session_info(&session, host).await;

        // Spawn background ping loop for latency measurement
        Self::spawn_ping_loop(session.client.clone(), latency_ms);

        log::info!("RemoteSession: connected to {addr}");
        Ok(session)
    }

    /// Connect via TransportManager (supports multi-transport discovery).
    ///
    /// Uses PeerInfo from a v2 QR pairing payload to try LAN, Tailscale, and
    /// relay transports in priority order. The TransportManager runs in the
    /// background and can reconnect on transport failure.
    pub async fn connect_with_peer_info(peer_info: PeerInfo) -> Result<Arc<Self>> {
        let hostname = peer_info.hostname.clone();
        log::info!(
            "RemoteSession: connecting via TransportManager to {}",
            hostname
        );

        // 1. Create TransportManager and get session-facing channels
        let (mut manager, recv_rx, send_tx) = TransportManager::new(peer_info);

        // 2. Run discovery to find and connect the best transport
        manager.connect().await?;

        // 3. Create RpcClient over the channel pair (transport-agnostic)
        let (rpc_client, notif_rx) = RpcClient::spawn_from_channels(recv_rx, send_tx);
        let client = Arc::new(rpc_client);

        // 4. Grab transport state handle for UI monitoring
        let transport_state_handle = manager.state();

        // 5. Build session
        let terminal_output: OutputBuffer = Arc::new(Mutex::new(VecDeque::new()));
        let state = Arc::new(Mutex::new(SessionState::Connecting));
        let latency_ms = Arc::new(AtomicU64::new(0));

        let session = Arc::new(Self {
            client,
            state,
            terminal_output,
            terminal_id: Arc::new(Mutex::new(None)),
            session_id: Arc::new(Mutex::new(None)),
            transport_state: Some(transport_state_handle),
            latency_ms: latency_ms.clone(),
        });

        // 6. Spawn notification listener
        Self::spawn_notification_listener(&session, notif_rx);

        // 7. Spawn TransportManager background task (bridges transport <-> channels)
        tokio::spawn(manager.run());

        // 8. Fetch session info to populate state
        Self::fetch_session_info(&session, &hostname).await;

        // 9. Spawn background ping loop for latency measurement
        Self::spawn_ping_loop(session.client.clone(), latency_ms);

        log::info!("RemoteSession: connected via TransportManager to {}", hostname);
        Ok(session)
    }

    /// Spawn the notification listener that routes terminal/output to the buffer.
    fn spawn_notification_listener(
        session: &Arc<Self>,
        notif_rx: tokio::sync::mpsc::Receiver<zedra_rpc::Notification>,
    ) {
        let output_buf = session.terminal_output.clone();
        let mut rx = notif_rx;

        tokio::spawn(async move {
            while let Some(notif) = rx.recv().await {
                if notif.method == methods::TERM_OUTPUT {
                    match serde_json::from_value::<TermOutputNotification>(notif.params) {
                        Ok(term_notif) => {
                            match base64_url::decode(&term_notif.data) {
                                Ok(bytes) => {
                                    if let Ok(mut buf) = output_buf.lock() {
                                        buf.push_back(bytes);
                                    }
                                    signal_terminal_data();
                                }
                                Err(e) => {
                                    log::warn!("terminal/output base64 decode error: {e}");
                                }
                            }
                        }
                        Err(e) => {
                            log::warn!("terminal/output parse error: {e}");
                        }
                    }
                }
                // Other notification types can be handled here in the future
            }
            log::info!("Notification listener exited");
        });
    }

    /// Fetch session/info from the host and populate the session state.
    async fn fetch_session_info(session: &Arc<Self>, fallback_hostname: &str) {
        let info_client = session.client.clone();
        let info_state = session.state.clone();
        let session_id_slot = session.session_id.clone();
        let hostname_fallback = fallback_hostname.to_string();

        match info_client
            .call(methods::SESSION_INFO, serde_json::json!({}))
            .await
        {
            Ok(resp) => {
                if let Some(result) = resp.result {
                    match serde_json::from_value::<SessionInfoResult>(result) {
                        Ok(info) => {
                            // Store session_id if present in the response
                            if let Some(ref sid) = info.session_id {
                                if let Ok(mut id_slot) = session_id_slot.lock() {
                                    *id_slot = Some(sid.clone());
                                }
                            }
                            if let Ok(mut s) = info_state.lock() {
                                *s = SessionState::Connected {
                                    hostname: info.hostname,
                                    workdir: info.workdir,
                                };
                            }
                        }
                        Err(e) => {
                            log::warn!("session/info parse error: {e}");
                            if let Ok(mut s) = info_state.lock() {
                                *s = SessionState::Connected {
                                    hostname: hostname_fallback,
                                    workdir: String::new(),
                                };
                            }
                        }
                    }
                } else if let Some(err) = resp.error {
                    log::warn!("session/info error: {}", err.message);
                    if let Ok(mut s) = info_state.lock() {
                        *s = SessionState::Connected {
                            hostname: hostname_fallback,
                            workdir: String::new(),
                        };
                    }
                }
            }
            Err(e) => {
                log::warn!("session/info call failed: {e}");
                if let Ok(mut s) = info_state.lock() {
                    *s = SessionState::Error(format!("session/info failed: {e}"));
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Accessors
    // -----------------------------------------------------------------------

    /// Current session state.
    pub fn state(&self) -> SessionState {
        self.state
            .lock()
            .map(|s| s.clone())
            .unwrap_or(SessionState::Disconnected)
    }

    /// The shared output buffer that receives terminal data from the host.
    pub fn output_buffer(&self) -> OutputBuffer {
        self.terminal_output.clone()
    }

    /// The currently-active terminal ID (set after `terminal_create`).
    pub fn terminal_id(&self) -> Option<String> {
        self.terminal_id
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
    }

    /// The session ID assigned by the host (if available).
    pub fn session_id(&self) -> Option<String> {
        self.session_id
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
    }

    /// Current transport state (None for direct TCP connections).
    pub fn transport_state(&self) -> Option<TransportState> {
        self.transport_state
            .as_ref()
            .and_then(|ts| ts.lock().ok().map(|s| s.clone()))
    }

    /// Latest ping RTT in milliseconds (0 = not yet measured).
    pub fn latency_ms(&self) -> u64 {
        self.latency_ms.load(Ordering::Relaxed)
    }

    /// Send a `session/ping` RPC and measure the round-trip time.
    pub async fn ping(&self) -> Result<u64> {
        let start = std::time::Instant::now();
        let resp = self
            .client
            .call(methods::SESSION_PING, serde_json::json!({}))
            .await?;
        Self::check_error(resp)?;
        let rtt_ms = start.elapsed().as_millis() as u64;
        self.latency_ms.store(rtt_ms, Ordering::Relaxed);
        Ok(rtt_ms)
    }

    /// Spawn a background loop that pings every 10 seconds to measure RTT.
    fn spawn_ping_loop(client: Arc<RpcClient>, latency_ms: Arc<AtomicU64>) {
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                let start = std::time::Instant::now();
                match client
                    .call(methods::SESSION_PING, serde_json::json!({}))
                    .await
                {
                    Ok(_) => {
                        let rtt = start.elapsed().as_millis() as u64;
                        latency_ms.store(rtt, Ordering::Relaxed);
                        log::debug!("ping RTT: {}ms", rtt);
                    }
                    Err(e) => {
                        latency_ms.store(0, Ordering::Relaxed);
                        log::warn!("ping failed: {}", e);
                    }
                }
            }
        });
    }

    // -----------------------------------------------------------------------
    // Filesystem RPCs
    // -----------------------------------------------------------------------

    /// List directory entries at `path`.
    pub async fn fs_list(&self, path: &str) -> Result<Vec<FsEntry>> {
        let params = FsListParams {
            path: path.to_string(),
        };
        let resp = self
            .client
            .call(methods::FS_LIST, serde_json::to_value(&params)?)
            .await?;
        Self::extract_result(resp)
    }

    /// Read a file and return its contents as a string.
    pub async fn fs_read(&self, path: &str) -> Result<String> {
        let params = FsReadParams {
            path: path.to_string(),
        };
        let resp = self
            .client
            .call(methods::FS_READ, serde_json::to_value(&params)?)
            .await?;
        let result: FsReadResult = Self::extract_result(resp)?;
        Ok(result.content)
    }

    /// Write `content` to a file at `path`.
    pub async fn fs_write(&self, path: &str, content: &str) -> Result<()> {
        let params = FsWriteParams {
            path: path.to_string(),
            content: content.to_string(),
        };
        let resp = self
            .client
            .call(methods::FS_WRITE, serde_json::to_value(&params)?)
            .await?;
        Self::check_error(resp)
    }

    /// Stat a file or directory at `path`.
    pub async fn fs_stat(&self, path: &str) -> Result<FsStatResult> {
        let params = FsStatParams {
            path: path.to_string(),
        };
        let resp = self
            .client
            .call(methods::FS_STAT, serde_json::to_value(&params)?)
            .await?;
        Self::extract_result(resp)
    }

    // -----------------------------------------------------------------------
    // Git RPCs
    // -----------------------------------------------------------------------

    /// Get the current git status (branch + changed files).
    pub async fn git_status(&self) -> Result<GitStatusResult> {
        let resp = self
            .client
            .call(methods::GIT_STATUS, serde_json::json!({}))
            .await?;
        Self::extract_result(resp)
    }

    /// Get a diff, optionally for a specific path and/or staged changes.
    pub async fn git_diff(&self, path: Option<&str>, staged: bool) -> Result<String> {
        let params = GitDiffParams {
            path: path.map(|s| s.to_string()),
            staged,
        };
        let resp = self
            .client
            .call(methods::GIT_DIFF, serde_json::to_value(&params)?)
            .await?;
        let result: GitDiffResult = Self::extract_result(resp)?;
        Ok(result.diff)
    }

    /// Get recent commit log entries.
    pub async fn git_log(&self, limit: Option<usize>) -> Result<Vec<GitLogEntry>> {
        let params = GitLogParams { limit };
        let resp = self
            .client
            .call(methods::GIT_LOG, serde_json::to_value(&params)?)
            .await?;
        Self::extract_result(resp)
    }

    /// List all branches.
    pub async fn git_branches(&self) -> Result<Vec<GitBranchEntry>> {
        let resp = self
            .client
            .call(methods::GIT_BRANCH_LIST, serde_json::json!({}))
            .await?;
        Self::extract_result(resp)
    }

    /// Checkout a branch by name.
    pub async fn git_checkout(&self, branch: &str) -> Result<()> {
        let resp = self
            .client
            .call(
                methods::GIT_CHECKOUT,
                serde_json::json!({ "branch": branch }),
            )
            .await?;
        Self::check_error(resp)
    }

    /// Commit staged changes (or specific paths) with the given message.
    /// Returns the commit hash.
    pub async fn git_commit(&self, message: &str, paths: &[String]) -> Result<String> {
        let params = GitCommitParams {
            message: message.to_string(),
            paths: paths.to_vec(),
        };
        let resp = self
            .client
            .call(methods::GIT_COMMIT, serde_json::to_value(&params)?)
            .await?;
        // Expect { "hash": "abc123..." }
        let val = Self::extract_result::<serde_json::Value>(resp)?;
        val.get("hash")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow::anyhow!("git/commit response missing 'hash' field"))
    }

    // -----------------------------------------------------------------------
    // Terminal RPCs
    // -----------------------------------------------------------------------

    /// Create a new terminal on the remote host.
    /// Stores the returned terminal ID in `self.terminal_id`.
    pub async fn terminal_create(&self, cols: u16, rows: u16) -> Result<String> {
        let params = TermCreateParams { cols, rows };
        let resp = self
            .client
            .call(methods::TERM_CREATE, serde_json::to_value(&params)?)
            .await?;
        let result: TermCreateResult = Self::extract_result(resp)?;
        if let Ok(mut id_slot) = self.terminal_id.lock() {
            *id_slot = Some(result.id.clone());
        }
        log::info!("Terminal created with id: {}", result.id);
        Ok(result.id)
    }

    /// Write data to the terminal (data should be base64-encoded).
    pub async fn terminal_write(&self, id: &str, data: &str) -> Result<()> {
        let params = TermDataParams {
            id: id.to_string(),
            data: data.to_string(),
        };
        let resp = self
            .client
            .call(methods::TERM_DATA, serde_json::to_value(&params)?)
            .await?;
        Self::check_error(resp)
    }

    /// Resize the terminal.
    pub async fn terminal_resize(&self, id: &str, cols: u16, rows: u16) -> Result<()> {
        let params = TermResizeParams {
            id: id.to_string(),
            cols,
            rows,
        };
        let resp = self
            .client
            .call(methods::TERM_RESIZE, serde_json::to_value(&params)?)
            .await?;
        Self::check_error(resp)
    }

    /// Close the active terminal (uses the stored terminal_id).
    pub async fn terminal_close(&self) -> Result<()> {
        let id = self
            .terminal_id
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
            .ok_or_else(|| anyhow::anyhow!("no active terminal to close"))?;

        let resp = self
            .client
            .call(methods::TERM_CLOSE, serde_json::json!({ "id": id }))
            .await?;

        // Clear stored terminal ID
        if let Ok(mut id_slot) = self.terminal_id.lock() {
            *id_slot = None;
        }

        Self::check_error(resp)
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Extract a typed result from a successful RPC response.
    fn extract_result<T: serde::de::DeserializeOwned>(
        resp: Response,
    ) -> Result<T> {
        if let Some(err) = resp.error {
            anyhow::bail!("RPC error {}: {}", err.code, err.message);
        }
        let val = resp
            .result
            .ok_or_else(|| anyhow::anyhow!("RPC response missing result"))?;
        Ok(serde_json::from_value(val)?)
    }

    /// Check that an RPC response has no error (for void methods).
    fn check_error(resp: Response) -> Result<()> {
        if let Some(err) = resp.error {
            anyhow::bail!("RPC error {}: {}", err.code, err.message);
        }
        Ok(())
    }
}
