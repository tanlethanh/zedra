// Session registry: manages persistent server sessions that survive transport
// reconnections. Each session owns its terminal PTY sessions and notification
// backlog, allowing a mobile client to disconnect and reconnect without losing
// state.
//
// v2: Sessions can be named and bound to a working directory, enabling
// multi-session support (one session per project/workdir). Clients can
// list available sessions and connect to a specific one.

use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// A server-side session that persists across transport reconnections.
///
/// Sessions are created when a client first connects (or explicitly via
/// `session/resume_or_create`) and remain alive for a grace period after
/// the transport disconnects. Terminals spawned within a session survive
/// reconnections — the PTY processes keep running and output is buffered
/// in the notification backlog for replay.
pub struct ServerSession {
    pub id: String,
    pub auth_token: String,
    /// Human-readable session name (e.g., "zedra", "webapp"). Optional for
    /// backward compatibility with unnamed sessions.
    pub name: Option<String>,
    /// Working directory for this session. All terminal, fs, and git operations
    /// are scoped to this directory.
    pub workdir: Option<PathBuf>,
    pub created_at: Instant,
    pub last_activity: Mutex<Instant>,
    pub terminals: Mutex<HashMap<String, TermSession>>,
    pub next_term_id: Mutex<u64>,
    pub notification_backlog: Mutex<VecDeque<(u64, Vec<u8>)>>,
    pub next_notif_seq: Mutex<u64>,
    /// Device IDs allowed to access this session. Empty means all trusted
    /// devices can access it.
    pub allowed_devices: Vec<String>,
}

/// A live terminal session owned by a ServerSession.
/// The PTY reader is split off and owned by a background streaming task.
pub struct TermSession {
    pub writer: Box<dyn Write + Send>,
    pub master: Box<dyn portable_pty::MasterPty + Send>,
    /// Swappable notification sender. Updated on each reconnect so the PTY
    /// reader can forward output to the current connection's write channel.
    /// `None` when no connection is active (output is still stored in the
    /// session's notification backlog).
    pub notif_sender: Arc<std::sync::Mutex<Option<tokio::sync::mpsc::Sender<Vec<u8>>>>>,
}

/// Summary of a session for listing purposes (no PTY handles).
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub id: String,
    pub name: Option<String>,
    pub workdir: Option<PathBuf>,
    pub terminal_count: usize,
    pub created_at_elapsed_secs: u64,
    pub last_activity_elapsed_secs: u64,
}

/// Registry of active sessions, keyed by session ID.
/// Also maintains a name → session_id index for named session lookup.
pub struct SessionRegistry {
    sessions: Mutex<HashMap<String, Arc<ServerSession>>>,
    /// Index: session name → session ID. Enables fast lookup by name.
    name_index: Mutex<HashMap<String, String>>,
}

impl SessionRegistry {
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            name_index: Mutex::new(HashMap::new()),
        }
    }

    /// Resume an existing session or create a new one.
    ///
    /// If `session_id` is provided and matches a live session with the correct
    /// `auth_token`, returns that session (updating last_activity). Otherwise
    /// creates a fresh session.
    pub async fn resume_or_create(
        &self,
        session_id: Option<&str>,
        auth_token: &str,
    ) -> Arc<ServerSession> {
        let mut sessions = self.sessions.lock().await;

        // Try to resume an existing session
        if let Some(id) = session_id {
            if let Some(session) = sessions.get(id) {
                if session.auth_token == auth_token {
                    *session.last_activity.lock().await = Instant::now();
                    return session.clone();
                }
            }
        }

        // Create new session
        let id = uuid::Uuid::new_v4().to_string();
        let session = Arc::new(ServerSession {
            id: id.clone(),
            auth_token: auth_token.to_string(),
            name: None,
            workdir: None,
            created_at: Instant::now(),
            last_activity: Mutex::new(Instant::now()),
            terminals: Mutex::new(HashMap::new()),
            next_term_id: Mutex::new(1),
            notification_backlog: Mutex::new(VecDeque::new()),
            next_notif_seq: Mutex::new(1),
            allowed_devices: Vec::new(),
        });
        sessions.insert(id, session.clone());
        session
    }

    /// Create a named session bound to a working directory.
    ///
    /// If a session with this name already exists, returns it (idempotent).
    /// Named sessions are accessible via `get_by_name()` and appear in
    /// `list_sessions()`.
    pub async fn create_named(
        &self,
        name: &str,
        workdir: PathBuf,
        auth_token: &str,
    ) -> Arc<ServerSession> {
        // Check if already exists by name.
        let name_index = self.name_index.lock().await;
        if let Some(existing_id) = name_index.get(name) {
            let sessions = self.sessions.lock().await;
            if let Some(session) = sessions.get(existing_id) {
                session.touch().await;
                return session.clone();
            }
        }
        drop(name_index);

        let id = uuid::Uuid::new_v4().to_string();
        let session = Arc::new(ServerSession {
            id: id.clone(),
            auth_token: auth_token.to_string(),
            name: Some(name.to_string()),
            workdir: Some(workdir),
            created_at: Instant::now(),
            last_activity: Mutex::new(Instant::now()),
            terminals: Mutex::new(HashMap::new()),
            next_term_id: Mutex::new(1),
            notification_backlog: Mutex::new(VecDeque::new()),
            next_notif_seq: Mutex::new(1),
            allowed_devices: Vec::new(),
        });

        self.sessions
            .lock()
            .await
            .insert(id.clone(), session.clone());
        self.name_index.lock().await.insert(name.to_string(), id);
        session
    }

    /// Look up a session by its unique ID.
    pub async fn get_by_id(&self, id: &str) -> Option<Arc<ServerSession>> {
        let sessions = self.sessions.lock().await;
        sessions.get(id).cloned()
    }

    /// Look up a session by its human-readable name.
    pub async fn get_by_name(&self, name: &str) -> Option<Arc<ServerSession>> {
        let name_index = self.name_index.lock().await;
        let id = name_index.get(name)?;
        let sessions = self.sessions.lock().await;
        sessions.get(id).cloned()
    }

    /// List all active sessions with summary info.
    pub async fn list_sessions(&self) -> Vec<SessionInfo> {
        let sessions = self.sessions.lock().await;
        let mut result = Vec::with_capacity(sessions.len());

        for session in sessions.values() {
            let terminal_count = session.terminals.lock().await.len();
            let last_activity = *session.last_activity.lock().await;
            result.push(SessionInfo {
                id: session.id.clone(),
                name: session.name.clone(),
                workdir: session.workdir.clone(),
                terminal_count,
                created_at_elapsed_secs: session.created_at.elapsed().as_secs(),
                last_activity_elapsed_secs: last_activity.elapsed().as_secs(),
            });
        }

        // Sort by name (named first, then unnamed; alphabetical within groups).
        result.sort_by(|a, b| match (&a.name, &b.name) {
            (Some(na), Some(nb)) => na.cmp(nb),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.id.cmp(&b.id),
        });

        result
    }

    /// List session names and workdirs (for coordination server registration).
    pub async fn list_session_summaries(&self) -> Vec<(String, PathBuf)> {
        let sessions = self.sessions.lock().await;
        sessions
            .values()
            .filter_map(|s| {
                let name = s.name.as_ref()?;
                let workdir = s.workdir.as_ref()?;
                Some((name.clone(), workdir.clone()))
            })
            .collect()
    }

    /// Remove a named session by name.
    /// Returns true if a session was removed.
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
        let mut removed = Vec::new();

        // Collect IDs to remove (can't hold inner locks while mutating outer map)
        let mut to_remove = Vec::new();
        for (id, session) in sessions.iter() {
            let last = *session.last_activity.lock().await;
            if last.elapsed() > grace_period {
                to_remove.push(id.clone());
            }
        }

        for id in &to_remove {
            sessions.remove(id);
            removed.push(id.clone());
        }
        drop(sessions);

        // Also clean up name index.
        if !to_remove.is_empty() {
            let mut name_index = self.name_index.lock().await;
            name_index.retain(|_, v| !to_remove.contains(v));
        }

        removed
    }

    /// Get a session by ID (for external inspection/testing).
    pub async fn get(&self, session_id: &str) -> Option<Arc<ServerSession>> {
        self.sessions.lock().await.get(session_id).cloned()
    }
}

impl ServerSession {
    /// Allocate the next terminal ID for this session.
    pub async fn next_terminal_id(&self) -> String {
        let mut id = self.next_term_id.lock().await;
        let current = *id;
        *id += 1;
        format!("term-{}", current)
    }

    /// Add a notification to the backlog (for replay on reconnect).
    /// Backlog is capped at 1000 entries; oldest are evicted.
    pub async fn push_notification(&self, payload: Vec<u8>) {
        let mut seq = self.next_notif_seq.lock().await;
        let current_seq = *seq;
        *seq += 1;
        let mut backlog = self.notification_backlog.lock().await;
        backlog.push_back((current_seq, payload));
        while backlog.len() > 1000 {
            backlog.pop_front();
        }
    }

    /// Get notifications after a given sequence number.
    pub async fn notifications_after(&self, after_seq: u64) -> Vec<(u64, Vec<u8>)> {
        let backlog = self.notification_backlog.lock().await;
        backlog
            .iter()
            .filter(|(seq, _)| *seq > after_seq)
            .cloned()
            .collect()
    }

    /// Update all terminal notification senders to point to a new connection's
    /// write channel. Called on reconnect so PTY readers forward output to the
    /// current transport.
    pub async fn update_notif_senders(&self, sender: tokio::sync::mpsc::Sender<Vec<u8>>) {
        let terms = self.terminals.lock().await;
        for term in terms.values() {
            *term.notif_sender.lock().unwrap() = Some(sender.clone());
        }
    }

    /// Clear all terminal notification senders (e.g. when connection drops).
    pub async fn clear_notif_senders(&self) {
        let terms = self.terminals.lock().await;
        for term in terms.values() {
            *term.notif_sender.lock().unwrap() = None;
        }
    }

    /// Touch the session to update last_activity.
    pub async fn touch(&self) {
        *self.last_activity.lock().await = Instant::now();
    }

    /// Check if a device is allowed to access this session.
    /// If `allowed_devices` is empty, all trusted devices are allowed.
    pub fn is_device_allowed(&self, device_id: &str) -> bool {
        self.allowed_devices.is_empty() || self.allowed_devices.iter().any(|d| d == device_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn create_new_session() {
        let registry = SessionRegistry::new();
        let session = registry.resume_or_create(None, "token123").await;
        assert!(!session.id.is_empty());
        assert_eq!(session.auth_token, "token123");
        assert!(session.name.is_none());
        assert!(session.workdir.is_none());
    }

    #[tokio::test]
    async fn resume_existing_session() {
        let registry = SessionRegistry::new();
        let s1 = registry.resume_or_create(None, "tok").await;
        let id = s1.id.clone();

        let s2 = registry.resume_or_create(Some(&id), "tok").await;
        assert_eq!(s2.id, id);
    }

    #[tokio::test]
    async fn resume_wrong_token_creates_new() {
        let registry = SessionRegistry::new();
        let s1 = registry.resume_or_create(None, "tok1").await;
        let id = s1.id.clone();

        let s2 = registry.resume_or_create(Some(&id), "wrong").await;
        assert_ne!(s2.id, id);
    }

    #[tokio::test]
    async fn resume_nonexistent_creates_new() {
        let registry = SessionRegistry::new();
        let s = registry
            .resume_or_create(Some("does-not-exist"), "tok")
            .await;
        assert_ne!(s.id, "does-not-exist");
    }

    #[tokio::test]
    async fn notification_backlog() {
        let registry = SessionRegistry::new();
        let session = registry.resume_or_create(None, "tok").await;

        session.push_notification(b"msg1".to_vec()).await;
        session.push_notification(b"msg2".to_vec()).await;
        session.push_notification(b"msg3".to_vec()).await;

        let after_0 = session.notifications_after(0).await;
        assert_eq!(after_0.len(), 3);

        let after_1 = session.notifications_after(1).await;
        assert_eq!(after_1.len(), 2);
        assert_eq!(after_1[0].1, b"msg2");
    }

    #[tokio::test]
    async fn notification_backlog_cap() {
        let registry = SessionRegistry::new();
        let session = registry.resume_or_create(None, "tok").await;

        for i in 0..1050 {
            session
                .push_notification(format!("msg{}", i).into_bytes())
                .await;
        }

        let backlog = session.notification_backlog.lock().await;
        assert_eq!(backlog.len(), 1000);
        // next_notif_seq starts at 1, so seqs are 1..1050. After capping at
        // 1000, the first 50 entries (seqs 1..50) are evicted.
        assert_eq!(backlog.front().unwrap().0, 51);
    }

    #[tokio::test]
    async fn cleanup_removes_idle_sessions() {
        let registry = SessionRegistry::new();
        let s = registry.resume_or_create(None, "tok").await;
        let id = s.id.clone();

        // Manually set last_activity to the past
        *s.last_activity.lock().await = Instant::now() - Duration::from_secs(600);

        let removed = registry.cleanup(Duration::from_secs(300)).await;
        assert_eq!(removed, vec![id.clone()]);

        assert!(registry.get(&id).await.is_none());
    }

    #[tokio::test]
    async fn cleanup_keeps_active_sessions() {
        let registry = SessionRegistry::new();
        let s = registry.resume_or_create(None, "tok").await;
        let id = s.id.clone();

        let removed = registry.cleanup(Duration::from_secs(300)).await;
        assert!(removed.is_empty());
        assert!(registry.get(&id).await.is_some());
    }

    #[tokio::test]
    async fn terminal_id_generation() {
        let registry = SessionRegistry::new();
        let session = registry.resume_or_create(None, "tok").await;

        let id1 = session.next_terminal_id().await;
        let id2 = session.next_terminal_id().await;
        assert_eq!(id1, "term-1");
        assert_eq!(id2, "term-2");
    }

    // -- Named session tests --

    #[tokio::test]
    async fn create_named_session() {
        let registry = SessionRegistry::new();
        let session = registry
            .create_named("zedra", PathBuf::from("/home/user/projects/zedra"), "tok")
            .await;

        assert_eq!(session.name.as_deref(), Some("zedra"));
        assert_eq!(
            session.workdir.as_deref(),
            Some(std::path::Path::new("/home/user/projects/zedra"))
        );
    }

    #[tokio::test]
    async fn create_named_session_idempotent() {
        let registry = SessionRegistry::new();
        let s1 = registry
            .create_named("zedra", PathBuf::from("/home/user/zedra"), "tok1")
            .await;
        let s2 = registry
            .create_named("zedra", PathBuf::from("/home/user/zedra"), "tok2")
            .await;

        // Same session returned (idempotent by name).
        assert_eq!(s1.id, s2.id);
    }

    #[tokio::test]
    async fn get_by_name() {
        let registry = SessionRegistry::new();
        let s = registry
            .create_named("webapp", PathBuf::from("/home/user/webapp"), "tok")
            .await;

        let found = registry.get_by_name("webapp").await.unwrap();
        assert_eq!(found.id, s.id);

        assert!(registry.get_by_name("nonexistent").await.is_none());
    }

    #[tokio::test]
    async fn list_sessions_sorted() {
        let registry = SessionRegistry::new();
        registry
            .create_named("webapp", PathBuf::from("/webapp"), "tok")
            .await;
        registry
            .create_named("api", PathBuf::from("/api"), "tok")
            .await;
        registry.resume_or_create(None, "tok").await; // unnamed

        let list = registry.list_sessions().await;
        assert_eq!(list.len(), 3);
        // Named sessions first, alphabetical.
        assert_eq!(list[0].name.as_deref(), Some("api"));
        assert_eq!(list[1].name.as_deref(), Some("webapp"));
        assert!(list[2].name.is_none());
    }

    #[tokio::test]
    async fn remove_by_name() {
        let registry = SessionRegistry::new();
        registry
            .create_named("temp", PathBuf::from("/tmp"), "tok")
            .await;

        assert!(registry.get_by_name("temp").await.is_some());
        assert!(registry.remove_by_name("temp").await);
        assert!(registry.get_by_name("temp").await.is_none());
        assert!(!registry.remove_by_name("temp").await); // already gone
    }

    #[tokio::test]
    async fn cleanup_removes_named_sessions_and_index() {
        let registry = SessionRegistry::new();
        let s = registry
            .create_named("old", PathBuf::from("/old"), "tok")
            .await;

        *s.last_activity.lock().await = Instant::now() - Duration::from_secs(600);

        let removed = registry.cleanup(Duration::from_secs(300)).await;
        assert_eq!(removed.len(), 1);
        assert!(registry.get_by_name("old").await.is_none());
    }

    #[tokio::test]
    async fn list_session_summaries() {
        let registry = SessionRegistry::new();
        registry
            .create_named("proj1", PathBuf::from("/proj1"), "tok")
            .await;
        registry
            .create_named("proj2", PathBuf::from("/proj2"), "tok")
            .await;
        registry.resume_or_create(None, "tok").await; // unnamed, excluded

        let summaries = registry.list_session_summaries().await;
        assert_eq!(summaries.len(), 2);
    }

    #[tokio::test]
    async fn device_access_control() {
        let registry = SessionRegistry::new();
        let session = registry.resume_or_create(None, "tok").await;

        // Empty allowed_devices → all devices allowed.
        assert!(session.is_device_allowed("any-device"));

        // Create a session with restricted access.
        let restricted = Arc::new(ServerSession {
            id: "restricted".to_string(),
            auth_token: "tok".to_string(),
            name: Some("restricted".to_string()),
            workdir: None,
            created_at: Instant::now(),
            last_activity: Mutex::new(Instant::now()),
            terminals: Mutex::new(HashMap::new()),
            next_term_id: Mutex::new(1),
            notification_backlog: Mutex::new(VecDeque::new()),
            next_notif_seq: Mutex::new(1),
            allowed_devices: vec!["device-A".to_string(), "device-B".to_string()],
        });

        assert!(restricted.is_device_allowed("device-A"));
        assert!(restricted.is_device_allowed("device-B"));
        assert!(!restricted.is_device_allowed("device-C"));
    }
}
