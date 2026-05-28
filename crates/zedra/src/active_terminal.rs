/// Active terminal input routing.
///
/// Native keyboard accessory bars send key input via platform FFI callbacks
/// without any GPUI context. This module
/// holds a single process-scoped sender for the active terminal. Terminal
/// activation and reconnect attach both refresh this slot, so native input
/// reads the current channel instead of a callback that captured an older one.
///
/// Beyond raw text (`send_to_active`), this module also registers itself as
/// the active `accessory_input` consumer whenever a terminal becomes active,
/// owning the `Key` → byte encoding so the accessory bar stays focus-agnostic.
///
/// Nothing in `zedra-session` is involved — the routing is entirely
/// within the `zedra` crate.
use std::sync::{Arc, Mutex, OnceLock};

use tokio::sync::mpsc;
use tracing::warn;

use crate::accessory_input;
use crate::key_encoding::encode_legacy;

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
/// Called by `WorkspaceView` whenever `active_terminal_id` changes. Also
/// installs the keystroke consumer so the native accessory bar routes into
/// this same channel.
pub fn set_active_input(terminal_id: String, sender: mpsc::Sender<Vec<u8>>) {
    if let Ok(mut slot) = slot().lock() {
        *slot = Some(ActiveInput {
            terminal_id: terminal_id.clone(),
            sender: sender.clone(),
        });
    }
    let consumer_sender = sender;
    let consumer_terminal = terminal_id;
    accessory_input::set_active_consumer(
        "terminal",
        Arc::new(move |key, mods| {
            let bytes = encode_legacy(key, mods);
            match consumer_sender.try_send(bytes) {
                Ok(()) => true,
                Err(error) => {
                    warn!(
                        terminal_id = consumer_terminal,
                        "failed to send accessory keystroke: {}", error
                    );
                    false
                }
            }
        }),
    );
}

/// Clear the active input channel (e.g. when all terminals are closed) and
/// release the accessory keystroke consumer so the bar no longer hits a stale
/// channel.
pub fn clear_active_input() {
    if let Ok(mut slot) = slot().lock() {
        *slot = None;
    }
    accessory_input::clear_active_consumer();
}

/// Send bytes to the currently-active terminal.
///
/// Used by the iOS terminal composer to inject finalized text; bypasses the
/// keystroke encoder.
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
    use crate::key_encoding::Mods;
    use tokio::sync::mpsc::error::TryRecvError;

    #[test]
    fn send_to_active_reads_latest_registered_channel() {
        let _guard = accessory_input::test_serialize_lock().lock().unwrap();
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
    fn registering_active_terminal_routes_accessory_keys_through_encoder() {
        let _guard = accessory_input::test_serialize_lock().lock().unwrap();
        clear_active_input();

        let (tx, mut rx) = mpsc::channel(2);
        set_active_input("encoded".to_string(), tx);

        assert!(accessory_input::dispatch("char:c", Mods::CTRL.0));
        assert_eq!(rx.try_recv(), Ok(b"\x03".to_vec()));

        assert!(accessory_input::dispatch("tab", Mods::SHIFT.0));
        assert_eq!(rx.try_recv(), Ok(b"\x1b[Z".to_vec()));

        clear_active_input();
        // Once cleared, accessory keystrokes no longer reach this channel.
        assert!(!accessory_input::dispatch("char:c", 0));
    }
}
