// Session registry: manages persistent server sessions that survive transport
// reconnections. Each session owns its terminal PTY sessions and notification
// backlog, allowing a mobile client to disconnect and reconnect without losing
// state.
//
// v3 (Phase 1 PKI): Auth tokens removed. Sessions use per-session ACLs of
// authorized client public keys. One active client per session at a time.
// Pairing slots (one-use handshake keys) are used for first registration.

use crate::docs_tree::{
    snapshot_page_result, DocsTreeCacheEntry, DocsTreeSnapshot, DOCS_TREE_CACHE_TTL,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use uuid::Uuid;
use zedra_osc::{OscEvent, OscScanner};
use zedra_rpc::proto::{
    BacklogEntry, FsDocsTreeError, FsDocsTreeResult, HostEvent, TermOutput, TerminalSyncEntry,
};

// ---------------------------------------------------------------------------
// Pairing slot
// ---------------------------------------------------------------------------

/// A one-use registration slot created when `zedra qr` is run.
///
/// The host stores the random `handshake_secret` locally. The QR ticket carries
/// the same secret for the client to produce an HMAC. On first use the slot is
/// consumed; the client pubkey is added to the session ACL.
#[derive(Clone)]
pub struct PairingSlot {
    /// Random 16-byte secret embedded in the QR ticket.
    pub handshake_secret: [u8; 16],
    /// Session the new client should be added to.
    pub session_id: String,
    /// When this slot expires (10 minutes after creation).
    pub expires_at: Instant,
}

/// Result of attempting to atomically consume a pairing slot.
pub enum ConsumeSlotResult {
    /// Slot was active — client may proceed with HMAC verification.
    Active(PairingSlot),
    /// Slot was already used by another device.
    Consumed,
    /// No slot found for this session ID.
    NotFound,
}

// ---------------------------------------------------------------------------
// ServerSession
// ---------------------------------------------------------------------------

/// A server-side session that persists across transport reconnections.
pub struct ServerSession {
    pub id: String,
    /// Human-readable session name (e.g., "zedra", "webapp").
    pub name: Option<String>,
    /// Working directory for this session.
    pub workdir: Option<PathBuf>,
    pub created_at: Instant,
    pub last_activity: Mutex<Instant>,
    pub terminals: Mutex<HashMap<String, TermSession>>,
    pub terminal_order: Mutex<Vec<String>>,
    /// Client pubkeys authorized to attach to this session (per-session ACL).
    pub acl: Mutex<HashSet<[u8; 32]>>,
    /// Currently attached client pubkey. None = session is free.
    pub active_client: Mutex<Option<[u8; 32]>>,
    /// Ephemeral in-memory session token for the currently attached client.
    /// At most one token exists per session at a time (bound to the active
    /// client pubkey). Rotated on every successful connect. Consumed atomically
    /// on validation to prevent replay within the TTL window.
    pub session_token: Mutex<Option<([u8; 32], SessionToken)>>,
    /// Channel for pushing host-initiated events to the connected client.
    /// Installed by the Subscribe RPC handler; replaced on each new subscription.
    pub event_tx: Mutex<Option<tokio::sync::mpsc::Sender<HostEvent>>>,
    /// Relative directory paths watched by the observer for this session.
    pub fs_watched_paths: Mutex<HashSet<String>>,
    /// Token bucket for FsWatch/FsUnwatch RPC rate limiting.
    pub fs_watch_rpc_limiter: Mutex<TokenBucket>,
    /// One bounded docs-tree snapshot per session, replaced by manual rebuild.
    pub docs_tree_cache: Mutex<Option<DocsTreeCacheEntry>>,
    /// Prevents repeated rebuild requests from starting overlapping filesystem scans.
    pub docs_tree_scan_in_flight: AtomicBool,
    // ── RPC usage counters (lifetime totals, never reset) ──────────────────
    /// Total FsRead calls served.
    pub rpc_fs_reads: AtomicU64,
    /// Total FsWrite calls served.
    pub rpc_fs_writes: AtomicU64,
    /// Total read-only git RPC calls (status, diff, log, branches, stage, unstage).
    pub rpc_git_ops: AtomicU64,
    /// Total GitCommit calls that succeeded.
    pub rpc_git_commits: AtomicU64,
    /// Total AiPrompt calls served.
    pub rpc_ai_prompts: AtomicU64,

    /// Observer generation; incremented on each Subscribe to stop stale observers.
    pub observer_gen: AtomicU64,
    /// Observer/event metrics for abuse and backpressure visibility.
    pub observer_events_sent: AtomicU64,
    pub observer_events_dropped_no_subscriber: AtomicU64,
    pub observer_events_dropped_full: AtomicU64,
    pub fs_watch_quota_rejected: AtomicU64,
    pub fs_watch_rate_limited: AtomicU64,
}

#[derive(Clone)]
pub struct SessionToken {
    pub token: [u8; 32],
}

/// Max number of observed paths stored per session.
pub const MAX_WATCHED_PATHS_PER_SESSION: usize = 128;
/// FsWatch/FsUnwatch token bucket refill rate.
pub const FS_WATCH_RPC_RATE_PER_SEC: f64 = 10.0;
/// FsWatch/FsUnwatch token bucket burst capacity.
pub const FS_WATCH_RPC_BURST: f64 = 20.0;

/// Lightweight token bucket limiter used for watch/unwatch control calls.
pub struct TokenBucket {
    tokens: f64,
    last_refill: Instant,
    rate_per_sec: f64,
    burst: f64,
}

impl TokenBucket {
    fn new(rate_per_sec: f64, burst: f64) -> Self {
        Self {
            tokens: burst,
            last_refill: Instant::now(),
            rate_per_sec,
            burst,
        }
    }

    fn allow(&mut self) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.last_refill = now;
        self.tokens = (self.tokens + elapsed * self.rate_per_sec).min(self.burst);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// Guards the swappable PTY output sender against stale cleanup.
///
/// `gen` is incremented each time a new TermAttach installs a sender.
/// Cleanup code compares its captured generation before clearing to avoid
/// clobbering a sender that was installed by a newer TermAttach connection.
pub struct OutputSenderSlot {
    pub gen: u64,
    pub sender: Option<tokio::sync::mpsc::Sender<TermOutput>>,
}

/// Per-terminal OSC metadata tracked by the host PTY reader.
///
/// Updated in real-time as PTY output flows through, so the host always has
/// the latest known terminal metadata regardless of backlog eviction.
pub struct HostTermMeta {
    pub scanner: OscScanner,
    pub title: Option<String>,
    pub icon_name: Option<String>,
    pub cwd: Option<String>,
    pub current_command: Option<String>,
    pub shell_state: HostShellState,
    pub last_exit_code: Option<i32>,
}

impl Default for HostTermMeta {
    fn default() -> Self {
        Self {
            scanner: OscScanner::new(),
            title: None,
            icon_name: None,
            cwd: None,
            current_command: None,
            shell_state: HostShellState::Unknown,
            last_exit_code: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum HostShellState {
    #[default]
    Unknown,
    Idle,
    Running,
}

impl HostTermMeta {
    pub fn apply_osc_event(&mut self, event: &OscEvent) {
        match event {
            OscEvent::Title(title) => self.title = Some(title.clone()),
            OscEvent::ResetTitle => self.title = None,
            OscEvent::IconName(name) => self.icon_name = Some(name.clone()),
            OscEvent::Cwd(cwd) => self.cwd = Some(cwd.clone()),
            OscEvent::CommandLine(command) => self.current_command = Some(command.clone()),
            OscEvent::CommandStart => self.shell_state = HostShellState::Running,
            OscEvent::CommandEnd { exit_code } => {
                self.shell_state = HostShellState::Idle;
                self.last_exit_code = Some(*exit_code);
                self.current_command = None;
            }
            OscEvent::PromptReady => {
                self.shell_state = HostShellState::Idle;
                self.current_command = None;
            }
            _ => {}
        }
    }
}

/// Per-terminal output backlog for replay on TermAttach reconnect.
///
/// Lives inside `TermSession` (one per terminal) so PTY readers never contend
/// across terminals. Uses `std::sync::Mutex` so the `spawn_blocking` PTY
/// reader can push entries without `rt.block_on`.
pub struct TermBacklog {
    pub entries: VecDeque<BacklogEntry>,
    pub next_seq: u64,
}

pub struct TermBacklogReplay {
    pub entries: Vec<BacklogEntry>,
    pub oldest_seq: Option<u64>,
    pub newest_seq: u64,
    pub retained_entries: usize,
    pub retained_bytes: usize,
}

impl TermBacklog {
    pub fn new() -> Self {
        Self {
            entries: VecDeque::new(),
            next_seq: 1,
        }
    }

    /// Allocate a sequence number, push the entry, evict oldest if over cap.
    /// Returns the allocated sequence number.
    pub fn push(&mut self, terminal_id: String, data: Vec<u8>) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.entries.push_back(BacklogEntry {
            seq,
            terminal_id,
            data,
        });
        while self.entries.len() > 1000 {
            self.entries.pop_front();
        }
        seq
    }

    /// Return all entries with seq > `after_seq`.
    pub fn after(&self, after_seq: u64) -> Vec<BacklogEntry> {
        self.entries
            .iter()
            .filter(|e| e.seq > after_seq)
            .cloned()
            .collect()
    }

    pub fn replay_after(&self, after_seq: u64) -> TermBacklogReplay {
        TermBacklogReplay {
            entries: self.after(after_seq),
            oldest_seq: self.entries.front().map(|entry| entry.seq),
            newest_seq: self.next_seq.saturating_sub(1),
            retained_entries: self.entries.len(),
            retained_bytes: self.entries.iter().map(|entry| entry.data.len()).sum(),
        }
    }
}

/// A live terminal session owned by a ServerSession.
pub struct TermSession {
    /// PTY writer in a mutex so the `TermAttach` input loop can hold a direct
    /// clone of the Arc and write without re-acquiring `session.terminals` on
    /// every keystroke.
    pub writer: Arc<std::sync::Mutex<Box<dyn Write + Send>>>,
    pub master: Box<dyn portable_pty::MasterPty + Send>,
    pub child: Box<dyn portable_pty::Child + Send + Sync>,
    /// Swappable output sender. Updated on each TermAttach.
    pub output_sender: Arc<std::sync::Mutex<OutputSenderSlot>>,
    /// Host-side OSC metadata cache. Updated by the PTY reader
    /// task as output bytes flow through. Used to seed the client on attach.
    pub host_meta: Arc<std::sync::Mutex<HostTermMeta>>,
    /// Per-terminal output backlog (seq + replay entries).
    pub backlog: Arc<std::sync::Mutex<TermBacklog>>,
    /// Wall-clock creation time for operator-facing status output.
    pub created_at: SystemTime,
    /// Monotonic creation time for terminal uptime calculations.
    pub started_at: Instant,
}

impl TermSession {
    pub fn terminate(mut self) -> bool {
        match self.child.try_wait() {
            Ok(Some(_)) => return true,
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(err = %e, "failed to inspect terminal child before close");
            }
        }

        if let Err(e) = self.child.kill() {
            tracing::warn!(err = %e, "failed to terminate terminal child");
            return false;
        }

        if let Err(e) = self.child.wait() {
            tracing::warn!(err = %e, "failed to reap terminal child after close");
        }

        true
    }
}

fn terminal_created_at_key(created_at: SystemTime) -> Duration {
    created_at.duration_since(UNIX_EPOCH).unwrap_or_default()
}

fn ordered_terminal_ids_locked(
    terms: &HashMap<String, TermSession>,
    order: &mut Vec<String>,
) -> Vec<String> {
    ordered_terminal_ids_from_entries(
        terms
            .iter()
            .map(|(id, term)| (id.clone(), terminal_created_at_key(term.created_at))),
        order,
    )
}

fn ordered_terminal_ids_from_entries(
    entries: impl IntoIterator<Item = (String, Duration)>,
    order: &mut Vec<String>,
) -> Vec<String> {
    let entries = entries.into_iter().collect::<Vec<_>>();
    let active_ids = entries
        .iter()
        .map(|(id, _)| id.as_str())
        .collect::<HashSet<_>>();

    order.retain(|id| active_ids.contains(id.as_str()));
    let mut ordered = order.clone();

    let mut missing = {
        let known = order.iter().map(String::as_str).collect::<HashSet<_>>();
        entries
            .into_iter()
            .filter(|(id, _)| !known.contains(id.as_str()))
            .map(|(id, created_at)| (created_at, id))
            .collect::<Vec<_>>()
    };
    missing.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));

    for (_, id) in missing {
        order.push(id.clone());
        ordered.push(id);
    }

    ordered
}

fn validate_terminal_order<'a>(
    ordered_ids: &[String],
    active_ids: impl IntoIterator<Item = &'a str>,
) -> Result<(), String> {
    let active_ids = active_ids.into_iter().collect::<HashSet<_>>();
    if ordered_ids.len() != active_ids.len() {
        return Err("terminal order must include every active terminal".to_string());
    }

    let mut seen = HashSet::with_capacity(ordered_ids.len());
    for id in ordered_ids {
        if !active_ids.contains(id.as_str()) {
            return Err(format!("unknown terminal id: {id}"));
        }
        if !seen.insert(id.as_str()) {
            return Err(format!("duplicate terminal id: {id}"));
        }
    }

    Ok(())
}

/// Summary of a session for listing purposes.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub id: String,
    pub name: Option<String>,
    pub workdir: Option<PathBuf>,
    pub terminal_count: usize,
    pub created_at_elapsed_secs: u64,
    pub last_activity_elapsed_secs: u64,
    /// Whether a client is currently attached to this session.
    pub is_occupied: bool,
}

/// Summary of a live terminal for operator-facing status output.
#[derive(Debug, Clone)]
pub struct TerminalInfo {
    pub id: String,
    pub title: Option<String>,
    pub created_at_unix_secs: u64,
    pub created_at_elapsed_secs: u64,
    pub uptime_secs: u64,
}

// ---------------------------------------------------------------------------
// Persistence
// ---------------------------------------------------------------------------

/// On-disk representation of registry state.
///
/// Stored at `~/.config/zedra/workspaces/<hash>/sessions.json`.
/// Serialized with serde_json. `[u8; 32]` keys are stored as arrays of
/// integers (compact, no extra dependencies).
#[derive(Serialize, Deserialize)]
struct PersistedState {
    version: u32,
    /// Global authorized client pubkeys (SSH authorized_keys equivalent).
    authorized_clients: Vec<[u8; 32]>,
    sessions: Vec<PersistedSession>,
}

#[derive(Serialize, Deserialize)]
struct PersistedSession {
    id: String,
    name: Option<String>,
    workdir: Option<PathBuf>,
    /// Per-session authorized pubkeys.
    acl: Vec<[u8; 32]>,
}

// ---------------------------------------------------------------------------
// SessionRegistry
// ---------------------------------------------------------------------------

pub struct SessionRegistry {
    sessions: Mutex<HashMap<String, Arc<ServerSession>>>,
    /// Index: session name → session ID.
    name_index: Mutex<HashMap<String, String>>,
    /// All client pubkeys that have ever paired with this host.
    /// Acts like SSH authorized_keys — globally trusted across all sessions.
    authorized_clients: Mutex<HashSet<[u8; 32]>>,
    /// Active pairing slots: session_id → slot.
    pairing_slots: Mutex<HashMap<String, PairingSlot>>,
    /// Consumed slot session IDs (kept briefly to return HandshakeConsumed).
    consumed_slots: Mutex<HashSet<String>>,
    /// Path to persist registry state across restarts. `None` = in-memory only.
    storage_path: Option<PathBuf>,
}

impl std::fmt::Debug for SessionRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionRegistry").finish_non_exhaustive()
    }
}

impl SessionRegistry {
    /// Create an empty in-memory registry (no persistence).
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            name_index: Mutex::new(HashMap::new()),
            authorized_clients: Mutex::new(HashSet::new()),
            pairing_slots: Mutex::new(HashMap::new()),
            consumed_slots: Mutex::new(HashSet::new()),
            storage_path: None,
        }
    }

    /// Load persisted state from `path` (if it exists) and return a registry
    /// pre-populated with the saved sessions and authorized clients.
    ///
    /// The file path is also stored so future mutations are automatically
    /// saved back via `save()`.
    pub async fn load_or_new(path: PathBuf) -> Self {
        let mut registry = Self::new();
        registry.storage_path = Some(path.clone());

        let data = match std::fs::read_to_string(&path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::info!("No persisted session state found, starting fresh");
                return registry;
            }
            Err(e) => {
                tracing::warn!("Failed to read sessions file {}: {}", path.display(), e);
                return registry;
            }
        };

        let state: PersistedState = match serde_json::from_str(&data) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Failed to parse sessions file: {}", e);
                return registry;
            }
        };

        // Restore global authorized clients
        {
            let mut auth = registry.authorized_clients.lock().await;
            for key in &state.authorized_clients {
                auth.insert(*key);
            }
        }

        // Restore sessions
        let session_count = {
            let mut sessions = registry.sessions.lock().await;
            let mut name_index = registry.name_index.lock().await;

            for ps in state.sessions {
                let session = Arc::new(ServerSession::new(
                    ps.id.clone(),
                    ps.name.clone(),
                    ps.workdir,
                ));

                // Restore ACL
                {
                    let mut acl = session.acl.lock().await;
                    for key in ps.acl {
                        acl.insert(key);
                    }
                }

                if let Some(ref name) = ps.name {
                    name_index.insert(name.clone(), ps.id.clone());
                }
                sessions.insert(ps.id, session);
            }

            sessions.len()
        }; // drop sessions + name_index guards here

        let client_count = registry.authorized_clients.lock().await.len();
        tracing::info!(
            "Loaded {} session(s), {} authorized client(s) from {}",
            session_count,
            client_count,
            path.display(),
        );

        registry
    }

    /// Persist the current registry state to disk.
    ///
    /// No-op if no storage path was configured. Errors are logged, not
    /// propagated — a save failure should never abort an RPC call.
    async fn save(&self) {
        let Some(ref path) = self.storage_path else {
            return;
        };

        // Snapshot under lock, then release before doing I/O.
        let authorized_clients: Vec<[u8; 32]> = self
            .authorized_clients
            .lock()
            .await
            .iter()
            .cloned()
            .collect();

        let mut persisted_sessions = Vec::new();
        {
            let sessions = self.sessions.lock().await;
            for session in sessions.values() {
                let acl: Vec<[u8; 32]> = session.acl.lock().await.iter().cloned().collect();
                persisted_sessions.push(PersistedSession {
                    id: session.id.clone(),
                    name: session.name.clone(),
                    workdir: session.workdir.clone(),
                    acl,
                });
            }
        }

        let state = PersistedState {
            version: 1,
            authorized_clients,
            sessions: persisted_sessions,
        };

        let json = match serde_json::to_string_pretty(&state) {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!("Failed to serialize session state: {}", e);
                return;
            }
        };

        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Write atomically: write to a temp file with 0o600 permissions, then rename.
        // Prevents a mid-write crash from corrupting the sessions file.
        // 0o600 is set at creation to avoid a TOCTOU window.
        let tmp_path = path.with_extension("json.tmp");
        let write_result = {
            #[cfg(unix)]
            {
                use std::io::Write;
                use std::os::unix::fs::OpenOptionsExt;
                std::fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .mode(0o600)
                    .open(&tmp_path)
                    .and_then(|mut f| f.write_all(json.as_bytes()))
            }
            #[cfg(not(unix))]
            {
                std::fs::write(&tmp_path, json.as_bytes())
            }
        };
        if let Err(e) = write_result {
            tracing::warn!(
                "Failed to write sessions temp file {}: {}",
                tmp_path.display(),
                e
            );
            return;
        }
        if let Err(e) = tokio::fs::rename(&tmp_path, path).await {
            tracing::warn!("Failed to rename sessions file: {}", e);
            let _ = tokio::fs::remove_file(&tmp_path).await;
        }
    }

    // -----------------------------------------------------------------------
    // Session creation / lookup
    // -----------------------------------------------------------------------

    /// Create a named session bound to a working directory.
    ///
    /// Idempotent: if a session with this name already exists, returns it.
    pub async fn create_named(&self, name: &str, workdir: PathBuf) -> Arc<ServerSession> {
        let name_index = self.name_index.lock().await;
        if let Some(existing_id) = name_index.get(name) {
            let sessions = self.sessions.lock().await;
            if let Some(session) = sessions.get(existing_id) {
                session.touch().await;
                return session.clone();
            }
        }
        drop(name_index);

        let id = zedra_rpc::generate_session_id();
        let session = Arc::new(ServerSession::new(
            id.clone(),
            Some(name.to_string()),
            Some(workdir),
        ));

        self.sessions
            .lock()
            .await
            .insert(id.clone(), session.clone());
        self.name_index.lock().await.insert(name.to_string(), id);
        session
    }

    /// Look up a session by its human-readable name.
    pub async fn get_by_name(&self, name: &str) -> Option<Arc<ServerSession>> {
        let name_index = self.name_index.lock().await;
        let id = name_index.get(name)?;
        let sessions = self.sessions.lock().await;
        sessions.get(id).cloned()
    }

    /// Get a session by ID.
    pub async fn get(&self, session_id: &str) -> Option<Arc<ServerSession>> {
        self.sessions.lock().await.get(session_id).cloned()
    }

    /// Return the number of active sessions.
    pub async fn session_count(&self) -> usize {
        self.sessions.lock().await.len()
    }

    /// Return the first session (arbitrary order), if any.
    /// Used by the REST API when no session_id is specified.
    pub async fn first_session(&self) -> Option<Arc<ServerSession>> {
        self.sessions.lock().await.values().next().cloned()
    }

    /// List all active sessions with summary info.
    pub async fn list_sessions(&self) -> Vec<SessionInfo> {
        let sessions = self.sessions.lock().await;
        let mut result = Vec::with_capacity(sessions.len());

        for session in sessions.values() {
            let terminal_count = session.terminals.lock().await.len();
            let last_activity = *session.last_activity.lock().await;
            let is_occupied = session.active_client.lock().await.is_some();
            result.push(SessionInfo {
                id: session.id.clone(),
                name: session.name.clone(),
                workdir: session.workdir.clone(),
                terminal_count,
                created_at_elapsed_secs: session.created_at.elapsed().as_secs(),
                last_activity_elapsed_secs: last_activity.elapsed().as_secs(),
                is_occupied,
            });
        }

        result.sort_by(|a, b| match (&a.name, &b.name) {
            (Some(na), Some(nb)) => na.cmp(nb),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.id.cmp(&b.id),
        });

        result
    }

    /// Remove a named session by name. Returns true if removed.
    pub async fn remove_by_name(&self, name: &str) -> bool {
        let mut name_index = self.name_index.lock().await;
        if let Some(id) = name_index.remove(name) {
            self.sessions.lock().await.remove(&id);
            true
        } else {
            false
        }
    }

    /// Clean up sessions that have been idle longer than `grace_period`.
    /// Returns the IDs of removed sessions.
    pub async fn cleanup(&self, grace_period: Duration) -> Vec<String> {
        let mut sessions = self.sessions.lock().await;
        let mut to_remove = Vec::new();

        for (id, session) in sessions.iter() {
            let last = *session.last_activity.lock().await;
            if last.elapsed() > grace_period {
                to_remove.push(id.clone());
            }
        }

        let mut removed = Vec::new();
        for id in &to_remove {
            sessions.remove(id);
            removed.push(id.clone());
        }
        drop(sessions);

        if !to_remove.is_empty() {
            let mut name_index = self.name_index.lock().await;
            name_index.retain(|_, v| !to_remove.contains(v));

            // Also expire old consumed slot entries
            let mut consumed = self.consumed_slots.lock().await;
            consumed.retain(|id| !to_remove.contains(id));
        }

        removed
    }

    // -----------------------------------------------------------------------
    // PKI authorization
    // -----------------------------------------------------------------------

    /// Check if a client pubkey is in the global authorized list.
    pub async fn is_globally_authorized(&self, client_pubkey: &[u8; 32]) -> bool {
        self.authorized_clients.lock().await.contains(client_pubkey)
    }

    /// Add a client pubkey to the global authorized list and the session ACL.
    ///
    /// Called after successful HMAC verification during registration.
    pub async fn add_client_to_session(&self, session_id: &str, client_pubkey: [u8; 32]) -> bool {
        // Add to global authorized list
        self.authorized_clients.lock().await.insert(client_pubkey);

        // Add to session ACL
        let added = {
            let sessions = self.sessions.lock().await;
            if let Some(session) = sessions.get(session_id) {
                session.acl.lock().await.insert(client_pubkey);
                tracing::info!(
                    "Registered client {:?}... in session {}",
                    &client_pubkey[..4],
                    session_id,
                );
                true
            } else {
                tracing::warn!("add_client_to_session: session {} not found", session_id);
                false
            }
        };

        if added {
            self.save().await;
        }
        added
    }

    /// Try to attach a client to a session (set as active_client).
    ///
    /// Returns `true` if the client was attached successfully.
    /// Returns `false` if:
    /// - Session not found
    /// - Client not in session ACL
    /// - Another client is already active
    pub async fn attach_client(&self, session_id: &str, client_pubkey: [u8; 32]) -> AttachResult {
        let sessions = self.sessions.lock().await;
        let session = match sessions.get(session_id) {
            Some(s) => s,
            None => return AttachResult::SessionNotFound,
        };

        // Check session ACL
        if !session.acl.lock().await.contains(&client_pubkey) {
            return AttachResult::NotInSessionAcl;
        }

        // Check if occupied
        let mut active = session.active_client.lock().await;
        if active.is_some() && *active != Some(client_pubkey) {
            return AttachResult::SessionOccupied;
        }

        *active = Some(client_pubkey);
        session.touch().await;
        AttachResult::Ok
    }

    /// Clear the active client for a session (on disconnect or detach).
    pub async fn detach_client(&self, session_id: &str, client_pubkey: [u8; 32]) {
        let sessions = self.sessions.lock().await;
        if let Some(session) = sessions.get(session_id) {
            let mut active = session.active_client.lock().await;
            if *active == Some(client_pubkey) {
                *active = None;
                tracing::info!("Detached client from session {}", session_id);
            }
        }
    }

    /// Find the first session that has `client_pubkey` in its ACL.
    ///
    /// Used during reconnect when the client's stored session_id is stale
    /// (e.g. after a daemon restart). Returns `None` if the client has never
    /// paired with any session on this host.
    pub async fn find_session_for_client(
        &self,
        client_pubkey: &[u8; 32],
    ) -> Option<Arc<ServerSession>> {
        let sessions = self.sessions.lock().await;
        for session in sessions.values() {
            if session.acl.lock().await.contains(client_pubkey) {
                return Some(session.clone());
            }
        }
        None
    }

    /// Force-detach any active client from a session.
    /// Used by `zedra detach --session-id <id>` CLI command.
    pub async fn force_detach(&self, session_id: &str) -> bool {
        let sessions = self.sessions.lock().await;
        if let Some(session) = sessions.get(session_id) {
            *session.active_client.lock().await = None;
            true
        } else {
            false
        }
    }

    // -----------------------------------------------------------------------
    // Pairing slots
    // -----------------------------------------------------------------------

    /// Store a new one-use pairing slot for a session.
    /// Replaces any existing slot for the same session_id.
    pub async fn add_pairing_slot(&self, session_id: &str, handshake_secret: [u8; 16]) {
        let slot = PairingSlot {
            handshake_secret,
            session_id: session_id.to_string(),
            expires_at: Instant::now() + Duration::from_secs(600), // 10 min
        };
        self.pairing_slots
            .lock()
            .await
            .insert(session_id.to_string(), slot);
        // Remove from consumed set if it was there (new QR supersedes old)
        self.consumed_slots.lock().await.remove(session_id);
        tracing::info!("Pairing slot created for session {}", session_id);
    }

    /// Atomically consume a pairing slot.
    ///
    /// If the slot exists and is still valid, removes it and returns `Active`.
    /// If it was already used, returns `Consumed`.
    /// If it never existed, returns `NotFound`.
    pub async fn consume_pairing_slot(&self, session_id: &str) -> ConsumeSlotResult {
        let mut slots = self.pairing_slots.lock().await;

        if let Some(slot) = slots.remove(session_id) {
            // Move to consumed set
            self.consumed_slots
                .lock()
                .await
                .insert(session_id.to_string());

            // Check expiry
            if slot.expires_at < Instant::now() {
                tracing::warn!("Pairing slot for {} expired", session_id);
                return ConsumeSlotResult::NotFound;
            }

            ConsumeSlotResult::Active(slot)
        } else if self.consumed_slots.lock().await.contains(session_id) {
            ConsumeSlotResult::Consumed
        } else {
            ConsumeSlotResult::NotFound
        }
    }
}

// ---------------------------------------------------------------------------
// AttachResult
// ---------------------------------------------------------------------------

/// Outcome of an `attach_client` attempt.
pub enum AttachResult {
    Ok,
    SessionNotFound,
    NotInSessionAcl,
    SessionOccupied,
}

// ---------------------------------------------------------------------------
// ServerSession impl
// ---------------------------------------------------------------------------

impl ServerSession {
    fn new(id: String, name: Option<String>, workdir: Option<PathBuf>) -> Self {
        Self {
            id,
            name,
            workdir,
            created_at: Instant::now(),
            last_activity: Mutex::new(Instant::now()),
            terminals: Mutex::new(HashMap::new()),
            terminal_order: Mutex::new(Vec::new()),
            acl: Mutex::new(HashSet::new()),
            active_client: Mutex::new(None),
            session_token: Mutex::new(None),
            event_tx: Mutex::new(None),
            fs_watched_paths: Mutex::new(HashSet::new()),
            fs_watch_rpc_limiter: Mutex::new(TokenBucket::new(
                FS_WATCH_RPC_RATE_PER_SEC,
                FS_WATCH_RPC_BURST,
            )),
            docs_tree_cache: Mutex::new(None),
            docs_tree_scan_in_flight: AtomicBool::new(false),
            rpc_fs_reads: AtomicU64::new(0),
            rpc_fs_writes: AtomicU64::new(0),
            rpc_git_ops: AtomicU64::new(0),
            rpc_git_commits: AtomicU64::new(0),
            rpc_ai_prompts: AtomicU64::new(0),
            observer_gen: AtomicU64::new(0),
            observer_events_sent: AtomicU64::new(0),
            observer_events_dropped_no_subscriber: AtomicU64::new(0),
            observer_events_dropped_full: AtomicU64::new(0),
            fs_watch_quota_rejected: AtomicU64::new(0),
            fs_watch_rate_limited: AtomicU64::new(0),
        }
    }

    /// Issue a new session token for the given client, replacing any existing one.
    /// Only one token exists per session at a time.
    pub async fn issue_session_token(&self, client_pubkey: [u8; 32]) -> [u8; 32] {
        let mut token = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut token);
        *self.session_token.lock().await = Some((client_pubkey, SessionToken { token }));
        token
    }

    /// Atomically consume the session token on validation.
    /// Returns `true` only if the token belongs to `client_pubkey` and matches.
    /// The slot is cleared on consumption to prevent replay.
    pub async fn validate_session_token(
        &self,
        client_pubkey: &[u8; 32],
        session_token: &[u8; 32],
    ) -> bool {
        let mut slot = self.session_token.lock().await;
        let Some((stored_pubkey, entry)) = slot.take() else {
            return false;
        };
        if &stored_pubkey != client_pubkey {
            // Token belongs to a different client — put it back and reject.
            *slot = Some((stored_pubkey, entry));
            return false;
        }
        &entry.token == session_token
    }

    pub async fn terminal_sync_entries(&self) -> Vec<TerminalSyncEntry> {
        let terms = self.terminals.lock().await;
        let mut order = self.terminal_order.lock().await;
        let ordered_ids = ordered_terminal_ids_locked(&terms, &mut order);
        let mut entries = Vec::with_capacity(ordered_ids.len());
        for (position, id) in ordered_ids.into_iter().enumerate() {
            let Some(term) = terms.get(&id) else {
                continue;
            };
            let (title, cwd, icon_name) = term
                .host_meta
                .lock()
                .ok()
                .map(|meta| (meta.title.clone(), meta.cwd.clone(), meta.icon_name.clone()))
                .unwrap_or((None, None, None));
            let last_seq = term
                .backlog
                .lock()
                .map(|b| b.next_seq.saturating_sub(1))
                .unwrap_or(0);
            entries.push(TerminalSyncEntry {
                id,
                position: position as u64,
                last_seq,
                title,
                cwd,
                icon_name,
            });
        }
        entries
    }

    pub async fn terminal_ids(&self) -> Vec<String> {
        let terms = self.terminals.lock().await;
        let mut order = self.terminal_order.lock().await;
        ordered_terminal_ids_locked(&terms, &mut order)
    }

    pub async fn terminal_infos(&self) -> Vec<TerminalInfo> {
        let terms = self.terminals.lock().await;
        let mut order = self.terminal_order.lock().await;
        let ordered_ids = ordered_terminal_ids_locked(&terms, &mut order);
        let mut entries = Vec::with_capacity(ordered_ids.len());
        for id in ordered_ids {
            let Some(term) = terms.get(&id) else {
                continue;
            };
            let title = term
                .host_meta
                .lock()
                .ok()
                .and_then(|meta| meta.title.clone());
            let created_at_unix_secs = term
                .created_at
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let uptime_secs = term.started_at.elapsed().as_secs();
            entries.push(TerminalInfo {
                id,
                title,
                created_at_unix_secs,
                created_at_elapsed_secs: uptime_secs,
                uptime_secs,
            });
        }
        entries
    }

    pub async fn insert_terminal(&self, id: String, terminal: TermSession) {
        let mut terms = self.terminals.lock().await;
        terms.insert(id.clone(), terminal);

        let mut order = self.terminal_order.lock().await;
        if !order.iter().any(|existing_id| existing_id == &id) {
            order.push(id);
        }
    }

    pub async fn remove_terminal(&self, id: &str) -> Option<TermSession> {
        let mut terms = self.terminals.lock().await;
        let terminal = terms.remove(id);

        if terminal.is_some() {
            let mut order = self.terminal_order.lock().await;
            order.retain(|terminal_id| terminal_id != id);
        }

        terminal
    }

    pub async fn reorder_terminals(&self, ordered_ids: Vec<String>) -> Result<(), String> {
        let terms = self.terminals.lock().await;
        validate_terminal_order(&ordered_ids, terms.keys().map(String::as_str))?;

        let mut order = self.terminal_order.lock().await;
        *order = ordered_ids;
        Ok(())
    }

    /// Push a host-initiated event to the subscribed client, if any.
    /// Non-blocking: drops when channel is absent/full and increments counters.
    pub async fn push_event(&self, event: HostEvent) -> bool {
        let tx = self.event_tx.lock().await.clone();
        let Some(tx) = tx else {
            self.observer_events_dropped_no_subscriber
                .fetch_add(1, Ordering::Relaxed);
            return false;
        };
        match tx.try_send(event) {
            Ok(()) => {
                self.observer_events_sent.fetch_add(1, Ordering::Relaxed);
                true
            }
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                self.observer_events_dropped_full
                    .fetch_add(1, Ordering::Relaxed);
                false
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                self.observer_events_dropped_no_subscriber
                    .fetch_add(1, Ordering::Relaxed);
                false
            }
        }
    }

    /// Token-bucket guard for FsWatch/FsUnwatch RPC calls.
    pub async fn allow_fs_watch_rpc(&self) -> bool {
        self.fs_watch_rpc_limiter.lock().await.allow()
    }

    /// Insert a watched path if quota allows.
    pub async fn try_add_watched_path(&self, path: String) -> bool {
        let mut watched = self.fs_watched_paths.lock().await;
        if watched.contains(&path) {
            return true;
        }
        if watched.len() >= MAX_WATCHED_PATHS_PER_SESSION {
            self.fs_watch_quota_rejected.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        watched.insert(path);
        true
    }

    /// Remove a watched path; returns true if it existed.
    pub async fn remove_watched_path(&self, path: &str) -> bool {
        self.fs_watched_paths.lock().await.remove(path)
    }

    pub fn try_begin_docs_tree_scan(&self) -> bool {
        self.docs_tree_scan_in_flight
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    pub fn finish_docs_tree_scan(&self) {
        self.docs_tree_scan_in_flight
            .store(false, Ordering::Release);
    }

    pub async fn store_docs_tree_snapshot(&self, root_key: String, snapshot: DocsTreeSnapshot) {
        // The cache is a single manual rebuild snapshot, not a persistent index.
        *self.docs_tree_cache.lock().await = Some(DocsTreeCacheEntry { root_key, snapshot });
    }

    pub async fn docs_tree_page(
        &self,
        root_key: &str,
        snapshot_id: Option<&str>,
        offset: u32,
        limit: u32,
    ) -> std::result::Result<FsDocsTreeResult, FsDocsTreeError> {
        let mut cache = self.docs_tree_cache.lock().await;
        let Some(entry) = cache.as_ref() else {
            return Err(FsDocsTreeError::CacheMiss);
        };

        if entry.snapshot.created_at.elapsed() > DOCS_TREE_CACHE_TTL {
            *cache = None;
            return Err(FsDocsTreeError::CacheMiss);
        }
        if entry.root_key != root_key {
            return Err(FsDocsTreeError::CacheMiss);
        }
        if snapshot_id != Some(entry.snapshot.id.as_str()) {
            return Err(FsDocsTreeError::StaleSnapshot);
        }

        // Page from the cached snapshot without cloning the full markdown list.
        Ok(snapshot_page_result(&entry.snapshot, offset, limit))
    }

    /// Whether a client is currently attached.
    pub async fn is_occupied(&self) -> bool {
        self.active_client.lock().await.is_some()
    }

    /// Allocate the next terminal ID for this session.
    pub async fn next_terminal_id(&self) -> String {
        Uuid::new_v4().to_string()
    }

    /// Get backlog entries for a terminal after a given sequence number.
    /// Reads from the terminal's own per-terminal backlog.
    pub async fn backlog_after(&self, terminal_id: &str, after_seq: u64) -> Vec<BacklogEntry> {
        let terms = self.terminals.lock().await;
        match terms.get(terminal_id) {
            Some(term) => term.backlog.lock().unwrap().after(after_seq),
            None => vec![],
        }
    }

    /// Get replay entries plus retained backlog window details for diagnostics.
    pub async fn backlog_replay_after(
        &self,
        terminal_id: &str,
        after_seq: u64,
    ) -> Option<TermBacklogReplay> {
        let terms = self.terminals.lock().await;
        terms
            .get(terminal_id)
            .map(|term| term.backlog.lock().unwrap().replay_after(after_seq))
    }

    /// Clear all terminal output senders (e.g. when connection drops).
    pub async fn clear_output_senders(&self) {
        let terms = self.terminals.lock().await;
        for term in terms.values() {
            term.output_sender.lock().unwrap().sender = None;
        }
    }

    /// Touch the session to update last_activity.
    pub async fn touch(&self) {
        *self.last_activity.lock().await = Instant::now();
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_pubkey(seed: u8) -> [u8; 32] {
        [seed; 32]
    }

    async fn create_session(registry: &SessionRegistry) -> Arc<ServerSession> {
        registry.create_named("test", PathBuf::from("/tmp")).await
    }

    fn docs_snapshot(id: &str) -> DocsTreeSnapshot {
        DocsTreeSnapshot {
            id: id.to_string(),
            root_name: "tmp".to_string(),
            root_path: "/tmp".to_string(),
            docs: Vec::new(),
            truncated: false,
            created_at: Instant::now(),
        }
    }

    #[test]
    fn terminal_order_keeps_existing_order_and_appends_missing_by_creation_time() {
        let mut order = vec![
            "stale".to_string(),
            "term-b".to_string(),
            "term-a".to_string(),
        ];

        let ordered = ordered_terminal_ids_from_entries(
            [
                ("term-a".to_string(), Duration::from_secs(30)),
                ("term-c".to_string(), Duration::from_secs(10)),
                ("term-b".to_string(), Duration::from_secs(20)),
                ("term-d".to_string(), Duration::from_secs(10)),
            ],
            &mut order,
        );

        assert_eq!(ordered, vec!["term-b", "term-a", "term-c", "term-d"]);
        assert_eq!(order, ordered);
    }

    #[tokio::test]
    async fn fs_docs_tree_cache_returns_matching_snapshot_page() {
        let registry = SessionRegistry::new();
        let session = create_session(&registry).await;
        session
            .store_docs_tree_snapshot("/tmp".to_string(), docs_snapshot("snapshot-a"))
            .await;

        let result = session
            .docs_tree_page("/tmp", Some("snapshot-a"), 0, 50)
            .await
            .unwrap();

        assert_eq!(result.snapshot_id.as_deref(), Some("snapshot-a"));
        assert!(result.root.is_some());
    }

    #[tokio::test]
    async fn fs_docs_tree_cache_rejects_stale_or_missing_snapshot() {
        let registry = SessionRegistry::new();
        let session = create_session(&registry).await;
        session
            .store_docs_tree_snapshot("/tmp".to_string(), docs_snapshot("snapshot-a"))
            .await;

        assert_eq!(
            session
                .docs_tree_page("/tmp", Some("snapshot-b"), 0, 50)
                .await
                .unwrap_err(),
            FsDocsTreeError::StaleSnapshot
        );
        assert_eq!(
            session
                .docs_tree_page("/other", Some("snapshot-a"), 0, 50)
                .await
                .unwrap_err(),
            FsDocsTreeError::CacheMiss
        );
    }

    #[test]
    fn fs_docs_tree_scan_guard_allows_one_rebuild_at_a_time() {
        let session = ServerSession::new("test".to_string(), None, None);

        assert!(session.try_begin_docs_tree_scan());
        assert!(!session.try_begin_docs_tree_scan());
        session.finish_docs_tree_scan();
        assert!(session.try_begin_docs_tree_scan());
    }

    #[test]
    fn terminal_order_tiebreaks_missing_terminals_by_id() {
        let mut order = Vec::new();

        let ordered = ordered_terminal_ids_from_entries(
            [
                ("term-c".to_string(), Duration::from_secs(10)),
                ("term-a".to_string(), Duration::from_secs(10)),
                ("term-b".to_string(), Duration::from_secs(10)),
            ],
            &mut order,
        );

        assert_eq!(ordered, vec!["term-a", "term-b", "term-c"]);
        assert_eq!(order, ordered);
    }

    #[test]
    fn terminal_order_validation_accepts_exact_permutation() {
        let ordered_ids = vec![
            "term-c".to_string(),
            "term-a".to_string(),
            "term-b".to_string(),
        ];

        assert!(validate_terminal_order(&ordered_ids, ["term-a", "term-b", "term-c"]).is_ok());
    }

    #[test]
    fn terminal_order_validation_rejects_partial_duplicate_and_unknown_ids() {
        assert!(validate_terminal_order(&["term-a".to_string()], ["term-a", "term-b"]).is_err());
        assert!(validate_terminal_order(
            &["term-a".to_string(), "term-a".to_string()],
            ["term-a", "term-b"]
        )
        .is_err());
        assert!(validate_terminal_order(
            &["term-a".to_string(), "missing".to_string()],
            ["term-a", "term-b"]
        )
        .is_err());
    }

    #[tokio::test]
    async fn create_new_session() {
        let registry = SessionRegistry::new();
        let session = registry
            .create_named("myapp", PathBuf::from("/home/user/myapp"))
            .await;
        assert!(!session.id.is_empty());
        assert_eq!(session.name.as_deref(), Some("myapp"));
    }

    #[tokio::test]
    async fn create_named_idempotent() {
        let registry = SessionRegistry::new();
        let s1 = registry
            .create_named("zedra", PathBuf::from("/zedra"))
            .await;
        let s2 = registry
            .create_named("zedra", PathBuf::from("/zedra"))
            .await;
        assert_eq!(s1.id, s2.id);
    }

    #[tokio::test]
    async fn get_by_name() {
        let registry = SessionRegistry::new();
        let s = registry
            .create_named("webapp", PathBuf::from("/webapp"))
            .await;
        let found = registry.get_by_name("webapp").await.unwrap();
        assert_eq!(found.id, s.id);
        assert!(registry.get_by_name("nonexistent").await.is_none());
    }

    #[tokio::test]
    async fn list_sessions_sorted() {
        let registry = SessionRegistry::new();
        registry
            .create_named("webapp", PathBuf::from("/webapp"))
            .await;
        registry.create_named("api", PathBuf::from("/api")).await;

        let list = registry.list_sessions().await;
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name.as_deref(), Some("api"));
        assert_eq!(list[1].name.as_deref(), Some("webapp"));
    }

    #[tokio::test]
    async fn remove_by_name() {
        let registry = SessionRegistry::new();
        registry.create_named("temp", PathBuf::from("/tmp")).await;

        assert!(registry.get_by_name("temp").await.is_some());
        assert!(registry.remove_by_name("temp").await);
        assert!(registry.get_by_name("temp").await.is_none());
        assert!(!registry.remove_by_name("temp").await);
    }

    #[tokio::test]
    async fn pairing_slot_roundtrip() {
        let registry = SessionRegistry::new();
        let session = registry.create_named("s1", PathBuf::from("/s1")).await;
        let key = [42u8; 16];

        registry.add_pairing_slot(&session.id, key).await;

        match registry.consume_pairing_slot(&session.id).await {
            ConsumeSlotResult::Active(slot) => {
                assert_eq!(slot.handshake_secret, key);
                assert_eq!(slot.session_id, session.id);
            }
            _ => panic!("expected Active"),
        }
    }

    #[tokio::test]
    async fn pairing_slot_consumed_returns_consumed() {
        let registry = SessionRegistry::new();
        let session = registry.create_named("s1", PathBuf::from("/s1")).await;

        registry.add_pairing_slot(&session.id, [1u8; 16]).await;
        // First consume
        let _ = registry.consume_pairing_slot(&session.id).await;
        // Second consume should get Consumed
        match registry.consume_pairing_slot(&session.id).await {
            ConsumeSlotResult::Consumed => {}
            _ => panic!("expected Consumed"),
        }
    }

    #[tokio::test]
    async fn pairing_slot_not_found() {
        let registry = SessionRegistry::new();
        match registry.consume_pairing_slot("no-such-session").await {
            ConsumeSlotResult::NotFound => {}
            _ => panic!("expected NotFound"),
        }
    }

    #[tokio::test]
    async fn add_client_and_attach() {
        let registry = SessionRegistry::new();
        let session = registry.create_named("s1", PathBuf::from("/s1")).await;
        let pubkey = make_pubkey(1);

        // Not yet authorized
        assert!(!registry.is_globally_authorized(&pubkey).await);

        // Register client
        registry.add_client_to_session(&session.id, pubkey).await;
        assert!(registry.is_globally_authorized(&pubkey).await);

        // Attach
        match registry.attach_client(&session.id, pubkey).await {
            AttachResult::Ok => {}
            _ => panic!("expected Ok"),
        }

        assert!(session.is_occupied().await);
    }

    #[tokio::test]
    async fn attach_session_occupied() {
        let registry = SessionRegistry::new();
        let session = registry.create_named("s1", PathBuf::from("/s1")).await;
        let key_a = make_pubkey(1);
        let key_b = make_pubkey(2);

        registry.add_client_to_session(&session.id, key_a).await;
        registry.add_client_to_session(&session.id, key_b).await;

        // A attaches first
        assert!(matches!(
            registry.attach_client(&session.id, key_a).await,
            AttachResult::Ok
        ));

        // B is blocked
        assert!(matches!(
            registry.attach_client(&session.id, key_b).await,
            AttachResult::SessionOccupied
        ));
    }

    #[tokio::test]
    async fn detach_client() {
        let registry = SessionRegistry::new();
        let session = registry.create_named("s1", PathBuf::from("/s1")).await;
        let pubkey = make_pubkey(1);

        registry.add_client_to_session(&session.id, pubkey).await;
        let _ = registry.attach_client(&session.id, pubkey).await;
        assert!(session.is_occupied().await);

        registry.detach_client(&session.id, pubkey).await;
        assert!(!session.is_occupied().await);
    }

    #[tokio::test]
    async fn attach_not_in_acl() {
        let registry = SessionRegistry::new();
        let session = registry.create_named("s1", PathBuf::from("/s1")).await;
        let pubkey = make_pubkey(99);

        match registry.attach_client(&session.id, pubkey).await {
            AttachResult::NotInSessionAcl => {}
            _ => panic!("expected NotInSessionAcl"),
        }
    }

    #[test]
    fn term_backlog_push_and_after() {
        let mut b = TermBacklog::new();

        for i in 1..=3 {
            b.push("term-1".to_string(), format!("msg{}", i).into_bytes());
        }

        let after_0 = b.after(0);
        assert_eq!(after_0.len(), 3);

        let after_1 = b.after(1);
        assert_eq!(after_1.len(), 2);
        assert_eq!(after_1[0].data, b"msg2");

        // Different terminal_id shares no entries.
        let other = b.after(0);
        assert!(other.iter().all(|e| e.terminal_id == "term-1"));
    }

    #[test]
    fn term_backlog_cap() {
        let mut b = TermBacklog::new();

        for i in 0..1050 {
            b.push("term-1".to_string(), format!("msg{}", i).into_bytes());
        }

        assert_eq!(b.entries.len(), 1000);
        // seq starts at 1, so after 1050 pushes the oldest retained is seq 51
        assert_eq!(b.entries.front().unwrap().seq, 51);
    }

    #[test]
    fn term_backlog_replay_reports_retained_window() {
        let mut b = TermBacklog::new();

        for data in ["one", "two-two", "three"] {
            b.push("term-1".to_string(), data.as_bytes().to_vec());
        }

        let replay = b.replay_after(1);

        assert_eq!(replay.entries.len(), 2);
        assert_eq!(replay.oldest_seq, Some(1));
        assert_eq!(replay.newest_seq, 3);
        assert_eq!(replay.retained_entries, 3);
        assert_eq!(
            replay.retained_bytes,
            "one".len() + "two-two".len() + "three".len()
        );
    }

    #[tokio::test]
    async fn cleanup_removes_idle_sessions() {
        let registry = SessionRegistry::new();
        let s = registry.create_named("old", PathBuf::from("/old")).await;
        let id = s.id.clone();

        *s.last_activity.lock().await = Instant::now() - Duration::from_secs(600);

        let removed = registry.cleanup(Duration::from_secs(300)).await;
        assert_eq!(removed, vec![id.clone()]);
        assert!(registry.get(&id).await.is_none());
        assert!(registry.get_by_name("old").await.is_none());
    }

    #[tokio::test]
    async fn cleanup_keeps_active_sessions() {
        let registry = SessionRegistry::new();
        let s = registry
            .create_named("active", PathBuf::from("/active"))
            .await;
        let id = s.id.clone();

        let removed = registry.cleanup(Duration::from_secs(300)).await;
        assert!(removed.is_empty());
        assert!(registry.get(&id).await.is_some());
    }

    #[tokio::test]
    async fn terminal_id_generation() {
        let registry = SessionRegistry::new();
        let session = create_session(&registry).await;

        let mut ids = std::collections::HashSet::new();
        for _ in 0..128 {
            let id = session.next_terminal_id().await;
            assert!(Uuid::parse_str(&id).is_ok());
            assert!(ids.insert(id));
        }
    }

    #[tokio::test]
    async fn list_sessions_occupied_flag() {
        let registry = SessionRegistry::new();
        let s = registry.create_named("s", PathBuf::from("/s")).await;
        let pubkey = make_pubkey(7);
        registry.add_client_to_session(&s.id, pubkey).await;
        let _ = registry.attach_client(&s.id, pubkey).await;

        let list = registry.list_sessions().await;
        assert_eq!(list.len(), 1);
        assert!(list[0].is_occupied);
    }

    #[tokio::test]
    async fn session_token_rotates_and_is_consumed() {
        let registry = SessionRegistry::new();
        let session = create_session(&registry).await;
        let pubkey = make_pubkey(11);

        registry.add_client_to_session(&session.id, pubkey).await;
        assert!(matches!(
            registry.attach_client(&session.id, pubkey).await,
            AttachResult::Ok
        ));

        let first = session.issue_session_token(pubkey).await;
        // Validate consumes the token (single-slot: also clears the stored value)
        assert!(session.validate_session_token(&pubkey, &first).await);
        // Second validation of same token fails (consumed/cleared)
        assert!(!session.validate_session_token(&pubkey, &first).await);

        // Issue a new token and verify it's different
        let second = session.issue_session_token(pubkey).await;
        assert_ne!(first, second);
        assert!(session.validate_session_token(&pubkey, &second).await);
    }

    #[tokio::test]
    async fn session_token_is_scoped_to_active_client() {
        let registry = SessionRegistry::new();
        let session = create_session(&registry).await;
        let key_a = make_pubkey(21);
        let key_b = make_pubkey(22);

        registry.add_client_to_session(&session.id, key_a).await;
        registry.add_client_to_session(&session.id, key_b).await;

        // Issue a token for key_a; key_b should not be able to use it.
        let token = session.issue_session_token(key_a).await;
        // key_b's check leaves the token in place (wrong pubkey → puts it back)
        assert!(!session.validate_session_token(&key_b, &token).await);
        // key_a can still use it
        assert!(session.validate_session_token(&key_a, &token).await);
    }
}
