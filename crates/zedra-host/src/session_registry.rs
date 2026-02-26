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
use zedra_rpc::proto::{BacklogEntry, TermOutput};

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
    /// Per-terminal output backlog for replay on TermAttach reconnect.
    /// Stores raw PTY bytes with sequence numbers and terminal IDs.
    pub notification_backlog: Mutex<VecDeque<BacklogEntry>>,
    pub next_notif_seq: Mutex<u64>,
}

/// A live terminal session owned by a ServerSession.
/// The PTY reader is split off and owned by a background streaming task.
pub struct TermSession {
    pub writer: Box<dyn Write + Send>,
    pub master: Box<dyn portable_pty::MasterPty + Send>,
    /// Swappable output sender. Updated on each TermAttach. The PTY reader
    /// sends TermOutput through this channel, and a bridge task forwards to
    /// the current irpc stream. `None` when no client is attached.
    pub output_sender: Arc<std::sync::Mutex<Option<tokio::sync::mpsc::Sender<TermOutput>>>>,
    /// Server-side virtual terminal for screen state capture on reconnect.
    /// Fed every PTY output byte; `screen().state_formatted()` produces a
    /// compact ANSI dump (~2-10 KB) that restores the full screen state.
    pub vterm: Arc<std::sync::Mutex<vt100::Parser>>,
    /// Terminal dimensions (for TermList metadata).
    pub cols: u16,
    pub rows: u16,
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
// Manual Debug impl because inner types contain non-Debug fields (PTY handles).
pub struct SessionRegistry {
    sessions: Mutex<HashMap<String, Arc<ServerSession>>>,
    /// Index: session name → session ID. Enables fast lookup by name.
    name_index: Mutex<HashMap<String, String>>,
}

impl std::fmt::Debug for SessionRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionRegistry").finish_non_exhaustive()
    }
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
        });

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

    /// Add a backlog entry for terminal output replay on reconnect.
    /// Backlog is capped at 1000 entries; oldest are evicted.
    pub async fn push_backlog_entry(&self, entry: BacklogEntry) {
        let mut backlog = self.notification_backlog.lock().await;
        backlog.push_back(entry);
        while backlog.len() > 1000 {
            backlog.pop_front();
        }
    }

    /// Allocate the next backlog sequence number.
    pub async fn next_backlog_seq(&self) -> u64 {
        let mut seq = self.next_notif_seq.lock().await;
        let current = *seq;
        *seq += 1;
        current
    }

    /// Get backlog entries for a specific terminal after a given sequence number.
    pub async fn backlog_after(&self, terminal_id: &str, after_seq: u64) -> Vec<BacklogEntry> {
        let backlog = self.notification_backlog.lock().await;
        backlog
            .iter()
            .filter(|e| e.terminal_id == terminal_id && e.seq > after_seq)
            .cloned()
            .collect()
    }

    /// Clear all terminal output senders (e.g. when connection drops).
    /// PTY readers will continue storing output in the backlog but won't
    /// attempt to send to a dead channel.
    pub async fn clear_output_senders(&self) {
        let terms = self.terminals.lock().await;
        for term in terms.values() {
            *term.output_sender.lock().unwrap() = None;
        }
    }

    /// Touch the session to update last_activity.
    pub async fn touch(&self) {
        *self.last_activity.lock().await = Instant::now();
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

        // Different terminal ID returns empty
        let other = session.backlog_after("term-2", 0).await;
        assert!(other.is_empty());
    }

    #[tokio::test]
    async fn notification_backlog_cap() {
        let registry = SessionRegistry::new();
        let session = registry.resume_or_create(None, "tok").await;

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

        let backlog = session.notification_backlog.lock().await;
        assert_eq!(backlog.len(), 1000);
        // Seqs 1..1050. After capping at 1000, first 50 are evicted.
        assert_eq!(backlog.front().unwrap().seq, 51);
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

}
