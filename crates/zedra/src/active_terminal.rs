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

/// Clear the active input channel (e.g. when all terminals are closed).
pub fn clear_active_input() {
    if let Ok(mut slot) = slot().lock() {
        *slot = None;
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

    #[test]
    fn send_to_active_reads_latest_registered_channel() {
        clear_active_input();

        let (first_tx, mut first_rx) = mpsc::channel(1);
        let (second_tx, mut second_rx) = mpsc::channel(1);
        set_active_input("first".to_string(), first_tx);
        set_active_input("second".to_string(), second_tx);

        assert!(send_to_active(b"\t".to_vec()));
        assert!(matches!(
            first_rx.try_recv(),
            Err(TryRecvError::Empty | TryRecvError::Disconnected)
        ));
        assert_eq!(second_rx.try_recv(), Ok(b"\t".to_vec()));

        clear_active_input();
    }
}
