// zedra-session: RemoteSession client library for connecting to a zedra-host RPC daemon.
//
// Uses irpc typed RPC over iroh QUIC streams. Terminal I/O uses bidi streaming
// (TermAttach) for efficient binary data transfer without base64 encoding.
//
// Per-workspace state is held in `SessionHandle`. Each workspace gets its own
// handle, so multiple concurrent connections don't conflict. A global "active
// handle" is maintained for backward-compat rendering code that reads session
// state during the main-thread frame loop (only the active workspace is rendered).
//
// Usage:
//   1. Create a SessionHandle for the workspace
//   2. Call RemoteSession::connect_with_iroh(addr, &handle)
//   3. Call set_active_handle(handle) when switching workspaces
//   4. Main thread polls check_and_clear_terminal_data() each frame
//   5. Main thread drains drain_callbacks() each frame for deferred work

pub mod signer;

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::Result;

use crate::signer::ClientSigner;
use zedra_rpc::proto::*;

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
    Connecting {
        /// Human-readable phase: "Creating endpoint", "QUIC handshake", etc.
        phase: String,
    },
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
    Reconnecting {
        attempt: u32,
        reason: ReconnectReason,
        /// Seconds until next attempt (0 = attempting now).
        next_retry_secs: u64,
    },
    /// All reconnect attempts exhausted (10 attempts, ~3 min total).
    /// User must explicitly retry or re-scan QR if credentials changed.
    HostUnreachable,
    Error(String),
}

/// Why a reconnect was triggered.
#[derive(Clone, Debug, PartialEq)]
pub enum ReconnectReason {
    /// QUIC connection closed (transport failure, timeout).
    ConnectionLost,
    /// App returned to foreground after iOS suspension.
    AppForegrounded,
}

/// Metadata about the iroh connection path (direct P2P vs relay).
#[derive(Clone, Debug)]
pub struct ConnectionInfo {
    /// true = direct P2P (UDP holepunch), false = relayed
    pub is_direct: bool,
    /// Selected path address (IP:port or relay URL)
    pub remote_addr: String,
    /// Relay hostname when path is relayed (e.g. "relay.zedra.dev"), None if direct.
    pub relay_url: Option<String>,
    /// Remote endpoint ID (short form)
    pub endpoint_id: String,
    /// Our local endpoint ID (short form)
    pub local_endpoint_id: String,
    /// Total number of available paths
    pub num_paths: usize,
    /// ALPN protocol string (e.g. "zedra/rpc/2")
    pub protocol: String,
    /// QUIC path RTT in milliseconds
    pub path_rtt_ms: u64,
    /// Bytes sent on this path
    pub bytes_sent: u64,
    /// Bytes received on this path
    pub bytes_recv: u64,
}

// ---------------------------------------------------------------------------
// SessionHandle — per-workspace state container
// ---------------------------------------------------------------------------

/// Per-workspace session state. Each workspace owns one of these.
///
/// Wraps shared state that persists across reconnects: terminal output
/// buffers, terminal IDs, endpoint address, credentials, and reconnect
/// state. Clonable (Arc-based).
#[derive(Clone)]
pub struct SessionHandle(Arc<SessionHandleInner>);

struct SessionHandleInner {
    session: Mutex<Option<Arc<RemoteSession>>>,
    endpoint_addr: Mutex<Option<iroh::EndpointAddr>>,
    /// Session ID used in AuthProveReq to reattach to the right session on reconnect.
    session_id_cred: Mutex<Option<String>>,
    terminal_outputs: TerminalOutputMap,
    terminal_ids: Arc<Mutex<Vec<String>>>,
    active_terminal: Arc<Mutex<Option<String>>>,
    reconnect_attempt: AtomicU32,
    reconnect_reason: Mutex<ReconnectReason>,
    user_disconnect: AtomicBool,
    skip_next_backoff: AtomicBool,
    /// Seconds until the next reconnect attempt (updated by the reconnect loop).
    next_retry_secs: AtomicU64,
    /// Client signing key for PKI auth (challenge response on every connection).
    signer: Mutex<Option<Arc<dyn ClientSigner>>>,
    /// Host's Ed25519 public key (EndpointId). Used to verify challenge signatures.
    endpoint_id: Mutex<Option<iroh::PublicKey>>,
    /// One-use pairing ticket from QR scan, consumed on first authenticate() call.
    pending_ticket: Mutex<Option<zedra_rpc::ZedraPairingTicket>>,
    /// Set after all reconnect attempts are exhausted.
    host_unreachable: AtomicBool,
}

impl SessionHandle {
    /// Create a new, empty session handle for a workspace.
    pub fn new() -> Self {
        Self(Arc::new(SessionHandleInner {
            session: Mutex::new(None),
            endpoint_addr: Mutex::new(None),
            session_id_cred: Mutex::new(None),
            terminal_outputs: Arc::new(Mutex::new(HashMap::new())),
            terminal_ids: Arc::new(Mutex::new(Vec::new())),
            active_terminal: Arc::new(Mutex::new(None)),
            reconnect_attempt: AtomicU32::new(0),
            reconnect_reason: Mutex::new(ReconnectReason::ConnectionLost),
            user_disconnect: AtomicBool::new(false),
            skip_next_backoff: AtomicBool::new(false),
            next_retry_secs: AtomicU64::new(0),
            signer: Mutex::new(None),
            endpoint_id: Mutex::new(None),
            pending_ticket: Mutex::new(None),
            host_unreachable: AtomicBool::new(false),
        }))
    }

    /// Set the client signing key for PKI auth.
    /// Must be called before `connect_with_ticket` or after construction for reconnect.
    pub fn set_signer(&self, signer: Arc<dyn ClientSigner>) {
        if let Ok(mut slot) = self.0.signer.lock() {
            *slot = Some(signer);
        }
    }

    /// Retrieve the stored signer (if set).
    pub fn signer(&self) -> Option<Arc<dyn ClientSigner>> {
        self.0.signer.lock().ok()?.clone()
    }

    /// Store the host's EndpointId for verifying challenge signatures on reconnect.
    pub fn store_endpoint_id(&self, id: iroh::PublicKey) {
        if let Ok(mut slot) = self.0.endpoint_id.lock() {
            *slot = Some(id);
        }
    }

    /// Get the stored host EndpointId.
    pub fn stored_endpoint_id(&self) -> Option<iroh::PublicKey> {
        self.0.endpoint_id.lock().ok()?.clone()
    }

    /// Whether the host is considered permanently unreachable (all attempts exhausted).
    pub fn is_host_unreachable(&self) -> bool {
        self.0.host_unreachable.load(Ordering::Relaxed)
    }

    /// Store a one-use pairing ticket from a QR scan.
    ///
    /// Consumed by the next `connect_with_iroh` call (Register step). After
    /// the first successful Register the slot is cleared and never used again.
    pub fn set_pending_ticket(&self, ticket: zedra_rpc::ZedraPairingTicket) {
        if let Ok(mut slot) = self.0.pending_ticket.lock() {
            *slot = Some(ticket);
        }
    }

    /// Get the current remote session (if connected).
    pub fn session(&self) -> Option<Arc<RemoteSession>> {
        self.0.session.lock().ok()?.clone()
    }

    /// Store a newly-connected session.
    pub fn set_session(&self, session: Arc<RemoteSession>) {
        if let Ok(mut slot) = self.0.session.lock() {
            *slot = Some(session);
            tracing::info!("SessionHandle: session set");
        }
    }

    /// Clear the session (user-initiated disconnect).
    ///
    /// Sets `user_disconnect` to prevent automatic reconnect attempts.
    pub fn clear_session(&self) {
        self.0.user_disconnect.store(true, Ordering::Release);
        if let Ok(mut slot) = self.0.session.lock() {
            *slot = None;
            tracing::info!("SessionHandle: session cleared (user disconnect)");
        }
    }

    /// Current reconnect attempt number (0 = not reconnecting).
    pub fn reconnect_attempt(&self) -> u32 {
        self.0.reconnect_attempt.load(Ordering::Relaxed)
    }

    /// Whether a reconnect is currently in progress.
    pub fn is_reconnecting(&self) -> bool {
        self.reconnect_attempt() > 0
    }

    /// Why the current reconnect was triggered.
    pub fn reconnect_reason(&self) -> ReconnectReason {
        self.0
            .reconnect_reason
            .lock()
            .map(|r| r.clone())
            .unwrap_or(ReconnectReason::ConnectionLost)
    }

    /// Seconds until the next reconnect attempt (0 = attempting now).
    pub fn next_retry_secs(&self) -> u64 {
        self.0.next_retry_secs.load(Ordering::Relaxed)
    }

    /// Effective session state for UI display.
    ///
    /// Reconnect state from the handle's atomics takes priority over the
    /// underlying session state (which is not updated by the reconnect loop).
    pub fn state(&self) -> SessionState {
        let attempt = self.reconnect_attempt();
        if attempt > 0 {
            return SessionState::Reconnecting {
                attempt,
                reason: self.reconnect_reason(),
                next_retry_secs: self.next_retry_secs(),
            };
        }
        if self.is_host_unreachable() {
            return SessionState::HostUnreachable;
        }
        self.session()
            .map(|s| s.state())
            .unwrap_or(SessionState::Disconnected)
    }

    /// Store an endpoint address for use during automatic reconnect.
    pub fn store_endpoint_addr(&self, addr: iroh::EndpointAddr) {
        if let Ok(mut slot) = self.0.endpoint_addr.lock() {
            *slot = Some(addr);
        }
    }

    /// Get the stored endpoint address (if any).
    pub fn endpoint_addr(&self) -> Option<iroh::EndpointAddr> {
        self.0.endpoint_addr.lock().ok()?.clone()
    }

    /// Get the stored session ID (used in AuthProveReq on reconnect).
    pub fn credentials_pub(&self) -> Option<String> {
        self.credentials()
    }

    /// Store the session ID from persisted workspace data (for session resumption).
    pub fn store_credentials_pub(&self, session_id: Option<String>) {
        self.store_credentials(session_id);
    }

    /// Send terminal input to the remote host via the active session.
    ///
    /// Sends raw bytes through the TermAttach bidi stream's input channel.
    /// Returns `true` if the data was successfully enqueued.
    pub fn send_terminal_input(&self, data: Vec<u8>) -> bool {
        let session = match self.session() {
            Some(s) => s,
            None => return false,
        };

        let term_id = match session.active_terminal_id() {
            Some(id) => id,
            None => return false,
        };

        let sender = {
            let senders = match session.terminal_input_senders.lock() {
                Ok(s) => s,
                Err(_) => return false,
            };
            match senders.get(&term_id) {
                Some(tx) => tx.clone(),
                None => return false,
            }
        };

        match sender.try_send(data) {
            Ok(()) => true,
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                tracing::warn!("terminal input channel full");
                true
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                tracing::warn!("terminal input channel closed");
                false
            }
        }
    }

    // --- Internal accessors used by RemoteSession ---

    fn terminal_outputs(&self) -> TerminalOutputMap {
        self.0.terminal_outputs.clone()
    }

    fn terminal_ids_slot(&self) -> Arc<Mutex<Vec<String>>> {
        self.0.terminal_ids.clone()
    }

    fn active_terminal_slot(&self) -> Arc<Mutex<Option<String>>> {
        self.0.active_terminal.clone()
    }

    fn credentials(&self) -> Option<String> {
        self.0.session_id_cred.lock().ok()?.clone()
    }

    fn store_credentials(&self, session_id: Option<String>) {
        if let Ok(mut slot) = self.0.session_id_cred.lock() {
            *slot = session_id;
        }
    }
}

// ---------------------------------------------------------------------------
// Global state (shared across all workspaces)
// ---------------------------------------------------------------------------

/// Atomic flag: set by the terminal output pump when TermOutput arrives.
/// Polled by the main-thread frame loop to trigger terminal refreshes.
pub static TERMINAL_DATA_PENDING: AtomicBool = AtomicBool::new(false);

/// Dedicated tokio runtime for session I/O (2 worker threads).
static SESSION_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

/// Queue of callbacks to be drained and executed on the main thread.
static MAIN_THREAD_CALLBACKS: OnceLock<Mutex<VecDeque<MainCallback>>> = OnceLock::new();

/// The currently-active workspace's session handle.
///
/// Swapped by `set_active_handle()` when the user switches workspaces.
/// Read by backward-compat functions (`active_session()`, `reconnect_attempt()`,
/// `send_terminal_input()`) during the render frame.
static ACTIVE_HANDLE: OnceLock<Mutex<Option<SessionHandle>>> = OnceLock::new();

fn active_handle_slot() -> &'static Mutex<Option<SessionHandle>> {
    ACTIVE_HANDLE.get_or_init(|| Mutex::new(None))
}

/// Set which workspace's handle is currently active.
///
/// Call this when the user switches workspaces so that backward-compat
/// rendering code (which calls `active_session()`) reads the right session.
pub fn set_active_handle(handle: SessionHandle) {
    if let Ok(mut slot) = active_handle_slot().lock() {
        *slot = Some(handle);
    }
}

/// Get the currently-active workspace's handle.
pub fn active_handle() -> Option<SessionHandle> {
    active_handle_slot().lock().ok()?.clone()
}

/// Clear the active handle (e.g. when the last workspace is closed).
pub fn clear_active_handle() {
    if let Ok(mut slot) = active_handle_slot().lock() {
        *slot = None;
    }
}

// ---------------------------------------------------------------------------
// Backward-compat global functions
// ---------------------------------------------------------------------------

/// Retrieve the active session (delegates to active handle).
pub fn active_session() -> Option<Arc<RemoteSession>> {
    active_handle()?.session()
}

/// Current reconnect attempt for the active handle (0 = not reconnecting).
pub fn reconnect_attempt() -> u32 {
    active_handle().map_or(0, |h| h.reconnect_attempt())
}

/// Whether a reconnect is in progress for the active handle.
pub fn is_reconnecting() -> bool {
    reconnect_attempt() > 0
}

/// Send terminal input via the active handle.
pub fn send_terminal_input(data: Vec<u8>) -> bool {
    active_handle().map_or(false, |h| h.send_terminal_input(data))
}

// ---------------------------------------------------------------------------
// Shared global utilities (not per-workspace)
// ---------------------------------------------------------------------------

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

fn callback_queue() -> &'static Mutex<VecDeque<MainCallback>> {
    MAIN_THREAD_CALLBACKS.get_or_init(|| Mutex::new(VecDeque::new()))
}

/// Signal that terminal data is available (called from output pump).
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

// ---------------------------------------------------------------------------
// Foreground resume
// ---------------------------------------------------------------------------

/// Notify that the app has returned to the foreground after being backgrounded.
///
/// On iOS, backgrounding suspends the process which kills UDP sockets. This
/// sets a flag to skip the next reconnect backoff delay and, if no reconnect
/// loop is already running, starts one immediately.
pub fn notify_foreground_resume(handle: &SessionHandle) {
    if handle.0.user_disconnect.load(Ordering::Acquire) {
        return;
    }

    // No stored endpoint → nothing to reconnect to
    if handle
        .0
        .endpoint_addr
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .is_none()
    {
        return;
    }

    if let Ok(mut reason) = handle.0.reconnect_reason.lock() {
        *reason = ReconnectReason::AppForegrounded;
    }
    handle.0.skip_next_backoff.store(true, Ordering::Release);

    if handle.is_reconnecting() {
        tracing::info!("foreground_resume: reconnect in progress, flagged to skip backoff");
        return;
    }

    tracing::info!("foreground_resume: triggering immediate reconnect");
    spawn_reconnect(handle.clone());
}

// ---------------------------------------------------------------------------
// RemoteSession
// ---------------------------------------------------------------------------

/// Per-terminal output buffers, keyed by terminal ID.
pub type TerminalOutputMap = Arc<Mutex<HashMap<String, OutputBuffer>>>;

/// Client-side handle to a remote zedra-host daemon.
///
/// Wraps an `irpc::Client<ZedraProto>` and provides typed accessors for every
/// RPC method. Terminal I/O uses TermAttach bidi streaming — output is pumped
/// into per-terminal `OutputBuffer`s and signals the main thread via
/// `signal_terminal_data()`.
pub struct RemoteSession {
    client: irpc::Client<ZedraProto>,
    state: Arc<Mutex<SessionState>>,
    /// Per-terminal output buffers (populated by TermAttach output pump).
    /// Shared with the SessionHandle so they persist across reconnects.
    terminal_outputs: TerminalOutputMap,
    /// All terminal IDs created on this session, in creation order.
    /// Shared with the SessionHandle.
    terminal_ids: Arc<Mutex<Vec<String>>>,
    /// Which terminal currently receives input from send_terminal_input().
    /// Shared with the SessionHandle.
    active_terminal_id: Arc<Mutex<Option<String>>>,
    /// Per-terminal input senders (tokio channels bridged to TermAttach streams).
    pub(crate) terminal_input_senders:
        Arc<Mutex<HashMap<String, tokio::sync::mpsc::Sender<Vec<u8>>>>>,
    /// Per-terminal last-seen seq numbers (for reconnect replay).
    terminal_last_seqs: Arc<Mutex<HashMap<String, u64>>>,
    session_id: Arc<Mutex<Option<String>>>,
    /// Latest ping RTT in milliseconds (0 = not yet measured).
    latency_ms: Arc<AtomicU64>,
    /// Connection path metadata (direct vs relay), updated by watcher task.
    connection_info: Arc<Mutex<Option<ConnectionInfo>>>,
}

impl RemoteSession {
    /// Connect to a zedra-host via iroh.
    ///
    /// Uses iroh's Endpoint for direct QUIC/UDP with automatic NAT traversal,
    /// relay fallback, and TLS 1.3 encryption. Terminal I/O uses TermAttach
    /// bidi streaming for efficient binary transfer.
    ///
    /// State is stored in the provided `SessionHandle` so it persists across
    /// reconnects and doesn't conflict with other workspaces.
    pub async fn connect_with_iroh(
        addr: iroh::EndpointAddr,
        handle: &SessionHandle,
    ) -> Result<Arc<Self>> {
        // Store endpoint address and host pubkey for future reconnect attempts
        handle.store_endpoint_addr(addr.clone());
        handle.store_endpoint_id(addr.id);

        // Reset user disconnect flag (we're intentionally connecting)
        handle.0.user_disconnect.store(false, Ordering::Release);

        tracing::info!(
            "RemoteSession: connecting via iroh (endpoint: {})",
            addr.id.fmt_short(),
        );

        // Fresh per-connection state — track phases as we progress
        let state = Arc::new(Mutex::new(SessionState::Connecting {
            phase: "Creating endpoint".into(),
        }));
        signal_terminal_data();

        // Build iroh endpoint (client side — generates ephemeral key).
        // No relay. pkarr resolver allows discovering the host's direct IPs
        // by pubkey alone via dns.iroh.link, enabling cross-network connections.
        let endpoint = iroh::Endpoint::builder()
            .relay_mode(iroh::RelayMode::Disabled)
            .alpns(vec![ZEDRA_ALPN.to_vec()])
            .address_lookup(iroh::address_lookup::PkarrResolver::n0_dns())
            .bind()
            .await?;
        tracing::info!("iroh client endpoint bound: {}", endpoint.id().fmt_short());

        if let Ok(mut s) = state.lock() {
            *s = SessionState::Connecting {
                phase: "QUIC handshake (direct P2P)".into(),
            };
        }
        signal_terminal_data();

        tracing::info!("Connecting to host endpoint: {:?}", addr);

        // Connect to host
        let conn = endpoint.connect(addr, ZEDRA_ALPN).await?;
        tracing::info!("iroh: connected to {}", conn.remote_id().fmt_short());

        if let Ok(mut s) = state.lock() {
            *s = SessionState::Connecting {
                phase: "Establishing RPC session".into(),
            };
        }
        signal_terminal_data();

        // Extract connection info before creating irpc client
        let local_eid = endpoint.id().fmt_short().to_string();
        let remote_eid = conn.remote_id().fmt_short().to_string();
        let alpn = String::from_utf8_lossy(conn.alpn()).to_string();
        let conn_for_paths = conn.clone();
        let conn_for_watcher = conn.clone();

        // Create typed irpc client from iroh connection
        let remote = irpc_iroh::IrohRemoteConnection::new(conn);
        let client = irpc::Client::<ZedraProto>::boxed(remote);

        // Use per-workspace state from the handle so terminal views survive reconnect
        let terminal_outputs = handle.terminal_outputs();
        let terminal_ids = handle.terminal_ids_slot();
        let active_terminal_id = handle.active_terminal_slot();

        let latency_ms = Arc::new(AtomicU64::new(0));
        let connection_info: Arc<Mutex<Option<ConnectionInfo>>> = Arc::new(Mutex::new(None));

        // Spawn path watcher to track direct vs relay connection
        {
            use iroh::Watcher;
            let mut paths = conn_for_paths.paths();
            let info_slot = connection_info.clone();
            let remote_eid = remote_eid.clone();
            tokio::spawn(async move {
                loop {
                    let path_list = paths.get();
                    let selected = path_list.iter().find(|p| p.is_selected());
                    if let Some(path) = selected {
                        let stats = path.stats();
                        let is_direct = path.is_ip();
                        let info = ConnectionInfo {
                            is_direct,
                            remote_addr: format!("{:?}", path.remote_addr()),
                            relay_url: None,
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
            terminal_outputs,
            terminal_ids,
            active_terminal_id,
            terminal_input_senders: Arc::new(Mutex::new(HashMap::new())),
            terminal_last_seqs: Arc::new(Mutex::new(HashMap::new())),
            session_id: Arc::new(Mutex::new(None)),
            latency_ms,
            connection_info,
        });

        // PKI authentication (Register on first connect; Auth+Prove on reconnect).
        //
        // The pending_ticket is set by `set_pending_ticket()` before connecting
        // when a QR code was scanned. On reconnect the slot is empty and we use
        // the stored session_id with Ed25519 challenge-response only.
        {
            let ticket = handle
                .0
                .pending_ticket
                .lock()
                .ok()
                .and_then(|mut g| g.take());
            let stored_session_id = handle.credentials();
            match handle.signer() {
                Some(signer) => {
                    // Safe: we called handle.store_endpoint_id(addr.id) above
                    let endpoint_id = handle
                        .stored_endpoint_id()
                        .expect("endpoint_id stored just above");
                    match Self::authenticate(
                        &session.client,
                        ticket.as_ref(),
                        signer.as_ref(),
                        &endpoint_id,
                        stored_session_id.as_deref(),
                    )
                    .await
                    {
                        Ok(sid) => {
                            if let Ok(mut slot) = session.session_id.lock() {
                                *slot = Some(sid.clone());
                            }
                            handle.store_credentials(Some(sid));
                        }
                        Err(e) => {
                            tracing::warn!("PKI auth failed: {}", e);
                        }
                    }
                }
                None => {
                    tracing::warn!("No signer on handle — skipping PKI auth (will fail RPC calls)");
                }
            }
        }

        // Fetch session info (hostname discovered here, not from QR)
        Self::fetch_session_info(&session, "unknown").await;

        // Re-attach existing terminals on reconnect
        Self::reattach_terminals(&session, handle).await;

        // Connection watcher: triggers reconnect when QUIC connection closes.
        // Uses a clone of the handle so reconnect targets the right workspace.
        let handle_for_watcher = handle.clone();
        tokio::spawn(async move {
            conn_for_watcher.closed().await;
            tracing::info!("iroh connection closed, triggering reconnect");
            if let Ok(mut reason) = handle_for_watcher.0.reconnect_reason.lock() {
                *reason = ReconnectReason::ConnectionLost;
            }
            spawn_reconnect(handle_for_watcher);
        });

        tracing::info!("RemoteSession: connected via iroh to {}", remote_eid);
        Ok(session)
    }

    /// Perform PKI authentication on the established QUIC connection.
    ///
    /// First connection (ticket provided):
    ///   Register → Authenticate → AuthProve
    ///
    /// Reconnect (ticket = None, uses stored session_id):
    ///   Authenticate → AuthProve
    ///
    /// Returns the authenticated session_id on success.
    async fn authenticate(
        client: &irpc::Client<ZedraProto>,
        ticket: Option<&zedra_rpc::ZedraPairingTicket>,
        signer: &dyn ClientSigner,
        endpoint_id: &iroh::PublicKey,
        session_id: Option<&str>,
    ) -> Result<String> {
        use ed25519_dalek::{Verifier, VerifyingKey};
        use std::time::{SystemTime, UNIX_EPOCH};

        let client_pubkey = signer.pubkey();

        // Step 1: Register (first connection only)
        if let Some(t) = ticket {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let hmac =
                zedra_rpc::compute_registration_hmac(&t.handshake_key, &client_pubkey, timestamp);
            match client
                .rpc(RegisterReq {
                    client_pubkey,
                    timestamp,
                    hmac,
                    slot_session_id: t.session_id.clone(),
                })
                .await?
            {
                RegisterResult::Ok => {
                    tracing::info!("PKI: registered with host, session={}", t.session_id);
                }
                other => {
                    return Err(anyhow::anyhow!("register failed: {:?}", other));
                }
            }
        }

        // Step 2: Authenticate — get nonce + host signature
        let challenge: AuthChallengeResult = client.rpc(AuthReq { client_pubkey }).await?;

        // Verify host signature before trusting the nonce
        {
            let vk_bytes = endpoint_id.as_bytes();
            let vk = VerifyingKey::from_bytes(vk_bytes)
                .map_err(|e| anyhow::anyhow!("invalid host pubkey: {e}"))?;
            let sig = ed25519_dalek::Signature::from_bytes(&challenge.host_signature);
            vk.verify(&challenge.nonce, &sig)
                .map_err(|_| anyhow::anyhow!("host challenge signature invalid"))?;
        }

        // Step 3: AuthProve — sign the nonce, specify target session
        let client_signature = signer.sign(&challenge.nonce);
        let attach_session_id = ticket
            .map(|t| t.session_id.clone())
            .or_else(|| session_id.map(|s| s.to_string()))
            .unwrap_or_default();

        match client
            .rpc(AuthProveReq {
                nonce: challenge.nonce,
                client_signature,
                session_id: attach_session_id.clone(),
            })
            .await?
        {
            AuthProveResult::Ok => {
                tracing::info!(
                    "PKI: authenticated, attached to session {}",
                    attach_session_id
                );
                Ok(attach_session_id)
            }
            other => Err(anyhow::anyhow!("auth prove failed: {:?}", other)),
        }
    }

    /// Fetch session info from the host and populate the session state.
    async fn fetch_session_info(session: &Arc<Self>, fallback_hostname: &str) {
        match session.client.rpc(SessionInfoReq {}).await {
            Ok(info) => {
                tracing::info!(
                    "Session info: host={}, user={}, workdir={}, os={}/{}",
                    info.hostname,
                    info.username,
                    info.workdir,
                    info.os.as_deref().unwrap_or("?"),
                    info.arch.as_deref().unwrap_or("?"),
                );
                // Store session_id if present in the response
                if let Some(ref sid) = info.session_id {
                    if let Ok(mut id_slot) = session.session_id.lock() {
                        *id_slot = Some(sid.clone());
                    }
                }
                if let Ok(mut s) = session.state.lock() {
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
                tracing::warn!("session/info failed: {e}");
                if let Ok(mut s) = session.state.lock() {
                    *s = SessionState::Connected {
                        hostname: fallback_hostname.to_string(),
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
    }

    /// Attach to a terminal via TermAttach bidi streaming.
    ///
    /// Spawns two bridge tasks:
    /// - Input bridge: reads from a tokio channel, forwards to irpc TermInput stream
    /// - Output pump: reads TermOutput from irpc stream, pushes to OutputBuffer
    async fn attach_terminal(
        &self,
        id: &str,
        last_seq: u64,
        _handle: &SessionHandle,
    ) -> Result<()> {
        let (irpc_input_tx, mut irpc_output_rx) = self
            .client
            .bidi_streaming::<TermAttachReq, TermInput, TermOutput>(
                TermAttachReq {
                    id: id.to_string(),
                    last_seq,
                },
                256,
                256,
            )
            .await?;

        // Create bridge channel for input (tokio mpsc → irpc sender)
        let (bridge_tx, mut bridge_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(256);

        // Store bridge sender for send_terminal_input()
        if let Ok(mut senders) = self.terminal_input_senders.lock() {
            senders.insert(id.to_string(), bridge_tx);
        }

        // Spawn input bridge: tokio channel → irpc TermInput stream
        tokio::spawn(async move {
            while let Some(data) = bridge_rx.recv().await {
                if let Err(e) = irpc_input_tx.send(TermInput { data }).await {
                    tracing::debug!("terminal input bridge closed: {e}");
                    break;
                }
            }
        });

        // Spawn output pump: irpc TermOutput stream → OutputBuffer
        let terminal_id = id.to_string();
        let outputs = self.terminal_outputs.clone();
        let seqs = self.terminal_last_seqs.clone();
        tokio::spawn(async move {
            loop {
                match irpc_output_rx.recv().await {
                    Ok(Some(output)) => {
                        // Track per-terminal seq for reconnect replay
                        if let Ok(mut seq_map) = seqs.lock() {
                            seq_map.insert(terminal_id.clone(), output.seq);
                        }
                        // Route to per-terminal buffer
                        let target_buf = {
                            let mut map = outputs.lock().unwrap();
                            map.entry(terminal_id.clone())
                                .or_insert_with(|| Arc::new(Mutex::new(VecDeque::new())))
                                .clone()
                        };
                        if let Ok(mut buf) = target_buf.lock() {
                            buf.push_back(output.data);
                        }
                        signal_terminal_data();
                    }
                    Ok(None) => {
                        tracing::debug!("terminal {} output stream ended", terminal_id);
                        break;
                    }
                    Err(e) => {
                        tracing::debug!("terminal {} output recv error: {e}", terminal_id);
                        break;
                    }
                }
            }
        });

        tracing::info!("Terminal {} attached (last_seq={})", id, last_seq);
        Ok(())
    }

    /// Re-attach all existing terminals on reconnect.
    ///
    /// Uses stored per-terminal last_seq values to replay missed output.
    async fn reattach_terminals(session: &Arc<Self>, handle: &SessionHandle) {
        let terminal_ids = session.terminal_ids();
        if terminal_ids.is_empty() {
            return;
        }

        tracing::info!("reattaching {} terminals", terminal_ids.len());

        for id in &terminal_ids {
            let last_seq = session
                .terminal_last_seqs
                .lock()
                .ok()
                .and_then(|map| map.get(id).copied())
                .unwrap_or(0);

            if let Err(e) = session.attach_terminal(id, last_seq, handle).await {
                tracing::warn!("failed to reattach terminal {}: {e}", id);
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

    /// The shared output buffer for the active terminal.
    ///
    /// Returns the per-terminal buffer if an active terminal is set,
    /// otherwise returns an empty buffer.
    pub fn output_buffer(&self) -> OutputBuffer {
        if let Some(id) = self.active_terminal_id() {
            if let Some(buf) = self.output_buffer_for(&id) {
                return buf;
            }
        }
        Arc::new(Mutex::new(VecDeque::new()))
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
        self.session_id.lock().ok().and_then(|guard| guard.clone())
    }

    /// Latest ping RTT in milliseconds (0 = not yet measured).
    pub fn latency_ms(&self) -> u64 {
        self.latency_ms.load(Ordering::Relaxed)
    }

    /// Connection path metadata (direct P2P vs relay).
    pub fn connection_info(&self) -> Option<ConnectionInfo> {
        self.connection_info.lock().ok().and_then(|g| g.clone())
    }

    /// Send a Ping RPC and measure the round-trip time.
    pub async fn ping(&self) -> Result<u64> {
        use std::time::{SystemTime, UNIX_EPOCH};
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let result: PongResult = self
            .client
            .rpc(PingReq {
                timestamp_ms: now_ms,
            })
            .await?;
        let after_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let rtt_ms = after_ms.saturating_sub(result.timestamp_ms);
        self.latency_ms.store(rtt_ms, Ordering::Relaxed);
        Ok(rtt_ms)
    }

    // -----------------------------------------------------------------------
    // Filesystem RPCs
    // -----------------------------------------------------------------------

    /// List directory entries at `path`.
    pub async fn fs_list(&self, path: &str) -> Result<Vec<FsEntry>> {
        let result: FsListResult = self
            .client
            .rpc(FsListReq {
                path: path.to_string(),
            })
            .await?;
        Ok(result.entries)
    }

    /// Read a file and return its contents as a string.
    pub async fn fs_read(&self, path: &str) -> Result<String> {
        let result: FsReadResult = self
            .client
            .rpc(FsReadReq {
                path: path.to_string(),
            })
            .await?;
        Ok(result.content)
    }

    /// Write `content` to a file at `path`.
    pub async fn fs_write(&self, path: &str, content: &str) -> Result<()> {
        let _: FsWriteResult = self
            .client
            .rpc(FsWriteReq {
                path: path.to_string(),
                content: content.to_string(),
            })
            .await?;
        Ok(())
    }

    /// Stat a file or directory at `path`.
    pub async fn fs_stat(&self, path: &str) -> Result<FsStatResult> {
        Ok(self
            .client
            .rpc(FsStatReq {
                path: path.to_string(),
            })
            .await?)
    }

    // -----------------------------------------------------------------------
    // Git RPCs
    // -----------------------------------------------------------------------

    /// Get the current git status (branch + changed files).
    pub async fn git_status(&self) -> Result<GitStatusResult> {
        Ok(self.client.rpc(GitStatusReq {}).await?)
    }

    /// Get a diff, optionally for a specific path and/or staged changes.
    pub async fn git_diff(&self, path: Option<&str>, staged: bool) -> Result<String> {
        let result: GitDiffResult = self
            .client
            .rpc(GitDiffReq {
                path: path.map(|s| s.to_string()),
                staged,
            })
            .await?;
        Ok(result.diff)
    }

    /// Get recent commit log entries.
    pub async fn git_log(&self, limit: Option<usize>) -> Result<Vec<GitLogEntry>> {
        let result: GitLogResult = self.client.rpc(GitLogReq { limit }).await?;
        Ok(result.entries)
    }

    /// List all branches.
    pub async fn git_branches(&self) -> Result<Vec<GitBranchEntry>> {
        let result: GitBranchesResult = self.client.rpc(GitBranchesReq {}).await?;
        Ok(result.branches)
    }

    /// Checkout a branch by name.
    pub async fn git_checkout(&self, branch: &str) -> Result<()> {
        let _: GitCheckoutResult = self
            .client
            .rpc(GitCheckoutReq {
                branch: branch.to_string(),
            })
            .await?;
        Ok(())
    }

    /// Commit staged changes (or specific paths) with the given message.
    /// Returns the commit hash.
    pub async fn git_commit(&self, message: &str, paths: &[String]) -> Result<String> {
        let result: GitCommitResult = self
            .client
            .rpc(GitCommitReq {
                message: message.to_string(),
                paths: paths.to_vec(),
            })
            .await?;
        Ok(result.hash)
    }

    // -----------------------------------------------------------------------
    // Terminal RPCs
    // -----------------------------------------------------------------------

    /// Create a new terminal on the remote host.
    /// Registers a per-terminal output buffer, attaches via bidi streaming,
    /// and sets as active if first terminal.
    pub async fn terminal_create(
        &self,
        cols: u16,
        rows: u16,
        handle: &SessionHandle,
    ) -> Result<String> {
        let result: TermCreateResult = self.client.rpc(TermCreateReq { cols, rows }).await?;

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

        // Attach to the terminal via bidi streaming
        self.attach_terminal(&result.id, 0, handle).await?;

        tracing::info!("Terminal created with id: {}", result.id);
        Ok(result.id)
    }

    /// Write data to the terminal via the TermAttach input channel.
    pub async fn terminal_write(&self, id: &str, data: &str) -> Result<()> {
        let sender = {
            let senders = self.terminal_input_senders.lock().unwrap();
            senders
                .get(id)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("terminal {} not attached", id))?
        };
        sender
            .send(data.as_bytes().to_vec())
            .await
            .map_err(|e| anyhow::anyhow!("terminal input send failed: {e}"))
    }

    /// Resize the terminal.
    pub async fn terminal_resize(&self, id: &str, cols: u16, rows: u16) -> Result<()> {
        let _: TermResizeResult = self
            .client
            .rpc(TermResizeReq {
                id: id.to_string(),
                cols,
                rows,
            })
            .await?;
        Ok(())
    }

    /// Close the active terminal.
    pub async fn terminal_close(&self) -> Result<()> {
        let id = self
            .active_terminal_id()
            .ok_or_else(|| anyhow::anyhow!("no active terminal to close"))?;

        let _: TermCloseResult = self.client.rpc(TermCloseReq { id: id.clone() }).await?;

        // Remove input sender
        if let Ok(mut senders) = self.terminal_input_senders.lock() {
            senders.remove(&id);
        }

        // Remove from terminal_ids
        if let Ok(mut ids) = self.terminal_ids.lock() {
            ids.retain(|i| i != &id);
        }

        // Remove per-terminal buffer
        if let Ok(mut map) = self.terminal_outputs.lock() {
            map.remove(&id);
        }

        // Remove seq tracking
        if let Ok(mut seqs) = self.terminal_last_seqs.lock() {
            seqs.remove(&id);
        }

        // If this was the active terminal, switch to the next available one
        if self.active_terminal_id() == Some(id) {
            let next = self
                .terminal_ids
                .lock()
                .ok()
                .and_then(|ids| ids.first().cloned());
            if let Ok(mut active) = self.active_terminal_id.lock() {
                *active = next;
            }
        }

        Ok(())
    }

    /// List active terminal IDs on the server (for reconnect reconciliation).
    pub async fn terminal_list(&self) -> Result<Vec<String>> {
        let result: TermListResult = self.client.rpc(TermListReq {}).await?;
        Ok(result.terminals.into_iter().map(|e| e.id).collect())
    }

    /// Attach to an existing terminal on the server (for cold-start session resume).
    ///
    /// Registers the terminal in the session's ID list and output buffer, then
    /// starts the bidi streaming pump. Uses `last_seq=0` to replay all available
    /// backlog output from the server.
    pub async fn terminal_attach_existing(&self, id: &str, handle: &SessionHandle) -> Result<()> {
        // Register per-terminal output buffer
        {
            let mut map = self.terminal_outputs.lock().unwrap();
            map.entry(id.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(VecDeque::new())));
        }

        // Append to terminal IDs list (if not already present)
        let is_first = {
            let mut ids = self.terminal_ids.lock().unwrap();
            let first = ids.is_empty();
            if !ids.contains(&id.to_string()) {
                ids.push(id.to_string());
            }
            first
        };

        // Set as active if it's the first terminal
        if is_first {
            if let Ok(mut active) = self.active_terminal_id.lock() {
                *active = Some(id.to_string());
            }
        }

        // Attach to the terminal via bidi streaming (last_seq=0 for full backlog replay)
        self.attach_terminal(id, 0, handle).await?;

        tracing::info!("Terminal attached (existing) id: {}", id);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Automatic reconnect (per-workspace)
// ---------------------------------------------------------------------------

/// Spawn a background task that attempts to reconnect after transport failure.
///
/// Guards:
/// - Does nothing if `user_disconnect` is set on the handle
/// - CAS on `reconnect_attempt` prevents concurrent reconnect loops
///
/// Uses exponential backoff (1s, 2s, 4s, 8s, 16s, 30s cap) with a maximum
/// of ~20 attempts (~5 minutes, matching the server's session grace period).
fn spawn_reconnect(handle: SessionHandle) {
    if handle.0.user_disconnect.load(Ordering::Acquire) {
        tracing::info!("spawn_reconnect: skipping, user disconnect in progress");
        return;
    }

    // CAS: only one reconnect loop at a time per handle
    if handle
        .0
        .reconnect_attempt
        .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Relaxed)
        .is_err()
    {
        tracing::info!("spawn_reconnect: already reconnecting");
        return;
    }

    session_runtime().spawn(async move {
        let max_attempts = 10u32;
        let mut attempt = 1u32;

        loop {
            if handle.0.user_disconnect.load(Ordering::Acquire) {
                tracing::info!("reconnect: user disconnect during reconnect, aborting");
                break;
            }

            if attempt > max_attempts {
                tracing::warn!(
                    "reconnect: max attempts ({}) reached, giving up",
                    max_attempts
                );
                break;
            }

            handle.0.reconnect_attempt.store(attempt, Ordering::Release);

            // Exponential backoff: 1s, 2s, 4s, 8s, 16s, 30s cap
            let delay_secs = std::cmp::min(1u64 << (attempt - 1), 30);
            let skip_backoff = handle.0.skip_next_backoff.swap(false, Ordering::AcqRel);
            let next_retry_secs = if skip_backoff { 0 } else { delay_secs };
            handle
                .0
                .next_retry_secs
                .store(next_retry_secs, Ordering::Release);
            signal_terminal_data(); // trigger UI refresh to show "Reconnecting..."

            if skip_backoff {
                tracing::info!(
                    "reconnect: attempt {} of {} (skipping {}s backoff — foreground resume)",
                    attempt,
                    max_attempts,
                    delay_secs,
                );
            } else {
                tracing::info!(
                    "reconnect: attempt {} of {} (backoff {}s)",
                    attempt,
                    max_attempts,
                    delay_secs,
                );
                // Tick down next_retry_secs each second for live UI updates
                for remaining in (1..=delay_secs).rev() {
                    handle.0.next_retry_secs.store(remaining, Ordering::Release);
                    signal_terminal_data();
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    if handle.0.user_disconnect.load(Ordering::Acquire) {
                        break;
                    }
                }
                handle.0.next_retry_secs.store(0, Ordering::Release);
            }

            // Check again after sleep
            if handle.0.user_disconnect.load(Ordering::Acquire) {
                tracing::info!("reconnect: user disconnect during backoff, aborting");
                break;
            }

            // Get stored endpoint address from this handle
            let addr = match handle.0.endpoint_addr.lock().ok().and_then(|g| g.clone()) {
                Some(a) => a,
                None => {
                    tracing::error!("reconnect: no stored endpoint address, aborting");
                    break;
                }
            };

            match RemoteSession::connect_with_iroh(addr, &handle).await {
                Ok(session) => {
                    tracing::info!("reconnect: success on attempt {}", attempt);
                    handle.set_session(session.clone());

                    // Verify server-side terminals are still alive
                    match session.terminal_list().await {
                        Ok(ids) => {
                            tracing::info!(
                                "reconnect: server has {} terminals: {:?}",
                                ids.len(),
                                ids,
                            );
                        }
                        Err(e) => {
                            tracing::warn!("reconnect: terminal_list failed: {}", e);
                        }
                    }

                    handle.0.reconnect_attempt.store(0, Ordering::Release);
                    signal_terminal_data(); // trigger UI refresh
                    return; // success — don't fall through to reset below
                }
                Err(e) => {
                    tracing::warn!("reconnect: attempt {} failed: {}", attempt, e);
                    attempt += 1;
                }
            }
        }

        // Gave up or aborted
        handle.0.reconnect_attempt.store(0, Ordering::Release);
        // If we exhausted all attempts (not user-initiated abort), mark unreachable
        if !handle.0.user_disconnect.load(Ordering::Acquire) && attempt > max_attempts {
            tracing::warn!(
                "reconnect: {} attempts exhausted, marking host unreachable",
                max_attempts
            );
            handle.0.host_unreachable.store(true, Ordering::Release);
        }
        signal_terminal_data();
    });
}
