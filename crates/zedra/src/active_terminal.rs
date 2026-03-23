/// Active terminal input routing.
///
/// The iOS keyboard accessory bar sends key input via a C FFI callback
/// (`zedra_ios_send_key_input`) without any GPUI context. This module
/// holds a single process-scoped callback that is updated by `WorkspaceView`
/// whenever the active terminal changes, pointing directly at the right
/// `SessionHandle::send_terminal_input` call.
///
/// Nothing in `zedra-session` is involved — the routing is entirely
/// within the `zedra` crate.
use std::sync::{Mutex, OnceLock};

type InputFn = Box<dyn Fn(Vec<u8>) + Send + 'static>;

static ACTIVE_INPUT: OnceLock<Mutex<Option<InputFn>>> = OnceLock::new();

fn slot() -> &'static Mutex<Option<InputFn>> {
    ACTIVE_INPUT.get_or_init(|| Mutex::new(None))
}

/// Register a callback for the currently-active terminal.
///
/// Called by `WorkspaceView` whenever `active_terminal_id` changes.
/// The callback captures the `SessionHandle` and terminal ID, so the
/// caller doesn't need to pass them separately.
pub fn set_active_input(f: InputFn) {
    if let Ok(mut slot) = slot().lock() {
        *slot = Some(f);
    }
}

/// Clear the callback (e.g. when all terminals are closed).
pub fn clear_active_input() {
    if let Ok(mut slot) = slot().lock() {
        *slot = None;
    }
}

/// Send bytes to the currently-active terminal.
///
/// Returns `true` if a callback was registered and invoked.
pub fn send_to_active(data: Vec<u8>) -> bool {
    if let Ok(slot) = slot().lock() {
        if let Some(ref f) = *slot {
            f(data);
            return true;
        }
    }
    false
}
