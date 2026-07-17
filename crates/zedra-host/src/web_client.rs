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

/// A pooled server process and how many cards depend on it.
struct Pooled {
    process: ServerProcess,
    refs: usize,
}

struct Inner {
    running: Mutex<HashMap<String, Running>>,
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
                running: Mutex::new(HashMap::new()),
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
        let ctx = WebClientOpenCtx {
            id: id.clone(),
            workdir: self.inner.workdir.clone(),
            sink: self.sink(id.clone()),
            pool: self.pool(),
        };
        let opened = actor.web_client_open(ctx).await?;

        let seq = self.inner.next_seq.fetch_add(1, Ordering::Relaxed);
        self.inner.running.lock().await.insert(
            id.clone(),
            Running {
                seq,
                slug: slug.to_string(),
                port: opened.port,
                title: None,
                state: AgentState::Idle,
                path: opened.path.clone(),
            },
        );
        // Broadcast the initial snapshot so live subscribers see the new card.
        self.inner.apply(&id, None, None, None).await;

        Ok(WebClientInfo {
            id,
            slug: slug.to_string(),
            port: opened.port,
            title: None,
            state: AgentState::Idle,
            path: opened.path,
        })
    }

    /// Close the card with `id`: tell its actor (which reaps the server when the
    /// last card on it closes), then drop the card and broadcast the close.
    pub async fn stop(&self, id: &str) -> Result<(), String> {
        let slug = match self.inner.running.lock().await.get(id) {
            Some(running) => running.slug.clone(),
            None => return Err(format!("no web client with id {id}")),
        };
        if let Some(actor) = agent::actor(&slug) {
            actor
                .web_client_close(WebClientCloseCtx {
                    id: id.to_string(),
                    pool: self.pool(),
                })
                .await;
        }
        self.inner.close_card(id).await;
        Ok(())
    }

    /// Record where the user navigated inside `id`'s web UI and broadcast it.
    /// Re-reporting the current path is a no-op: the app reports the route on
    /// page load too, which is the path it just opened.
    pub async fn set_path(&self, id: &str, path: &str) -> Result<(), String> {
        match self.inner.running.lock().await.get(id) {
            Some(running) if running.path == path => return Ok(()),
            Some(_) => {}
            None => return Err(format!("no web client with id {id}")),
        }
        self.inner
            .apply(id, None, None, Some(path.to_string()))
            .await;
        Ok(())
    }

    /// Snapshot in creation order, so the app's cards keep a stable position
    /// across reconnects (the registry itself is unordered).
    pub async fn list(&self) -> Vec<WebClientInfo> {
        let running = self.inner.running.lock().await;
        let mut clients: Vec<(u64, WebClientInfo)> = running
            .iter()
            .map(|(id, r)| {
                (
                    r.seq,
                    WebClientInfo {
                        id: id.clone(),
                        slug: r.slug.clone(),
                        port: r.port,
                        title: r.title.clone(),
                        state: r.state,
                        path: r.path.clone(),
                    },
                )
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
    /// Remove `id` and broadcast a closed update, once. Removal is the guard, so
    /// a user stop and a server-death close racing still emit a single close.
    async fn close_card(&self, id: &str) {
        if let Some(entry) = self.running.lock().await.remove(id) {
            let _ = self.updates.send(WebClientUpdate {
                id: id.to_string(),
                slug: entry.slug,
                port: entry.port,
                title: entry.title,
                state: entry.state,
                closed: true,
                path: entry.path,
            });
        }
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
            let mut running = self.running.lock().await;
            let Some(entry) = running.get_mut(id) else {
                return;
            };
            if let Some(state) = state {
                entry.state = state;
            }
            if let Some(title) = title {
                entry.title = Some(title);
            }
            if let Some(path) = path {
                entry.path = path;
            }
            WebClientUpdate {
                id: id.to_string(),
                slug: entry.slug.clone(),
                port: entry.port,
                title: entry.title.clone(),
                state: entry.state,
                closed: false,
                path: entry.path.clone(),
            }
        };
        let _ = self.updates.send(update);
    }
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
            manager
                .inner
                .running
                .lock()
                .await
                .insert(format!("web-{port}"), running_fixture(seq, 4096 + port));
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
            .running
            .lock()
            .await
            .insert(id.clone(), running_fixture(0, 4096));
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
