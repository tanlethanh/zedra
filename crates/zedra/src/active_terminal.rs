/// Active terminal input routing.
///
/// The iOS keyboard accessory bar sends key input via a C FFI callback
/// (`zedra_ios_send_key_input`) without any GPUI context. This module
/// holds a single process-scoped sender for the active terminal. Terminal
/// activation and reconnect attach both refresh this slot, so native input
/// reads the current channel instead of a callback that captured an older one.
///
/// Nothing in `zedra-session` is involved — the routing is entirely
/// within the `zedra` crate.
use std::sync::{Mutex, OnceLock};

use tokio::sync::mpsc;
use tracing::warn;

#[derive(Clone)]
struct ActiveInput {
    terminal_id: String,
    sender: mpsc::Sender<Vec<u8>>,
}

static ACTIVE_INPUT: OnceLock<Mutex<Option<ActiveInput>>> = OnceLock::new();

fn slot() -> &'static Mutex<Option<ActiveInput>> {
    ACTIVE_INPUT.get_or_init(|| Mutex::new(None))
}

/// Register the input channel for the currently-active terminal.
///
/// Called by `WorkspaceView` whenever `active_terminal_id` changes.
pub fn set_active_input(terminal_id: String, sender: mpsc::Sender<Vec<u8>>) {
    if let Ok(mut slot) = slot().lock() {
        *slot = Some(ActiveInput {
            terminal_id,
            sender,
        });
    }
}

/// Clear the active input channel, but only if `terminal_id` still owns the slot.
///
/// A stale close (e.g. an old terminal tearing down after a newer one became
/// active) must not drop the current terminal's channel, so this compares the
/// stored id before clearing.
pub fn clear_active_input(terminal_id: &str) {
    if let Ok(mut slot) = slot().lock() {
        if slot
            .as_ref()
            .is_some_and(|active| active.terminal_id == terminal_id)
        {
            *slot = None;
        }
    }
}

/// Send bytes to the currently-active terminal.
///
/// Returns `true` if the data was accepted by the active channel.
pub fn send_to_active(data: Vec<u8>) -> bool {
    let active = slot().lock().ok().and_then(|slot| slot.clone());
    let Some(active) = active else {
        return false;
    };

    match active.sender.try_send(data) {
        Ok(()) => true,
        Err(error) => {
            warn!(
                terminal_id = active.terminal_id,
                "failed to send input: {}", error
            );
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc::error::TryRecvError;

    // Single test: the module holds one process-global slot, so parallel tests
    // would race on it.
    #[test]
    fn active_input_routing_and_ownership() {
        clear_active_input("second");

        let (first_tx, mut first_rx) = mpsc::channel(1);
        let (second_tx, mut second_rx) = mpsc::channel(1);
        set_active_input("first".to_string(), first_tx);
        set_active_input("second".to_string(), second_tx);

        // send_to_active reads the latest registered channel.
        assert!(send_to_active(b"\t".to_vec()));
        assert!(matches!(
            first_rx.try_recv(),
            Err(TryRecvError::Empty | TryRecvError::Disconnected)
        ));
        assert_eq!(second_rx.try_recv(), Ok(b"\t".to_vec()));

        // A stale close for a different terminal must not drop the active slot.
        clear_active_input("first");
        assert!(send_to_active(b"\t".to_vec()));
        assert_eq!(second_rx.try_recv(), Ok(b"\t".to_vec()));

        // The owning terminal clears it.
        clear_active_input("second");
        assert!(!send_to_active(b"\t".to_vec()));
    }
}
