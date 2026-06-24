pub mod connect;
pub mod handle;
pub mod session;
pub mod signer;
pub mod state;
pub mod terminal;

pub use connect::*;
pub use handle::*;
pub use session::*;
pub use state::*;
pub use terminal::*;

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

static ACTIVE_CONNECTIONS: OnceLock<Mutex<HashMap<usize, iroh::endpoint::Connection>>> =
    OnceLock::new();

fn active_connections() -> &'static Mutex<HashMap<usize, iroh::endpoint::Connection>> {
    ACTIVE_CONNECTIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn register_active_connection(conn: &iroh::endpoint::Connection) {
    if let Ok(mut active) = active_connections().lock() {
        active.insert(conn.stable_id(), conn.clone());
    }
}

pub(crate) fn unregister_active_connection(conn: &iroh::endpoint::Connection) {
    if let Ok(mut active) = active_connections().lock() {
        active.remove(&conn.stable_id());
    }
}

pub fn close_all_active_connections_for_lifecycle(reason: &'static [u8]) {
    // Lifecycle callbacks may run after GPUI state is unavailable; keep transport
    // release independent so the host can free its active-client slot promptly.
    let connections = active_connections()
        .lock()
        .map(|mut active| active.drain().map(|(_, conn)| conn).collect::<Vec<_>>())
        .unwrap_or_default();
    for conn in connections {
        conn.close(0u32.into(), reason);
    }
}
