// Client library for connecting to a zedra-host RPC daemon.
//
// Key type:
//   SessionHandle  — workspace-scoped durable state (credentials, terminals, reconnect,
//                    and the live RPC client). Arc-wrapped, cheap to clone, survives
//                    transport failures.

pub mod connect_state;
pub mod handle;
pub mod signer;
pub mod terminal;

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

/// A boxed callback to execute on the main thread.
pub type MainCallback = Box<dyn FnOnce() + Send + 'static>;

static SESSION_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
static MAIN_THREAD_CALLBACKS: OnceLock<Mutex<VecDeque<MainCallback>>> = OnceLock::new();

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

fn callback_queue() -> &'static Mutex<VecDeque<MainCallback>> {
    MAIN_THREAD_CALLBACKS.get_or_init(|| Mutex::new(VecDeque::new()))
}

/// Enqueue a closure to run on the main thread at the next frame boundary.
///
/// Draining this queue also signals the frame loop to call `request_frame_forced()`.
/// Use an empty closure `Box::new(|| {})` as a pure "please re-render" signal.
pub fn push_callback(cb: MainCallback) {
    if let Ok(mut queue) = callback_queue().lock() {
        queue.push_back(cb);
    }
}

/// Drain all pending main-thread callbacks; call from the frame loop.
/// Returns the callbacks so the caller can execute them. Non-empty return value
/// means the caller should force a render on the next frame.
pub fn drain_callbacks() -> VecDeque<MainCallback> {
    if let Ok(mut queue) = callback_queue().lock() {
        std::mem::take(&mut *queue)
    } else {
        VecDeque::new()
    }
}

// Re-exports for crate consumers.
pub use connect_state::{
    AuthOutcome, ConnectError, ConnectPhase, ConnectSnapshot, ConnectState, ReconnectReason,
    STEPPER_STEP_NAMES, TransportSnapshot,
};
pub use handle::SessionHandle;
pub use terminal::{OscEvent, RemoteTerminal, ShellState, TerminalMeta};
