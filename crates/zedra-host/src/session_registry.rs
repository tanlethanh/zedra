// Session registry: manages persistent server sessions that survive transport
// reconnections. Each session owns its terminal PTY sessions and notification
// backlog, allowing a mobile client to disconnect and reconnect without losing
// state.
//
// v3 (Phase 1 PKI): Auth tokens removed. Sessions use per-session ACLs of
// authorized client public keys. One active client per session at a time.
// Pairing slots (one-use handshake keys) are used for first registration.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use serde::{Deserialize, Serialize};
use zedra_rpc::proto::{BacklogEntry, TermOutput};

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
    pub next_term_id: Mutex<u64>,
    /// Per-terminal output backlog for replay on TermAttach reconnect.
    pub output_backlog: Mutex<VecDeque<BacklogEntry>>,
    pub next_output_seq: Mutex<u64>,
    /// Client pubkeys authorized to attach to this session (per-session ACL).
    pub acl: Mutex<HashSet<[u8; 32]>>,
    /// Currently attached client pubkey. None = session is free.
    pub active_client: Mutex<Option<[u8; 32]>>,
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

/// A live terminal session owned by a ServerSession.
pub struct TermSession {
    pub writer: Box<dyn Write + Send>,
    pub master: Box<dyn portable_pty::MasterPty + Send>,
    /// Swappable output sender. Updated on each TermAttach.
    pub output_sender: Arc<std::sync::Mutex<OutputSenderSlot>>,
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
        let Some(ref path) = self.storage_path else { return };

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
                let acl: Vec<[u8; 32]> =
                    session.acl.lock().await.iter().cloned().collect();
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
        if let Err(e) = tokio::fs::write(path, json).await {
            tracing::warn!("Failed to write sessions file {}: {}", path.display(), e);
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
        let session = Arc::new(ServerSession::new(id.clone(), Some(name.to_string()), Some(workdir)));

        self.sessions.lock().await.insert(id.clone(), session.clone());
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
        self.authorized_clients
            .lock()
            .await
            .contains(client_pubkey)
    }

    /// Add a client pubkey to the global authorized list and the session ACL.
    ///
    /// Called after successful HMAC verification during registration.
    pub async fn add_client_to_session(
        &self,
        session_id: &str,
        client_pubkey: [u8; 32],
    ) -> bool {
        // Add to global authorized list
        self.authorized_clients
            .lock()
            .await
            .insert(client_pubkey);

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
                tracing::warn!(
                    "add_client_to_session: session {} not found",
                    session_id
                );
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
    pub async fn attach_client(
        &self,
        session_id: &str,
        client_pubkey: [u8; 32],
    ) -> AttachResult {
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
            next_term_id: Mutex::new(1),
            output_backlog: Mutex::new(VecDeque::new()),
            next_output_seq: Mutex::new(1),
            acl: Mutex::new(HashSet::new()),
            active_client: Mutex::new(None),
        }
    }

    /// Whether a client is currently attached.
    pub async fn is_occupied(&self) -> bool {
        self.active_client.lock().await.is_some()
    }

    /// Allocate the next terminal ID for this session.
    pub async fn next_terminal_id(&self) -> String {
        let mut id = self.next_term_id.lock().await;
        let current = *id;
        *id += 1;
        format!("term-{}", current)
    }

    /// Add a backlog entry for terminal output replay on reconnect.
    /// Backlog is capped at 1000 entries; oldest are evicted.
    pub async fn push_backlog_entry(&self, entry: BacklogEntry) {
        let mut backlog = self.output_backlog.lock().await;
        backlog.push_back(entry);
        while backlog.len() > 1000 {
            backlog.pop_front();
        }
    }

    /// Allocate the next backlog sequence number.
    pub async fn next_backlog_seq(&self) -> u64 {
        let mut seq = self.next_output_seq.lock().await;
        let current = *seq;
        *seq += 1;
        current
    }

    /// Get backlog entries for a specific terminal after a given sequence number.
    pub async fn backlog_after(&self, terminal_id: &str, after_seq: u64) -> Vec<BacklogEntry> {
        let backlog = self.output_backlog.lock().await;
        backlog
            .iter()
            .filter(|e| e.terminal_id == terminal_id && e.seq > after_seq)
            .cloned()
            .collect()
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
        registry
            .create_named("test", PathBuf::from("/tmp"))
            .await
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
        let s1 = registry.create_named("zedra", PathBuf::from("/zedra")).await;
        let s2 = registry.create_named("zedra", PathBuf::from("/zedra")).await;
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
        registry.create_named("webapp", PathBuf::from("/webapp")).await;
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

    #[tokio::test]
    async fn output_backlog() {
        let registry = SessionRegistry::new();
        let session = create_session(&registry).await;

        for i in 1..=3 {
            let seq = session.next_backlog_seq().await;
            session
                .push_backlog_entry(BacklogEntry {
                    seq,
                    terminal_id: "term-1".to_string(),
                    data: format!("msg{}", i).into_bytes(),
                })
                .await;
        }

        let after_0 = session.backlog_after("term-1", 0).await;
        assert_eq!(after_0.len(), 3);

        let after_1 = session.backlog_after("term-1", 1).await;
        assert_eq!(after_1.len(), 2);
        assert_eq!(after_1[0].data, b"msg2");

        let other = session.backlog_after("term-2", 0).await;
        assert!(other.is_empty());
    }

    #[tokio::test]
    async fn output_backlog_cap() {
        let registry = SessionRegistry::new();
        let session = create_session(&registry).await;

        for i in 0..1050 {
            let seq = session.next_backlog_seq().await;
            session
                .push_backlog_entry(BacklogEntry {
                    seq,
                    terminal_id: "term-1".to_string(),
                    data: format!("msg{}", i).into_bytes(),
                })
                .await;
        }

        let backlog = session.output_backlog.lock().await;
        assert_eq!(backlog.len(), 1000);
        assert_eq!(backlog.front().unwrap().seq, 51);
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
        let s = registry.create_named("active", PathBuf::from("/active")).await;
        let id = s.id.clone();

        let removed = registry.cleanup(Duration::from_secs(300)).await;
        assert!(removed.is_empty());
        assert!(registry.get(&id).await.is_some());
    }

    #[tokio::test]
    async fn terminal_id_generation() {
        let registry = SessionRegistry::new();
        let session = create_session(&registry).await;

        let id1 = session.next_terminal_id().await;
        let id2 = session.next_terminal_id().await;
        assert_eq!(id1, "term-1");
        assert_eq!(id2, "term-2");
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
}
