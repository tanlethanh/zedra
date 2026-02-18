// Session registry: manages persistent server sessions that survive transport
// reconnections. Each session owns its terminal PTY sessions and notification
// backlog, allowing a mobile client to disconnect and reconnect without losing
// state.

use std::collections::{HashMap, VecDeque};
use std::io::Write;
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
    pub created_at: Instant,
    pub last_activity: Mutex<Instant>,
    pub terminals: Mutex<HashMap<String, TermSession>>,
    pub next_term_id: Mutex<u64>,
    pub notification_backlog: Mutex<VecDeque<(u64, Vec<u8>)>>,
    pub next_notif_seq: Mutex<u64>,
}

/// A live terminal session owned by a ServerSession.
/// The PTY reader is split off and owned by a background streaming task.
pub struct TermSession {
    pub writer: Box<dyn Write + Send>,
    pub master: Box<dyn portable_pty::MasterPty + Send>,
}

/// Registry of active sessions, keyed by session ID.
pub struct SessionRegistry {
    sessions: Mutex<HashMap<String, Arc<ServerSession>>>,
}

impl SessionRegistry {
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
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

        for id in to_remove {
            sessions.remove(&id);
            removed.push(id);
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
}
