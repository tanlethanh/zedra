//! Exact-port adapter: binds a real `127.0.0.1:<host-port>` listener on the
//! device so the webview loads the unmodified `http://localhost:<port>` with an
//! honest loopback origin. Each device port is owned by one host (endpoint id);
//! a second host colliding on the same port makes [`ensure`] return
//! `Unavailable`, which the orchestrator turns into the alias fallback. A
//! best-effort sniffer binds companion ports the page references so its
//! API/WS/SSE traffic reaches the same host.

use std::collections::{HashMap, HashSet};
use std::net::Ipv4Addr;
use std::sync::{Mutex, OnceLock};

use iroh::PublicKey;
use tokio::net::{TcpListener, TcpStream};

use super::bridge;

// `owner` reserves a port for a host before the fallible bind (race guard) and is
// the ownership source of truth. `bound` tracks the live accept task per port so
// the manager (`web_tunnel_manager.rs`) can stop a listener: aborting the task
// drops its `TcpListener`, which frees the device port for another app.
struct Bound {
    endpoint_id: PublicKey,
    task: tokio::task::JoinHandle<()>,
}

struct State {
    owner: Mutex<HashMap<u16, PublicKey>>,
    bound: Mutex<HashMap<u16, Bound>>,
}

fn state() -> &'static State {
    static STATE: OnceLock<State> = OnceLock::new();
    STATE.get_or_init(|| State {
        owner: Mutex::new(HashMap::new()),
        bound: Mutex::new(HashMap::new()),
    })
}

/// A live exact-port listener, for the manager view to display and stop.
pub(crate) struct ListenerInfo {
    pub(crate) port: u16,
    pub(crate) endpoint_id: PublicKey,
}

/// Live listeners sorted by device port.
pub(crate) fn list_listeners() -> Vec<ListenerInfo> {
    let mut listeners: Vec<ListenerInfo> = state()
        .bound
        .lock()
        .unwrap()
        .iter()
        .map(|(&port, bound)| ListenerInfo {
            port,
            endpoint_id: bound.endpoint_id,
        })
        .collect();
    listeners.sort_by_key(|listener| listener.port);
    listeners
}

/// Stop the listener on `port`, freeing the device port. Aborting the accept task
/// drops its `TcpListener`. Returns whether a listener was present.
pub(crate) fn stop(port: u16) -> bool {
    state().owner.lock().unwrap().remove(&port);
    match state().bound.lock().unwrap().remove(&port) {
        Some(bound) => {
            bound.task.abort();
            tracing::info!("[debug:web-tunnel] exact-port stopped 127.0.0.1:{port}");
            true
        }
        None => false,
    }
}

/// Serve `port` for `endpoint_id` on the device loopback. `Err(())` means the
/// port is owned by a different host or the bind failed (another app / OS
/// restriction) — the caller falls back to the alias adapter.
pub(super) async fn ensure(endpoint_id: PublicKey, port: u16) -> Result<(), ()> {
    // Reserve ownership before the fallible bind so racing callers for the same
    // host reuse the listener and racing callers for another host get Err.
    {
        let mut owner = state().owner.lock().unwrap();
        match owner.get(&port) {
            Some(existing) if *existing == endpoint_id => return Ok(()),
            Some(_) => return Err(()),
            None => {
                owner.insert(port, endpoint_id);
            }
        }
    }
    match TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await {
        Ok(listener) => {
            tracing::info!("[debug:web-tunnel] exact-port bound 127.0.0.1:{port}");
            let task = spawn_accept_loop(listener, endpoint_id);
            state()
                .bound
                .lock()
                .unwrap()
                .insert(port, Bound { endpoint_id, task });
            Ok(())
        }
        Err(_) => {
            state().owner.lock().unwrap().remove(&port);
            Err(())
        }
    }
}

fn spawn_accept_loop(listener: TcpListener, endpoint_id: PublicKey) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let (stream, _) = match listener.accept().await {
                Ok(accepted) => accepted,
                Err(error) => {
                    tracing::warn!("[debug:web-tunnel] exact-port accept failed: {error}");
                    break;
                }
            };
            tokio::spawn(handle_connection(stream, endpoint_id));
        }
    })
}

// The accepted socket's local port is the listener's port — i.e. the host port
// the page asked for.
async fn handle_connection(stream: TcpStream, endpoint_id: PublicKey) {
    let port = stream.local_addr().map(|a| a.port()).unwrap_or(0);
    let Some(session) = super::session_for(&endpoint_id) else {
        return;
    };
    let (tx, rx, initial) = match bridge::connect(&session, port).await {
        Ok(parts) => parts,
        Err(error) => {
            tracing::info!("[debug:web-tunnel] exact-port connect {port} failed: {error}");
            return;
        }
    };
    let mut sniffer = CompanionSniffer::new(endpoint_id);
    bridge::pump(stream, tx, rx, initial, move |data| sniffer.scan(data)).await;
}

// A page's `localhost:<port>` mentions declare companion ports (a separate
// API/WS backend) it opens as soon as it loads, with no user action to hook.
// Scanning plaintext response bytes and eagerly binding a matching listener
// (owned by the same host) lets those same-page requests find a live socket.
// Best-effort: plaintext only (HTTPS bytes are opaque), capped at the first
// SNIFF_BYTE_LIMIT since a page declares its ports near the top of its HTML/JS.
const SNIFF_BYTE_LIMIT: usize = 64 * 1024;
const SNIFF_CARRY_LEN: usize = 32;

struct CompanionSniffer {
    endpoint_id: PublicKey,
    carry: Vec<u8>,
    scanned: usize,
    seen: HashSet<u16>,
}

impl CompanionSniffer {
    fn new(endpoint_id: PublicKey) -> Self {
        Self {
            endpoint_id,
            carry: Vec::new(),
            scanned: 0,
            seen: HashSet::new(),
        }
    }

    fn scan(&mut self, chunk: &[u8]) {
        if self.scanned >= SNIFF_BYTE_LIMIT {
            return;
        }
        self.scanned += chunk.len();
        self.carry.extend_from_slice(chunk);
        for port in find_localhost_ports(&self.carry) {
            if port != 0 && self.seen.insert(port) {
                let endpoint_id = self.endpoint_id;
                tokio::spawn(async move {
                    let _ = ensure(endpoint_id, port).await;
                });
            }
        }
        // Keep a small trailing window so a match split across two reads still
        // gets found, without retaining the whole response.
        let keep_from = self.carry.len().saturating_sub(SNIFF_CARRY_LEN);
        self.carry.drain(..keep_from);
    }
}

#[cfg(debug_assertions)]
pub(super) fn debug_clear_owners() {
    state().owner.lock().unwrap().clear();
}

#[cfg(debug_assertions)]
pub(super) fn debug_mark_foreign(port: u16) {
    // A deterministic key no real host will have, so the next `ensure` for this
    // port by a real host returns Unavailable (simulates a same-port collision).
    let foreign = iroh::SecretKey::from([0x7fu8; 32]).public();
    state().owner.lock().unwrap().insert(port, foreign);
}

fn find_localhost_ports(data: &[u8]) -> Vec<u16> {
    const NEEDLE: &[u8] = b"localhost:";
    let mut ports = Vec::new();
    let mut start = 0;
    while start + NEEDLE.len() <= data.len() {
        let Some(offset) = data[start..]
            .windows(NEEDLE.len())
            .position(|w| w == NEEDLE)
        else {
            break;
        };
        let digits_start = start + offset + NEEDLE.len();
        let mut digits_end = digits_start;
        while digits_end < data.len()
            && data[digits_end].is_ascii_digit()
            && digits_end - digits_start < 5
        {
            digits_end += 1;
        }
        if digits_end > digits_start {
            if let Ok(port) = std::str::from_utf8(&data[digits_start..digits_end])
                .unwrap_or_default()
                .parse::<u16>()
            {
                ports.push(port);
            }
        }
        start = start + offset + NEEDLE.len();
    }
    ports
}
