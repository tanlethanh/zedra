// zedra-session: RemoteSession client library for connecting to a zedra-host RPC daemon.
//
// Bridges async RPC calls to the GPUI main thread using a global-state pattern
// (OutputBuffer, AtomicBool signaling, OnceLock singletons).
//
// Usage:
//   1. Call RemoteSession::connect_with_iroh(payload) on the session runtime
//   2. Store the result via set_active_session()
//   3. Main thread polls check_and_clear_terminal_data() each frame
//   4. Main thread drains drain_callbacks() each frame for deferred work

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::Result;

use zedra_rpc::{
    methods, FsEntry, FsListParams, FsReadParams, FsReadResult, FsStatParams, FsStatResult,
    FsWriteParams, GitBranchEntry, GitCommitParams, GitDiffParams, GitDiffResult, GitLogEntry,
    GitLogParams, GitStatusResult, Response, RpcClient, SessionInfoResult,
    TermCreateParams, TermCreateResult, TermDataParams, TermOutputNotification, TermResizeParams,
};
use zedra_transport::{CfWorkerDiscovery, IrohTransport, PairingPayload};

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
    Connected {
        hostname: String,
        username: String,
        workdir: String,
        /// e.g. "linux", "macos"
        os: String,
        /// e.g. "aarch64", "x86_64"
        arch: String,
        /// e.g. "Ubuntu 22.04.3 LTS", "macOS 15.3"
        os_version: String,
        /// zedra-host binary version
        host_version: String,
    },
    Error(String),
}

/// Metadata about the iroh connection path (direct P2P vs relay).
#[derive(Clone, Debug)]
pub struct ConnectionInfo {
    /// true = direct P2P (UDP holepunch), false = relayed
    pub is_direct: bool,
    /// Selected path address (IP:port or relay URL)
    pub remote_addr: String,
    /// Remote endpoint ID (short form)
    pub endpoint_id: String,
    /// Our local endpoint ID (short form)
    pub local_endpoint_id: String,
    /// Total number of available paths
    pub num_paths: usize,
    /// ALPN protocol string (e.g. "zedra/rpc/1")
    pub protocol: String,
    /// QUIC path RTT in milliseconds
    pub path_rtt_ms: u64,
    /// Bytes sent on this path
    pub bytes_sent: u64,
    /// Bytes received on this path
    pub bytes_recv: u64,
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
        tracing::info!("Active remote session set");
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
        tracing::info!("Active remote session cleared");
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

    let term_id = match session.active_terminal_id() {
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
            tracing::error!("terminal_write RPC failed: {e}");
        }
    });

    true
}

// ---------------------------------------------------------------------------
// RemoteSession
// ---------------------------------------------------------------------------

/// Per-terminal output buffers, keyed by terminal ID.
pub type TerminalOutputMap = Arc<Mutex<HashMap<String, OutputBuffer>>>;

/// Client-side handle to a remote zedra-host daemon.
///
/// Wraps an `RpcClient` and provides typed accessors for every RPC method.
/// The notification listener task pushes terminal output into per-terminal
/// buffers in `terminal_outputs` and signals the main thread via
/// `signal_terminal_data()`.
pub struct RemoteSession {
    client: Arc<RpcClient>,
    state: Arc<Mutex<SessionState>>,
    /// Legacy single-buffer field — used as fallback for direct TCP connections
    /// that don't go through terminal_create().
    terminal_output: OutputBuffer,
    /// Per-terminal output buffers (populated by terminal_create + notification listener).
    terminal_outputs: TerminalOutputMap,
    /// All terminal IDs created on this session, in creation order.
    terminal_ids: Arc<Mutex<Vec<String>>>,
    /// Which terminal currently receives input from send_terminal_input().
    active_terminal_id: Arc<Mutex<Option<String>>>,
    session_id: Arc<Mutex<Option<String>>>,
    auth_token: Arc<Mutex<Option<String>>>,
    /// Latest ping RTT in milliseconds (0 = not yet measured).
    latency_ms: Arc<AtomicU64>,
    /// Connection path metadata (direct vs relay), updated by watcher task.
    connection_info: Arc<Mutex<Option<ConnectionInfo>>>,
}

impl RemoteSession {
    /// Connect to a zedra-host via iroh.
    ///
    /// Uses iroh's Endpoint for direct QUIC/UDP with automatic NAT traversal,
    /// relay fallback, and TLS 1.3 encryption. No TransportManager needed —
    /// iroh handles discovery, connection, and reconnection internally.
    pub async fn connect_with_iroh(payload: PairingPayload) -> Result<Arc<Self>> {
        let hostname = payload.name.clone();
        tracing::info!(
            "RemoteSession: connecting via iroh to {} (endpoint: {})",
            hostname,
            &payload.endpoint_id[..16.min(payload.endpoint_id.len())],
        );

        // Build iroh endpoint (client side — generates ephemeral key)
        let mut builder = iroh::Endpoint::builder()
            .relay_mode(iroh::RelayMode::Disabled)
            .alpns(vec![b"zedra/rpc/1".to_vec()]);

        // Add CF Worker discovery if coord URL is available
        if let Some(ref url) = payload.coord_url {
            builder = builder.address_lookup(CfWorkerDiscovery::new(url));
        }

        let endpoint = builder.bind().await?;
        tracing::info!("iroh client endpoint bound: {}", endpoint.id().fmt_short());

        // Parse host's EndpointAddr from the pairing payload
        let addr = payload.to_endpoint_addr()?;
        tracing::info!(
            "Connecting to host endpoint: {:?}",
            addr,
        );

        // Connect to host
        let conn = endpoint.connect(addr, b"zedra/rpc/1").await?;
        tracing::info!("iroh: connected to {}", conn.remote_id().fmt_short());

        // Open a bidi stream for RPC
        let (send, recv) = conn.open_bi().await?;
        let transport = IrohTransport::new(send, recv);

        // Create RpcClient from transport channels
        let (reader, writer) = transport.into_rpc_channels();
        let (rpc_client, notif_rx) = RpcClient::spawn_from_channels(reader, writer);
        let client = Arc::new(rpc_client);

        let terminal_output: OutputBuffer = Arc::new(Mutex::new(VecDeque::new()));
        let terminal_outputs: TerminalOutputMap = Arc::new(Mutex::new(HashMap::new()));
        let state = Arc::new(Mutex::new(SessionState::Connecting));
        let latency_ms = Arc::new(AtomicU64::new(0));
        let connection_info: Arc<Mutex<Option<ConnectionInfo>>> = Arc::new(Mutex::new(None));

        // Spawn path watcher to track direct vs relay connection
        {
            use iroh::Watcher;
            let mut paths = conn.paths();
            let info_slot = connection_info.clone();
            let local_eid = endpoint.id().fmt_short().to_string();
            let remote_eid = conn.remote_id().fmt_short().to_string();
            let alpn = String::from_utf8_lossy(conn.alpn()).to_string();
            tokio::spawn(async move {
                loop {
                    let path_list = paths.get();
                    let selected = path_list.iter().find(|p| p.is_selected());
                    if let Some(path) = selected {
                        let stats = path.stats();
                        let info = ConnectionInfo {
                            is_direct: path.is_ip(),
                            remote_addr: format!("{:?}", path.remote_addr()),
                            endpoint_id: remote_eid.clone(),
                            local_endpoint_id: local_eid.clone(),
                            num_paths: path_list.len(),
                            protocol: alpn.clone(),
                            path_rtt_ms: stats.rtt.as_millis() as u64,
                            bytes_sent: stats.udp_tx.bytes,
                            bytes_recv: stats.udp_rx.bytes,
                        };
                        let was_relay = info_slot
                            .lock()
                            .ok()
                            .and_then(|g| g.as_ref().map(|i| !i.is_direct))
                            .unwrap_or(true);
                        if was_relay && info.is_direct {
                            tracing::info!("iroh path upgraded: relay -> direct P2P");
                        }
                        if let Ok(mut slot) = info_slot.lock() {
                            *slot = Some(info);
                        }
                    }
                    if paths.updated().await.is_err() {
                        tracing::debug!("iroh path watcher disconnected");
                        break;
                    }
                }
            });
        }

        let session = Arc::new(Self {
            client,
            state,
            terminal_output,
            terminal_outputs,
            terminal_ids: Arc::new(Mutex::new(Vec::new())),
            active_terminal_id: Arc::new(Mutex::new(None)),
            session_id: Arc::new(Mutex::new(None)),
            auth_token: Arc::new(Mutex::new(None)),
            latency_ms: latency_ms.clone(),
            connection_info,
        });

        // Spawn notification listener
        Self::spawn_notification_listener(&session, notif_rx);

        // Establish RPC session
        Self::establish_rpc_session(&session).await;

        // Fetch session info
        Self::fetch_session_info(&session, &hostname).await;

        // Spawn ping loop
        Self::spawn_ping_loop(session.client.clone(), latency_ms);

        tracing::info!("RemoteSession: connected via iroh to {}", hostname);
        Ok(session)
    }

    /// Spawn the notification listener that routes terminal/output to per-terminal buffers.
    fn spawn_notification_listener(
        session: &Arc<Self>,
        notif_rx: tokio::sync::mpsc::Receiver<zedra_rpc::Notification>,
    ) {
        let terminal_outputs = session.terminal_outputs.clone();
        let mut rx = notif_rx;

        tokio::spawn(async move {
            while let Some(notif) = rx.recv().await {
                if notif.method == methods::TERM_OUTPUT {
                    match serde_json::from_value::<TermOutputNotification>(notif.params) {
                        Ok(term_notif) => {
                            match base64_url::decode(&term_notif.data) {
                                Ok(bytes) => {
                                    // Route to per-terminal buffer if it exists,
                                    // creating one on the fly if needed (handles
                                    // race between create and first output).
                                    // Route to per-terminal buffer, creating one on
                                    // the fly if needed (handles race between create
                                    // and first output).
                                    let target_buf = {
                                        let mut map = terminal_outputs.lock().unwrap();
                                        map.entry(term_notif.id.clone())
                                            .or_insert_with(|| Arc::new(Mutex::new(VecDeque::new())))
                                            .clone()
                                    };
                                    if let Ok(mut buf) = target_buf.lock() {
                                        buf.push_back(bytes);
                                    }
                                    signal_terminal_data();
                                }
                                Err(e) => {
                                    tracing::warn!("terminal/output base64 decode error: {e}");
                                }
                            }
                        }
                        Err(e) => {
                            tracing::warn!("terminal/output parse error: {e}");
                        }
                    }
                }
                // Other notification types can be handled here in the future
            }
            tracing::info!("Notification listener exited");
        });
    }

    /// Establish an RPC session on the host via session/resume_or_create.
    ///
    /// On first call, creates a new session and stores the session_id + auth_token.
    /// On subsequent calls (after reconnect), resumes the existing session so
    /// terminals and other state survive transport switches.
    async fn establish_rpc_session(session: &Arc<Self>) {
        let auth_token = {
            let guard = session.auth_token.lock().unwrap();
            guard.clone()
        }
        .unwrap_or_else(|| {
            // Generate a stable auth token for this session lifetime
            let token = format!("{:016x}", std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos());
            if let Ok(mut guard) = session.auth_token.lock() {
                *guard = Some(token.clone());
            }
            token
        });

        let existing_session_id = session.session_id();

        let params = zedra_rpc::SessionResumeParams {
            session_id: existing_session_id.clone(),
            auth_token,
            last_notif_seq: 0,
        };

        match session
            .client
            .call(
                methods::SESSION_RESUME_OR_CREATE,
                serde_json::to_value(&params).unwrap(),
            )
            .await
        {
            Ok(resp) => {
                if let Some(result) = resp.result {
                    match serde_json::from_value::<zedra_rpc::SessionResumeResult>(result) {
                        Ok(resume) => {
                            tracing::info!(
                                "RPC session {}: id={}, backlog={} notifications",
                                if resume.resumed { "resumed" } else { "created" },
                                resume.session_id,
                                resume.backlog.len(),
                            );
                            if let Ok(mut id_slot) = session.session_id.lock() {
                                *id_slot = Some(resume.session_id);
                            }
                        }
                        Err(e) => {
                            tracing::warn!("session/resume_or_create parse error: {}", e);
                        }
                    }
                } else if let Some(err) = resp.error {
                    tracing::warn!("session/resume_or_create error: {}", err.message);
                }
            }
            Err(e) => {
                tracing::warn!("session/resume_or_create call failed: {}", e);
            }
        }
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
                                    username: info.username,
                                    workdir: info.workdir,
                                    os: info.os.unwrap_or_default(),
                                    arch: info.arch.unwrap_or_default(),
                                    os_version: info.os_version.unwrap_or_default(),
                                    host_version: info.host_version.unwrap_or_default(),
                                };
                            }
                        }
                        Err(e) => {
                            tracing::warn!("session/info parse error: {e}");
                            if let Ok(mut s) = info_state.lock() {
                                *s = SessionState::Connected {
                                    hostname: hostname_fallback,
                                    username: String::new(),
                                    workdir: String::new(),
                                    os: String::new(),
                                    arch: String::new(),
                                    os_version: String::new(),
                                    host_version: String::new(),
                                };
                            }
                        }
                    }
                } else if let Some(err) = resp.error {
                    tracing::warn!("session/info error: {}", err.message);
                    if let Ok(mut s) = info_state.lock() {
                        *s = SessionState::Connected {
                            hostname: hostname_fallback,
                            username: String::new(),
                            workdir: String::new(),
                            os: String::new(),
                            arch: String::new(),
                            os_version: String::new(),
                            host_version: String::new(),
                        };
                    }
                }
            }
            Err(e) => {
                tracing::warn!("session/info call failed: {e}");
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

    /// The shared output buffer for the active terminal (backward compat).
    ///
    /// If an active terminal is set and has a per-terminal buffer, returns that.
    /// Otherwise falls back to the legacy single buffer.
    pub fn output_buffer(&self) -> OutputBuffer {
        if let Some(id) = self.active_terminal_id() {
            if let Some(buf) = self.output_buffer_for(&id) {
                return buf;
            }
        }
        self.terminal_output.clone()
    }

    /// Get the output buffer for a specific terminal ID.
    pub fn output_buffer_for(&self, id: &str) -> Option<OutputBuffer> {
        self.terminal_outputs
            .lock()
            .ok()
            .and_then(|map| map.get(id).cloned())
    }

    /// The currently-active terminal ID (receives input from send_terminal_input).
    pub fn active_terminal_id(&self) -> Option<String> {
        self.active_terminal_id
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
    }

    /// Backward-compat alias for `active_terminal_id()`.
    pub fn terminal_id(&self) -> Option<String> {
        self.active_terminal_id()
    }

    /// List all terminal IDs in creation order.
    pub fn terminal_ids(&self) -> Vec<String> {
        self.terminal_ids
            .lock()
            .map(|ids| ids.clone())
            .unwrap_or_default()
    }

    /// Switch which terminal receives input from `send_terminal_input()`.
    pub fn set_active_terminal(&self, id: &str) {
        if let Ok(mut active) = self.active_terminal_id.lock() {
            *active = Some(id.to_string());
        }
    }

    /// The session ID assigned by the host (if available).
    pub fn session_id(&self) -> Option<String> {
        self.session_id
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
    }

    /// Latest ping RTT in milliseconds (0 = not yet measured).
    pub fn latency_ms(&self) -> u64 {
        self.latency_ms.load(Ordering::Relaxed)
    }

    /// Connection path metadata (direct P2P vs relay).
    pub fn connection_info(&self) -> Option<ConnectionInfo> {
        self.connection_info.lock().ok().and_then(|g| g.clone())
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
                        tracing::debug!("ping RTT: {}ms", rtt);
                    }
                    Err(e) => {
                        latency_ms.store(0, Ordering::Relaxed);
                        tracing::warn!("ping failed: {}", e);
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
    /// Registers a per-terminal output buffer and sets as active if first terminal.
    pub async fn terminal_create(&self, cols: u16, rows: u16) -> Result<String> {
        let params = TermCreateParams { cols, rows };
        let resp = self
            .client
            .call(methods::TERM_CREATE, serde_json::to_value(&params)?)
            .await?;
        let result: TermCreateResult = Self::extract_result(resp)?;

        // Register per-terminal output buffer
        {
            let mut map = self.terminal_outputs.lock().unwrap();
            map.entry(result.id.clone())
                .or_insert_with(|| Arc::new(Mutex::new(VecDeque::new())));
        }

        // Append to terminal IDs list
        let is_first = {
            let mut ids = self.terminal_ids.lock().unwrap();
            let first = ids.is_empty();
            ids.push(result.id.clone());
            first
        };

        // Set as active if it's the first terminal
        if is_first {
            if let Ok(mut active) = self.active_terminal_id.lock() {
                *active = Some(result.id.clone());
            }
        }

        tracing::info!("Terminal created with id: {}", result.id);
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

    /// Close the active terminal.
    pub async fn terminal_close(&self) -> Result<()> {
        let id = self
            .active_terminal_id()
            .ok_or_else(|| anyhow::anyhow!("no active terminal to close"))?;

        let resp = self
            .client
            .call(methods::TERM_CLOSE, serde_json::json!({ "id": id }))
            .await?;

        // Remove from terminal_ids
        if let Ok(mut ids) = self.terminal_ids.lock() {
            ids.retain(|i| i != &id);
        }

        // Remove per-terminal buffer
        if let Ok(mut map) = self.terminal_outputs.lock() {
            map.remove(&id);
        }

        // If this was the active terminal, switch to the next available one
        if self.active_terminal_id() == Some(id) {
            let next = self.terminal_ids.lock().ok().and_then(|ids| ids.first().cloned());
            if let Ok(mut active) = self.active_terminal_id.lock() {
                *active = next;
            }
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
