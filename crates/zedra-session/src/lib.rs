// zedra-session: RemoteSession client library for connecting to a zedra-host RPC daemon.
//
// Uses irpc typed RPC over iroh QUIC streams. Terminal I/O uses bidi streaming
// (TermAttach) for efficient binary data transfer without base64 encoding.
//
// Bridges async RPC calls to the GPUI main thread using a global-state pattern
// (OutputBuffer, AtomicBool signaling, OnceLock singletons).
//
// Usage:
//   1. Call RemoteSession::connect_with_iroh(addr) on the session runtime
//   2. Store the result via set_active_session()
//   3. Main thread polls check_and_clear_terminal_data() each frame
//   4. Main thread drains drain_callbacks() each frame for deferred work

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::Result;

use zedra_rpc::proto::*;
use zedra_rpc::DEFAULT_RELAY_URL;

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
    Reconnecting {
        attempt: u32,
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
// Global state
// ---------------------------------------------------------------------------

/// Atomic flag: set by the terminal output pump when TermOutput arrives.
/// Polled by the main-thread frame loop to trigger terminal refreshes.
pub static TERMINAL_DATA_PENDING: AtomicBool = AtomicBool::new(false);

/// Dedicated tokio runtime for session I/O (2 worker threads).
static SESSION_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

/// Singleton slot for the currently-active remote session.
static ACTIVE_SESSION: OnceLock<Mutex<Option<Arc<RemoteSession>>>> = OnceLock::new();

/// Queue of callbacks to be drained and executed on the main thread.
static MAIN_THREAD_CALLBACKS: OnceLock<Mutex<VecDeque<MainCallback>>> = OnceLock::new();

// ---------------------------------------------------------------------------
// Reconnect state (persists across RemoteSession rebuilds)
// ---------------------------------------------------------------------------

/// Current reconnect attempt number (0 = not reconnecting).
static RECONNECT_ATTEMPT: AtomicU32 = AtomicU32::new(0);

/// Set on user-initiated disconnect to prevent automatic reconnect.
static USER_DISCONNECT: AtomicBool = AtomicBool::new(false);

/// Highest notification seq successfully processed (for backlog resumption).
static LAST_NOTIF_SEQ: AtomicU64 = AtomicU64::new(0);

/// Stored endpoint address for reconnect attempts.
static ENDPOINT_ADDR: OnceLock<Mutex<Option<iroh::EndpointAddr>>> = OnceLock::new();

/// (session_id, auth_token) preserved across RemoteSession rebuilds.
static SESSION_CREDENTIALS: OnceLock<Mutex<(Option<String>, Option<String>)>> = OnceLock::new();

/// Shared terminal output buffers that survive reconnect.
static PERSISTENT_TERMINAL_OUTPUTS: OnceLock<TerminalOutputMap> = OnceLock::new();

/// Terminal ID list that survives reconnect.
static PERSISTENT_TERMINAL_IDS: OnceLock<Arc<Mutex<Vec<String>>>> = OnceLock::new();

/// Active terminal ID that survives reconnect.
static PERSISTENT_ACTIVE_TERMINAL: OnceLock<Arc<Mutex<Option<String>>>> = OnceLock::new();

fn endpoint_addr_slot() -> &'static Mutex<Option<iroh::EndpointAddr>> {
    ENDPOINT_ADDR.get_or_init(|| Mutex::new(None))
}

fn session_credentials_slot() -> &'static Mutex<(Option<String>, Option<String>)> {
    SESSION_CREDENTIALS.get_or_init(|| Mutex::new((None, None)))
}

fn persistent_terminal_outputs() -> TerminalOutputMap {
    PERSISTENT_TERMINAL_OUTPUTS
        .get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
        .clone()
}

fn persistent_terminal_ids() -> Arc<Mutex<Vec<String>>> {
    PERSISTENT_TERMINAL_IDS
        .get_or_init(|| Arc::new(Mutex::new(Vec::new())))
        .clone()
}

fn persistent_active_terminal() -> Arc<Mutex<Option<String>>> {
    PERSISTENT_ACTIVE_TERMINAL
        .get_or_init(|| Arc::new(Mutex::new(None)))
        .clone()
}

/// Current reconnect attempt number (0 = not reconnecting).
pub fn reconnect_attempt() -> u32 {
    RECONNECT_ATTEMPT.load(Ordering::Relaxed)
}

/// Whether a reconnect is currently in progress.
pub fn is_reconnecting() -> bool {
    reconnect_attempt() > 0
}

/// Store an endpoint address for use during automatic reconnect.
pub fn store_endpoint_addr(addr: iroh::EndpointAddr) {
    if let Ok(mut slot) = endpoint_addr_slot().lock() {
        *slot = Some(addr);
    }
}

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

/// Clear the active session (user-initiated disconnect).
///
/// Sets `USER_DISCONNECT` to prevent automatic reconnect attempts.
pub fn clear_active_session() {
    USER_DISCONNECT.store(true, Ordering::Release);
    if let Ok(mut slot) = active_session_slot().lock() {
        *slot = None;
        tracing::info!("Active remote session cleared (user disconnect)");
    }
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

/// Send terminal input to the remote host via the active session.
///
/// Sends raw bytes through the TermAttach bidi stream's input channel.
/// Returns `true` if the data was successfully enqueued.
pub fn send_terminal_input(data: Vec<u8>) -> bool {
    let session = match active_session() {
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

    // Non-blocking send from main thread
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
    terminal_outputs: TerminalOutputMap,
    /// All terminal IDs created on this session, in creation order.
    terminal_ids: Arc<Mutex<Vec<String>>>,
    /// Which terminal currently receives input from send_terminal_input().
    active_terminal_id: Arc<Mutex<Option<String>>>,
    /// Per-terminal input senders (tokio channels bridged to TermAttach streams).
    terminal_input_senders: Arc<Mutex<HashMap<String, tokio::sync::mpsc::Sender<Vec<u8>>>>>,
    /// Per-terminal last-seen seq numbers (for reconnect replay).
    terminal_last_seqs: Arc<Mutex<HashMap<String, u64>>>,
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
    /// relay fallback, and TLS 1.3 encryption. Terminal I/O uses TermAttach
    /// bidi streaming for efficient binary transfer.
    ///
    /// On first connect, creates fresh persistent state. On reconnect, reuses
    /// existing terminal output buffers and IDs so UI views survive.
    pub async fn connect_with_iroh(addr: iroh::EndpointAddr) -> Result<Arc<Self>> {
        // Store endpoint address for future reconnect attempts
        store_endpoint_addr(addr.clone());

        // Reset user disconnect flag (we're intentionally connecting)
        USER_DISCONNECT.store(false, Ordering::Release);

        tracing::info!(
            "RemoteSession: connecting via iroh (endpoint: {})",
            addr.id.fmt_short(),
        );

        // Build iroh endpoint (client side — generates ephemeral key)
        let relay_url: iroh::RelayUrl = DEFAULT_RELAY_URL
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid relay URL: {}", e))?;
        tracing::info!("Using relay: {}", relay_url);

        let endpoint = iroh::Endpoint::builder()
            .relay_mode(iroh::RelayMode::custom([relay_url]))
            .alpns(vec![ZEDRA_ALPN.to_vec()])
            .bind()
            .await?;
        tracing::info!("iroh client endpoint bound: {}", endpoint.id().fmt_short());

        tracing::info!("Connecting to host endpoint: {:?}", addr);

        // Connect to host
        let conn = endpoint.connect(addr, ZEDRA_ALPN).await?;
        tracing::info!("iroh: connected to {}", conn.remote_id().fmt_short());

        // Extract connection info before creating irpc client
        let local_eid = endpoint.id().fmt_short().to_string();
        let remote_eid = conn.remote_id().fmt_short().to_string();
        let alpn = String::from_utf8_lossy(conn.alpn()).to_string();
        let conn_for_paths = conn.clone();
        let conn_for_watcher = conn.clone();

        // Create typed irpc client from iroh connection
        let remote = irpc_iroh::IrohRemoteConnection::new(conn);
        let client = irpc::Client::<ZedraProto>::boxed(remote);

        // Use persistent state for terminal outputs/IDs so UI views survive reconnect
        let terminal_outputs = persistent_terminal_outputs();
        let terminal_ids = persistent_terminal_ids();
        let active_terminal_id = persistent_active_terminal();

        // Fresh per-connection state
        let state = Arc::new(Mutex::new(SessionState::Connecting));
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
            terminal_outputs,
            terminal_ids,
            active_terminal_id,
            terminal_input_senders: Arc::new(Mutex::new(HashMap::new())),
            terminal_last_seqs: Arc::new(Mutex::new(HashMap::new())),
            session_id: Arc::new(Mutex::new(None)),
            auth_token: Arc::new(Mutex::new(None)),
            latency_ms,
            connection_info,
        });

        // Establish RPC session (uses SESSION_CREDENTIALS for reconnect)
        let resumed = Self::establish_rpc_session(&session).await;

        // Fetch session info (hostname discovered here, not from QR)
        Self::fetch_session_info(&session, "unknown").await;

        if resumed {
            // Resumed existing session: discover server-side terminals and attach
            Self::discover_and_attach_terminals(&session).await;
        }

        // Re-attach any client-side terminals not covered by discovery
        Self::reattach_terminals(&session).await;

        // Connection watcher: triggers reconnect when QUIC connection closes
        tokio::spawn(async move {
            conn_for_watcher.closed().await;
            tracing::info!("iroh connection closed, triggering reconnect");
            spawn_reconnect();
        });

        tracing::info!("RemoteSession: connected via iroh to {}", remote_eid);
        Ok(session)
    }

    /// Establish an RPC session on the host via ResumeOrCreate.
    ///
    /// On first call, creates a new session. On reconnect, resumes the existing
    /// session using credentials from `SESSION_CREDENTIALS`.
    ///
    /// Returns `true` if an existing session was resumed.
    async fn establish_rpc_session(session: &Arc<Self>) -> bool {
        let (stored_session_id, stored_auth_token) = session_credentials_slot()
            .lock()
            .map(|g| g.clone())
            .unwrap_or((None, None));

        let auth_token = stored_auth_token.unwrap_or_else(|| {
            format!(
                "{:016x}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            )
        });

        let session_id_to_resume = stored_session_id.or_else(|| session.session_id());

        match session
            .client
            .rpc(ResumeOrCreateReq {
                session_id: session_id_to_resume,
                auth_token: auth_token.clone(),
                last_notif_seq: LAST_NOTIF_SEQ.load(Ordering::Relaxed),
            })
            .await
        {
            Ok(result) => {
                tracing::info!(
                    "RPC session {}: id={}",
                    if result.resumed { "resumed" } else { "created" },
                    result.session_id,
                );

                // Store in instance fields
                if let Ok(mut id_slot) = session.session_id.lock() {
                    *id_slot = Some(result.session_id.clone());
                }
                if let Ok(mut token_slot) = session.auth_token.lock() {
                    *token_slot = Some(auth_token.clone());
                }

                // Persist credentials for future reconnects
                if let Ok(mut creds) = session_credentials_slot().lock() {
                    *creds = (Some(result.session_id), Some(auth_token));
                }

                return result.resumed;
            }
            Err(e) => {
                tracing::warn!("session/resume_or_create failed: {}", e);
            }
        }
        false
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
    async fn attach_terminal(&self, id: &str, last_seq: u64) -> Result<()> {
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
                        LAST_NOTIF_SEQ.fetch_max(output.seq, Ordering::Release);

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

    /// Discover server-side terminals via TermList and register+attach any
    /// that the client doesn't already know about.
    ///
    /// Called after ResumeOrCreate returns `resumed=true`. This is the key
    /// method for fresh client terminal recovery — it populates the client's
    /// terminal ID list and output buffers from server state.
    async fn discover_and_attach_terminals(session: &Arc<Self>) {
        let server_terminals = match session.terminal_list().await {
            Ok(ids) => ids,
            Err(e) => {
                tracing::warn!("terminal discovery failed: {e}");
                return;
            }
        };

        if server_terminals.is_empty() {
            tracing::info!("discover: no server-side terminals");
            return;
        }

        let client_ids = session.terminal_ids();
        tracing::info!(
            "discover: server has {} terminals, client knows {}",
            server_terminals.len(),
            client_ids.len(),
        );

        for server_tid in &server_terminals {
            if client_ids.contains(server_tid) {
                continue; // Already known, will be handled by reattach_terminals
            }

            tracing::info!("discover: registering new terminal {}", server_tid);

            // Register per-terminal output buffer
            if let Ok(mut map) = session.terminal_outputs.lock() {
                map.entry(server_tid.clone())
                    .or_insert_with(|| Arc::new(Mutex::new(VecDeque::new())));
            }

            // Add to terminal IDs list
            if let Ok(mut ids) = session.terminal_ids.lock() {
                if !ids.contains(server_tid) {
                    ids.push(server_tid.clone());
                }
            }

            // Set as active if it's the first terminal
            if client_ids.is_empty() {
                if let Ok(mut active) = session.active_terminal_id.lock() {
                    if active.is_none() {
                        *active = Some(server_tid.clone());
                    }
                }
            }

            // Attach with last_seq=0 (fresh — server sends vt100 screen dump)
            if let Err(e) = session.attach_terminal(server_tid, 0).await {
                tracing::warn!("discover: failed to attach terminal {}: {e}", server_tid);
            }
        }

        // Remove client-side terminals that no longer exist on the server
        let stale: Vec<String> = client_ids
            .iter()
            .filter(|id| !server_terminals.contains(id))
            .cloned()
            .collect();
        for id in &stale {
            tracing::info!("discover: removing stale terminal {}", id);
            if let Ok(mut ids) = session.terminal_ids.lock() {
                ids.retain(|i| i != id);
            }
            if let Ok(mut map) = session.terminal_outputs.lock() {
                map.remove(id);
            }
            if let Ok(mut senders) = session.terminal_input_senders.lock() {
                senders.remove(id);
            }
            if let Ok(mut seqs) = session.terminal_last_seqs.lock() {
                seqs.remove(id);
            }
        }
    }

    /// Re-attach all existing terminals on reconnect.
    ///
    /// Uses stored per-terminal last_seq values to replay missed output.
    /// Skips terminals that already have an active input sender (attached
    /// by `discover_and_attach_terminals`).
    async fn reattach_terminals(session: &Arc<Self>) {
        let terminal_ids = session.terminal_ids();
        if terminal_ids.is_empty() {
            return;
        }

        tracing::info!("reattaching {} terminals", terminal_ids.len());
        for id in &terminal_ids {
            // Skip if already attached (e.g., by discover_and_attach_terminals)
            let already_attached = session
                .terminal_input_senders
                .lock()
                .ok()
                .map(|senders| senders.contains_key(id))
                .unwrap_or(false);
            if already_attached {
                tracing::debug!("reattach: skipping {} (already attached)", id);
                continue;
            }

            let last_seq = session
                .terminal_last_seqs
                .lock()
                .ok()
                .and_then(|map| map.get(id).copied())
                .unwrap_or(0);

            if let Err(e) = session.attach_terminal(id, last_seq).await {
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

    /// Send a heartbeat RPC and measure the round-trip time.
    pub async fn ping(&self) -> Result<u64> {
        let start = std::time::Instant::now();
        let _: HeartbeatResult = self.client.rpc(HeartbeatReq {}).await?;
        let rtt_ms = start.elapsed().as_millis() as u64;
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
    pub async fn terminal_create(&self, cols: u16, rows: u16) -> Result<String> {
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
        self.attach_terminal(&result.id, 0).await?;

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

        let _: TermCloseResult = self
            .client
            .rpc(TermCloseReq { id: id.clone() })
            .await?;

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

    /// List active terminals on the server with full metadata.
    pub async fn terminal_list_full(&self) -> Result<Vec<TermListEntry>> {
        let result: TermListResult = self.client.rpc(TermListReq {}).await?;
        Ok(result.terminals)
    }
}

// ---------------------------------------------------------------------------
// Automatic reconnect
// ---------------------------------------------------------------------------

/// Spawn a background task that attempts to reconnect after transport failure.
///
/// Guards:
/// - Does nothing if `USER_DISCONNECT` is set (user initiated disconnect)
/// - CAS on `RECONNECT_ATTEMPT` prevents concurrent reconnect loops
///
/// Uses exponential backoff (1s, 2s, 4s, 8s, 16s, 30s cap) with a maximum
/// of ~20 attempts (~5 minutes, matching the server's session grace period).
fn spawn_reconnect() {
    if USER_DISCONNECT.load(Ordering::Acquire) {
        tracing::info!("spawn_reconnect: skipping, user disconnect in progress");
        return;
    }

    // CAS: only one reconnect loop at a time
    if RECONNECT_ATTEMPT
        .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Relaxed)
        .is_err()
    {
        tracing::info!("spawn_reconnect: already reconnecting");
        return;
    }

    tokio::spawn(async move {
        let max_attempts = 20u32;
        let mut attempt = 1u32;

        loop {
            if USER_DISCONNECT.load(Ordering::Acquire) {
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

            RECONNECT_ATTEMPT.store(attempt, Ordering::Release);
            signal_terminal_data(); // trigger UI refresh to show "Reconnecting..."

            // Exponential backoff: 1s, 2s, 4s, 8s, 16s, 30s cap
            let delay_secs = std::cmp::min(1u64 << (attempt - 1), 30);
            tracing::info!(
                "reconnect: attempt {} of {} (backoff {}s)",
                attempt,
                max_attempts,
                delay_secs,
            );
            tokio::time::sleep(std::time::Duration::from_secs(delay_secs)).await;

            // Check again after sleep
            if USER_DISCONNECT.load(Ordering::Acquire) {
                tracing::info!("reconnect: user disconnect during backoff, aborting");
                break;
            }

            // Get stored endpoint address
            let addr = match endpoint_addr_slot().lock().ok().and_then(|g| g.clone()) {
                Some(a) => a,
                None => {
                    tracing::error!("reconnect: no stored endpoint address, aborting");
                    break;
                }
            };

            match RemoteSession::connect_with_iroh(addr).await {
                Ok(session) => {
                    tracing::info!(
                        "reconnect: success on attempt {}, terminals={}",
                        attempt,
                        session.terminal_ids().len(),
                    );
                    set_active_session(session.clone());

                    RECONNECT_ATTEMPT.store(0, Ordering::Release);
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
        RECONNECT_ATTEMPT.store(0, Ordering::Release);
        signal_terminal_data();
    });
}
