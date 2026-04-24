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

use std::sync::OnceLock;

static SESSION_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

/// Returns the shared Tokio runtime for session/network work.
///
/// Use this from reusable session-layer code that may be called from GPUI or
/// other non-Tokio threads. Prefer this over bare `tokio::spawn()` unless the
/// caller is already guaranteed to be running inside the session runtime.
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
