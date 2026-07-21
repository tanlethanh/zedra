//! Host-managed agent web clients.
//!
//! A web client is one entry in the app's card list: a webview the app opens
//! over the tunnel onto an agent's web UI. This module is agent-agnostic. It
//! owns the card registry (title/state/path per card, broadcast to the app) and
//! a process pool, and delegates everything server-specific to the agent actor
//! through [`AgentActor::web_client_open`](crate::agent::AgentActor::web_client_open):
//! how to launch a server, whether one server backs many cards, how to read a
//! card's live title/state. opencode, for one, runs a single shared
//! `opencode serve` and opens a fresh session per card; another agent could run
//! one process per card. That choice lives in the actor, never here.
//!
//! The [`ServerPool`] is the seam that lets an actor share a process without the
//! manager knowing: the actor picks a pool key (a constant to share, a unique
//! key per card not to). The pool refcounts by key and reaps a server when its
//! last card closes. It is owned here, daemon-scoped, so a server dies with the
//! daemon; actor-held [`WebClientSink`]/[`WebClientPool`] handles are weak and
//! never keep it alive.

use std::collections::HashMap;
use std::net::{Ipv4Addr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use tokio::process::{Child, Command};
use tokio::sync::{broadcast, Mutex};
use zedra_rpc::proto::{AgentState, WebClientInfo, WebClientUpdate};

use crate::agent;

/// How long a freshly spawned server has to start accepting connections.
const READY_TIMEOUT: Duration = Duration::from_secs(15);
/// Poll interval while waiting for readiness.
const READY_POLL: Duration = Duration::from_millis(200);
/// Update channel depth; a slow watcher drops old updates, never blocks a server.
const UPDATE_CAPACITY: usize = 64;

/// One card being opened. Sink updates are buffered here until the actor
/// returns a usable port and path, so early title/state updates are not lost.
struct Opening {
    seq: u64,
    slug: String,
    title: Option<String>,
    state: AgentState,
}

/// One card in the app's list — a web session on some agent's server.
struct Running {
    /// Creation order. The registry is a hash map, so this is what gives `list`
    /// a stable order — without it cards reshuffle on every reconnect.
    seq: u64,
    slug: String,
    port: u16,
    title: Option<String>,
    state: AgentState,
    /// Where the app reopens this card; seeded by the actor and moved by
    /// `set_path` as the user navigates the web UI.
    path: String,
}

enum CardEntry {
    Opening(Opening),
    Running(Running),
}

/// A pooled server process and how many cards depend on it.
struct Pooled {
    process: ServerProcess,
    refs: usize,
}

struct Inner {
    cards: Mutex<HashMap<String, CardEntry>>,
    updates: broadcast::Sender<WebClientUpdate>,
    workdir: PathBuf,
    next_seq: AtomicU64,
    /// Shared server processes, keyed by an actor-chosen pool key.
    pool: Mutex<HashMap<String, Pooled>>,
}

/// Daemon-scoped registry of host-managed web clients.
pub struct WebClientManager {
    inner: Arc<Inner>,
}

impl WebClientManager {
    pub fn new(workdir: PathBuf) -> Self {
        let (updates, _) = broadcast::channel(UPDATE_CAPACITY);
        Self {
            inner: Arc::new(Inner {
                cards: Mutex::new(HashMap::new()),
                updates,
                workdir,
                next_seq: AtomicU64::new(0),
                pool: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Open a new card for `slug`. The actor launches or reuses its server and
    /// returns the card's port and path; the card appears in the registry and
    /// on the watch stream.
    pub async fn start(&self, slug: &str) -> Result<WebClientInfo, String> {
        let actor = agent::actor(slug).ok_or_else(|| format!("unknown agent: {slug}"))?;
        if !actor.has_web_client() {
            return Err(format!("agent {slug} has no web client"));
        }
        let id = uuid::Uuid::new_v4().to_string();
        let seq = self.inner.next_seq.fetch_add(1, Ordering::Relaxed);
        self.inner.cards.lock().await.insert(
            id.clone(),
            CardEntry::Opening(Opening {
                seq,
                slug: slug.to_string(),
                title: None,
                state: AgentState::Idle,
            }),
        );
        let ctx = WebClientOpenCtx {
            id: id.clone(),
            workdir: self.inner.workdir.clone(),
            sink: self.sink(id.clone()),
            pool: self.pool(),
        };
        let opened = match actor.web_client_open(ctx).await {
            Ok(opened) => opened,
            Err(error) => {
                self.inner.remove_opening(&id).await;
                return Err(error);
            }
        };
        let Some(info) = self.inner.activate(&id, opened).await else {
            actor
                .web_client_close(WebClientCloseCtx {
                    id,
                    pool: self.pool(),
                })
                .await;
            return Err("web client closed while opening".to_string());
        };
        Ok(info)
    }

    /// Close the card with `id`: tell its actor (which reaps the server when the
    /// last card on it closes), then drop the card and broadcast the close.
    pub async fn stop(&self, id: &str) -> Result<(), String> {
        let running = {
            let mut cards = self.inner.cards.lock().await;
            match cards.remove(id) {
                Some(CardEntry::Running(running)) => running,
                Some(entry @ CardEntry::Opening(_)) => {
                    cards.insert(id.to_string(), entry);
                    return Err(format!("no web client with id {id}"));
                }
                None => return Err(format!("no web client with id {id}")),
            }
        };
        if let Some(actor) = agent::actor(&running.slug) {
            actor
                .web_client_close(WebClientCloseCtx {
                    id: id.to_string(),
                    pool: self.pool(),
                })
                .await;
        }
        self.inner.broadcast_closed(id, running);
        Ok(())
    }

    /// Record where the user navigated inside `id`'s web UI and broadcast it.
    /// Re-reporting the current path is a no-op: the app reports the route on
    /// page load too, which is the path it just opened.
    pub async fn set_path(&self, id: &str, path: &str) -> Result<(), String> {
        match self.inner.cards.lock().await.get(id) {
            Some(CardEntry::Running(running)) if running.path == path => return Ok(()),
            Some(CardEntry::Running(_)) => {}
            None => return Err(format!("no web client with id {id}")),
            Some(CardEntry::Opening(_)) => return Err(format!("no web client with id {id}")),
        }
        self.inner
            .apply(id, None, None, Some(path.to_string()))
            .await;
        Ok(())
    }

    /// Snapshot in creation order, so the app's cards keep a stable position
    /// across reconnects (the registry itself is unordered).
    pub async fn list(&self) -> Vec<WebClientInfo> {
        let cards = self.inner.cards.lock().await;
        let mut clients: Vec<(u64, WebClientInfo)> = cards
            .iter()
            .filter_map(|(id, entry)| {
                let CardEntry::Running(r) = entry else {
                    return None;
                };
                Some((
                    r.seq,
                    WebClientInfo {
                        id: id.clone(),
                        slug: r.slug.clone(),
                        port: r.port,
                        title: r.title.clone(),
                        state: r.state,
                        path: r.path.clone(),
                    },
                ))
            })
            .collect();
        clients.sort_by_key(|(seq, _)| *seq);
        clients.into_iter().map(|(_, info)| info).collect()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<WebClientUpdate> {
        self.inner.updates.subscribe()
    }

    fn sink(&self, id: String) -> WebClientSink {
        WebClientSink {
            inner: Arc::downgrade(&self.inner),
            id,
        }
    }

    fn pool(&self) -> WebClientPool {
        WebClientPool {
            inner: Arc::downgrade(&self.inner),
        }
    }
}

impl Inner {
    async fn remove_opening(&self, id: &str) {
        let mut cards = self.cards.lock().await;
        if matches!(cards.get(id), Some(CardEntry::Opening(_))) {
            cards.remove(id);
        }
    }

    /// Activate a provisional card and publish its first complete snapshot.
    async fn activate(&self, id: &str, opened: WebClientOpened) -> Option<WebClientInfo> {
        let (info, update) = {
            let mut cards = self.cards.lock().await;
            let CardEntry::Opening(opening) = cards.remove(id)? else {
                return None;
            };
            let running = Running {
                seq: opening.seq,
                slug: opening.slug,
                port: opened.port,
                title: opening.title,
                state: opening.state,
                path: opened.path,
            };
            let info = running.info(id);
            let update = running.update(id, false);
            cards.insert(id.to_string(), CardEntry::Running(running));
            (info, update)
        };
        let _ = self.updates.send(update);
        Some(info)
    }

    /// Remove `id` and broadcast a closed update, once. Removal is the guard, so
    /// a user stop and a server-death close racing still emit a single close.
    async fn close_card(&self, id: &str) {
        if let Some(CardEntry::Running(running)) = self.cards.lock().await.remove(id) {
            self.broadcast_closed(id, running);
        }
    }

    fn broadcast_closed(&self, id: &str, running: Running) {
        let _ = self.updates.send(running.update(id, true));
    }

    /// Apply a live state, title, and/or path change and broadcast the full
    /// snapshot. Called with all `None` to broadcast the current snapshot
    /// unchanged (the initial "added" update).
    async fn apply(
        &self,
        id: &str,
        state: Option<AgentState>,
        title: Option<String>,
        path: Option<String>,
    ) {
        let update = {
            let mut cards = self.cards.lock().await;
            let Some(entry) = cards.get_mut(id) else {
                return;
            };
            match entry {
                CardEntry::Opening(opening) => {
                    if let Some(state) = state {
                        opening.state = state;
                    }
                    if let Some(title) = title {
                        opening.title = Some(title);
                    }
                    return;
                }
                CardEntry::Running(running) => {
                    if let Some(state) = state {
                        running.state = state;
                    }
                    if let Some(title) = title {
                        running.title = Some(title);
                    }
                    if let Some(path) = path {
                        running.path = path;
                    }
                    running.update(id, false)
                }
            }
        };
        let _ = self.updates.send(update);
    }
}

impl Running {
    fn info(&self, id: &str) -> WebClientInfo {
        WebClientInfo {
            id: id.to_string(),
            slug: self.slug.clone(),
            port: self.port,
            title: self.title.clone(),
            state: self.state,
            path: self.path.clone(),
        }
    }

    fn update(&self, id: &str, closed: bool) -> WebClientUpdate {
        WebClientUpdate {
            id: id.to_string(),
            slug: self.slug.clone(),
            port: self.port,
            title: self.title.clone(),
            state: self.state,
            closed,
            path: self.path.clone(),
        }
    }
}

/// Turn an authoritative list into stream updates relative to what one watcher
/// has already seen. Missing cards become explicit closes; current cards are
/// replayed in creation order.
pub(crate) fn reconcile_snapshot(
    sent: &mut HashMap<String, WebClientUpdate>,
    snapshot: Vec<WebClientInfo>,
) -> Vec<WebClientUpdate> {
    let mut current = HashMap::new();
    let mut updates = Vec::new();
    for info in snapshot {
        let update = WebClientUpdate {
            id: info.id,
            slug: info.slug,
            port: info.port,
            title: info.title,
            state: info.state,
            closed: false,
            path: info.path,
        };
        current.insert(update.id.clone(), update.clone());
        updates.push(update);
    }

    let mut closed: Vec<_> = sent
        .iter()
        .filter(|(id, _)| !current.contains_key(*id))
        .map(|(_, update)| {
            let mut update = update.clone();
            update.closed = true;
            update
        })
        .collect();
    closed.sort_by(|left, right| left.id.cmp(&right.id));
    closed.extend(updates);
    *sent = current;
    closed
}

// ---------------------------------------------------------------------------
// Actor-facing handles. Both hold a weak manager ref, so an actor may stash
// them for a card's lifetime without keeping the daemon's servers alive.
// ---------------------------------------------------------------------------

/// Everything an actor needs to open a card. The actor launches or reuses its
/// server (via `pool`), does whatever per-card setup it wants (opencode creates
/// a fresh session), and returns the card's [`WebClientOpened`]. It keeps `sink`
/// to push the card's live title/state until the card closes.
pub(crate) struct WebClientOpenCtx {
    pub id: String,
    pub workdir: PathBuf,
    pub sink: WebClientSink,
    pub pool: WebClientPool,
}

/// The card an actor produced from [`WebClientOpenCtx`].
pub(crate) struct WebClientOpened {
    /// Loopback port the app tunnels to.
    pub port: u16,
    /// URL path the app opens, e.g. opencode's `/<dir>/session/<id>`.
    pub path: String,
}

/// Passed to `web_client_close` so the actor can release its pool reference.
pub(crate) struct WebClientCloseCtx {
    pub id: String,
    pub pool: WebClientPool,
}

/// An actor uses this to publish a card's live title/state, and to close the
/// card when its server dies. No-op once the manager (or the card) is gone.
#[derive(Clone)]
pub(crate) struct WebClientSink {
    inner: Weak<Inner>,
    id: String,
}

impl WebClientSink {
    /// Publish a state and/or title change; either `None` leaves that field
    /// unchanged.
    pub(crate) async fn set(&self, state: Option<AgentState>, title: Option<String>) {
        if let Some(inner) = self.inner.upgrade() {
            inner.apply(&self.id, state, title, None).await;
        }
    }

    /// Mark the card closed — for an actor whose server exited on its own.
    pub(crate) async fn closed(&self) {
        if let Some(inner) = self.inner.upgrade() {
            inner.close_card(&self.id).await;
        }
    }
}

/// Shared server processes, keyed by an actor-chosen key. An actor that wants
/// one server behind many cards acquires the same key each open; one that wants
/// a process per card uses a unique key. Reaps a server when its refcount hits
/// zero.
#[derive(Clone)]
pub(crate) struct WebClientPool {
    inner: Weak<Inner>,
}

impl WebClientPool {
    /// Reuse the server under `key`, or spawn one from `spec` in the daemon's
    /// workdir. Bumps the refcount; pair every `acquire` with a `release`.
    pub(crate) async fn acquire(&self, key: &str, spec: &ServerSpec) -> Result<u16, String> {
        let inner = self
            .inner
            .upgrade()
            .ok_or_else(|| "web client manager stopped".to_string())?;
        let mut pool = inner.pool.lock().await;
        if let Some(pooled) = pool.get_mut(key) {
            pooled.refs += 1;
            return Ok(pooled.process.port);
        }
        let process = spawn_server(spec, &inner.workdir).await?;
        let port = process.port;
        pool.insert(key.to_string(), Pooled { process, refs: 1 });
        Ok(port)
    }

    /// Drop one reference to `key`; reaps (kills) the server at zero.
    pub(crate) async fn release(&self, key: &str) {
        if let Some(inner) = self.inner.upgrade() {
            let mut pool = inner.pool.lock().await;
            if let Some(pooled) = pool.get_mut(key) {
                pooled.refs = pooled.refs.saturating_sub(1);
                if pooled.refs == 0 {
                    pool.remove(key);
                }
            }
        }
    }

    /// Force-drop `key` regardless of refcount — for a server that has already
    /// died, so a later `acquire` respawns instead of handing back a dead port.
    pub(crate) async fn remove(&self, key: &str) {
        if let Some(inner) = self.inner.upgrade() {
            inner.pool.lock().await.remove(key);
        }
    }
}

/// How to launch a web-client server the pool spawns on demand.
pub(crate) struct ServerSpec {
    /// Executable resolved on `PATH` (e.g. `opencode`).
    pub program: String,
    /// Build the child argv for a chosen loopback `port`.
    pub args: fn(u16) -> Vec<String>,
    /// Extra environment for the child process.
    pub env: Vec<(String, String)>,
}

/// A spawned loopback server. Killed on drop, so the pool reaps a server by
/// dropping it and the daemon reaps all of them on shutdown.
pub(crate) struct ServerProcess {
    pub port: u16,
    _child: Child,
}

/// Spawn `spec` in `workdir` on a free loopback port and wait until it listens.
pub(crate) async fn spawn_server(
    spec: &ServerSpec,
    workdir: &Path,
) -> Result<ServerProcess, String> {
    let port = free_loopback_port()?;
    let mut cmd = Command::new(&spec.program);
    cmd.args((spec.args)(port))
        .current_dir(workdir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    for (key, value) in &spec.env {
        cmd.env(key, value);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to launch {}: {e}", spec.program))?;
    if !wait_ready(port).await {
        let _ = child.start_kill();
        return Err(format!(
            "{} did not start listening on :{port}",
            spec.program
        ));
    }
    tracing::info!(
        "web-client: {} server listening on 127.0.0.1:{port}",
        spec.program
    );
    Ok(ServerProcess {
        port,
        _child: child,
    })
}

/// A free loopback port. Small TOCTOU window: the server binds it right after.
fn free_loopback_port() -> Result<u16, String> {
    TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .and_then(|listener| listener.local_addr())
        .map(|addr| addr.port())
        .map_err(|e| format!("no free loopback port: {e}"))
}

/// Poll until the loopback server answers HTTP (any status = listening).
async fn wait_ready(port: u16) -> bool {
    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{port}/");
    let deadline = Instant::now() + READY_TIMEOUT;
    while Instant::now() < deadline {
        if client
            .get(&url)
            .timeout(Duration::from_secs(1))
            .send()
            .await
            .is_ok()
        {
            return true;
        }
        tokio::time::sleep(READY_POLL).await;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_loopback_port_returns_a_usable_port() {
        let port = free_loopback_port().expect("a free loopback port");
        assert_ne!(port, 0);
    }

    #[test]
    fn web_capable_agents_advertise_the_affordance() {
        assert!(agent::actor("opencode")
            .expect("opencode actor registered")
            .has_web_client());
        assert!(!agent::actor("claude")
            .expect("claude actor registered")
            .has_web_client());
    }

    /// `list` seeds every `WebClientWatch` subscriber, so a hash-map order would
    /// reshuffle the app's cards on each reconnect.
    #[tokio::test]
    async fn list_returns_clients_in_creation_order() {
        let manager = WebClientManager::new(PathBuf::from("/tmp"));
        for port in 0..8u16 {
            let seq = manager.inner.next_seq.fetch_add(1, Ordering::Relaxed);
            manager.inner.cards.lock().await.insert(
                format!("web-{port}"),
                CardEntry::Running(running_fixture(seq, 4096 + port)),
            );
        }
        let ports: Vec<u16> = manager.list().await.iter().map(|c| c.port).collect();
        assert_eq!(ports, (4096..4104).collect::<Vec<_>>());
    }

    fn running_fixture(seq: u64, port: u16) -> Running {
        Running {
            seq,
            slug: "opencode".to_string(),
            port,
            title: None,
            state: AgentState::Idle,
            path: "/dGVzdA".to_string(),
        }
    }

    #[tokio::test]
    async fn set_path_moves_the_snapshot_and_rejects_unknown_ids() {
        let manager = WebClientManager::new(PathBuf::from("/tmp"));
        let id = "web-1".to_string();
        manager
            .inner
            .cards
            .lock()
            .await
            .insert(id.clone(), CardEntry::Running(running_fixture(0, 4096)));
        let mut updates = manager.subscribe();

        manager
            .set_path(&id, "/dGVzdA/session/ses_1")
            .await
            .expect("path recorded");
        let update = updates.try_recv().expect("a broadcast update");
        assert_eq!(update.path, "/dGVzdA/session/ses_1");
        assert!(!update.closed);
        assert_eq!(manager.list().await[0].path, "/dGVzdA/session/ses_1");

        // The app reports the route on page load, i.e. the path it just opened.
        manager
            .set_path(&id, "/dGVzdA/session/ses_1")
            .await
            .expect("re-reporting the current path is a no-op");
        assert!(
            updates.try_recv().is_err(),
            "no update for an unchanged path"
        );

        assert!(manager.set_path("nope", "/x").await.is_err());
    }

    #[tokio::test]
    async fn opening_buffers_sink_updates_until_activation() {
        let manager = WebClientManager::new(PathBuf::from("/tmp"));
        let id = "web-1".to_string();
        manager.inner.cards.lock().await.insert(
            id.clone(),
            CardEntry::Opening(Opening {
                seq: 0,
                slug: "opencode".to_string(),
                title: None,
                state: AgentState::Idle,
            }),
        );
        let mut updates = manager.subscribe();

        manager
            .sink(id.clone())
            .set(Some(AgentState::Running), Some("Fresh session".to_string()))
            .await;
        assert!(updates.try_recv().is_err(), "opening cards stay private");

        let info = manager
            .inner
            .activate(
                &id,
                WebClientOpened {
                    port: 4096,
                    path: "/dGVzdA/session/ses_1".to_string(),
                },
            )
            .await
            .expect("opening card activates");
        assert_eq!(info.title.as_deref(), Some("Fresh session"));
        assert_eq!(info.state, AgentState::Running);
        let update = updates.try_recv().expect("one complete initial update");
        assert_eq!(update.title.as_deref(), Some("Fresh session"));
        assert_eq!(update.port, 4096);
        assert!(!update.closed);
    }

    #[tokio::test]
    async fn closing_an_opening_card_prevents_late_activation() {
        let manager = WebClientManager::new(PathBuf::from("/tmp"));
        let id = "web-1".to_string();
        manager.inner.cards.lock().await.insert(
            id.clone(),
            CardEntry::Opening(Opening {
                seq: 0,
                slug: "opencode".to_string(),
                title: None,
                state: AgentState::Idle,
            }),
        );
        let mut updates = manager.subscribe();

        manager.sink(id.clone()).closed().await;
        assert!(manager
            .inner
            .activate(
                &id,
                WebClientOpened {
                    port: 4096,
                    path: "/dGVzdA/session/ses_1".to_string(),
                },
            )
            .await
            .is_none());
        assert!(manager.list().await.is_empty());
        assert!(updates.try_recv().is_err());
    }

    #[tokio::test]
    async fn concurrent_stops_claim_the_card_once() {
        let manager = WebClientManager::new(PathBuf::from("/tmp"));
        let id = "web-1".to_string();
        let mut running = running_fixture(0, 4096);
        running.slug = "claude".to_string();
        manager
            .inner
            .cards
            .lock()
            .await
            .insert(id.clone(), CardEntry::Running(running));
        let mut updates = manager.subscribe();

        let (first, second) = tokio::join!(manager.stop(&id), manager.stop(&id));
        assert_ne!(first.is_ok(), second.is_ok());
        assert!(manager.list().await.is_empty());
        assert!(updates.try_recv().expect("one close update").closed);
        assert!(updates.try_recv().is_err(), "close is emitted once");
    }

    #[test]
    fn snapshot_reconciliation_closes_missing_and_replays_current_cards() {
        let mut sent = HashMap::from([
            (
                "gone".to_string(),
                running_fixture(0, 4096).update("gone", false),
            ),
            (
                "kept".to_string(),
                running_fixture(1, 4097).update("kept", false),
            ),
        ]);
        let current = vec![running_fixture(1, 4097).info("kept")];

        let updates = reconcile_snapshot(&mut sent, current);
        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0].id, "gone");
        assert!(updates[0].closed);
        assert_eq!(updates[1].id, "kept");
        assert!(!updates[1].closed);
        assert_eq!(sent.len(), 1);
        assert!(sent.contains_key("kept"));
    }

    /// A weak sink must not keep the daemon's servers alive; once the manager
    /// drops, pushing through a stale sink is a no-op, not a panic.
    #[tokio::test]
    async fn sink_is_inert_after_the_manager_drops() {
        let manager = WebClientManager::new(PathBuf::from("/tmp"));
        let sink = manager.sink("web-1".to_string());
        drop(manager);
        sink.set(Some(AgentState::Running), None).await;
        sink.closed().await;
    }
}
