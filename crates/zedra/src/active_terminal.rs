/// Active terminal input routing.
///
/// Native keyboard accessory bars send key input via platform FFI callbacks
/// without any GPUI context. This module
/// holds a single process-scoped sender for the active terminal. Terminal
/// activation and reconnect attach both refresh this slot, so native input
/// reads the current channel instead of a callback that captured an older one.
///
/// Nothing in `zedra-session` is involved — the routing is entirely
/// within the `zedra` crate.
use std::sync::{Mutex, OnceLock};

use tokio::sync::mpsc;
use tracing::warn;

use crate::key_encoding::{Key, Mods, encode_legacy};

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

/// Map a native accessory keystroke `(name, mods)` to bytes and send them.
///
/// `key_name` is a stable wire identifier — see `key_encoding::Key::parse`.
/// `mod_bits` packs Shift/Alt/Ctrl per `key_encoding::Mods`.
pub fn send_keystroke(key_name: &str, mod_bits: u8) -> bool {
    let Some(key) = Key::parse(key_name) else {
        warn!(key_name, "unknown keyboard accessory key");
        return false;
    };
    let bytes = encode_legacy(&key, Mods::from_bits(mod_bits));
    send_to_active(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tokio::sync::mpsc::error::TryRecvError;

    // The global `ACTIVE_INPUT` slot is shared across all tests in this module,
    // so they must not run concurrently.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn send_to_active_reads_latest_registered_channel() {
        let _guard = TEST_LOCK.lock().unwrap();
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

    #[test]
    fn send_keystroke_routes_modified_combo_to_active_channel() {
        let _guard = TEST_LOCK.lock().unwrap();
        clear_active_input();

        let (tx, mut rx) = mpsc::channel(2);
        set_active_input("modified".to_string(), tx);

        assert!(send_keystroke("char:c", Mods::CTRL.0));
        assert_eq!(rx.try_recv(), Ok(b"\x03".to_vec()));

        assert!(send_keystroke("tab", Mods::SHIFT.0));
        assert_eq!(rx.try_recv(), Ok(b"\x1b[Z".to_vec()));

        assert!(!send_keystroke("bogus", 0));

        clear_active_input();
    }
}
