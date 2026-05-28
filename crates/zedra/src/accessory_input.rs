/// Focus-aware routing for native keyboard accessory keystrokes.
///
/// The accessory bar (iOS / Android) is shared across the app, but who should
/// receive a tap depends on what currently has focus. This module owns a single
/// process-scoped "active consumer" slot: whichever view last claimed focus
/// installs a closure here, and the platform FFI dispatches every key into it.
///
/// Consumers decide what a `(Key, Mods)` pair means. A terminal turns it into
/// PTY bytes; a text input could move the caret or trigger an undo. Nothing in
/// this module assumes a terminal exists.
use std::sync::{Arc, Mutex, OnceLock};

use tracing::warn;

use crate::key_encoding::{HostOs, Key, Mods};

/// Handler invoked for every accessory keystroke that targets the active focus.
/// Returns `true` when the keystroke was consumed.
pub type Consumer = Arc<dyn Fn(&Key, Mods) -> bool + Send + Sync + 'static>;

#[derive(Clone)]
struct ActiveConsumer {
    label: &'static str,
    consumer: Consumer,
}

static ACTIVE: OnceLock<Mutex<Option<ActiveConsumer>>> = OnceLock::new();

/// Host OS associated with whichever focus claimed the active consumer slot.
/// Read by the native keyboard panel so it can pick a layout for the host's
/// shortcuts (Cmd vs Ctrl labels, mac-style Option keys, etc.).
static ACTIVE_HOST_OS: OnceLock<Mutex<HostOs>> = OnceLock::new();

fn slot() -> &'static Mutex<Option<ActiveConsumer>> {
    ACTIVE.get_or_init(|| Mutex::new(None))
}

fn host_os_slot() -> &'static Mutex<HostOs> {
    ACTIVE_HOST_OS.get_or_init(|| Mutex::new(HostOs::Unknown))
}

/// Set the host OS associated with the active focus. Callers should refresh
/// this whenever a different host becomes the active focus or whenever the
/// host OS becomes known for the first time on a session.
pub fn set_active_host_os(os: HostOs) {
    if let Ok(mut slot) = host_os_slot().lock() {
        *slot = os;
    }
}

/// Current host OS. Returns `Unknown` when no host has claimed focus yet —
/// callers should treat that as the macOS default per product direction.
pub fn active_host_os() -> HostOs {
    host_os_slot()
        .lock()
        .map(|guard| *guard)
        .unwrap_or(HostOs::Unknown)
}

/// Install `consumer` as the active accessory handler. Replaces any prior one.
///
/// `label` is a short static string used only for log messages — pick something
/// the developer can recognise (e.g. `"terminal"`, `"text-input"`).
pub fn set_active_consumer(label: &'static str, consumer: Consumer) {
    if let Ok(mut slot) = slot().lock() {
        *slot = Some(ActiveConsumer { label, consumer });
    }
}

/// Drop the active consumer. Subsequent dispatches no-op until something else
/// claims focus.
pub fn clear_active_consumer() {
    if let Ok(mut slot) = slot().lock() {
        *slot = None;
    }
}

/// Parse a wire `(name, mod_bits)` from the native bar and hand it to the
/// active consumer. Returns `true` when the keystroke was consumed.
pub fn dispatch(key_name: &str, mod_bits: u8) -> bool {
    let Some(key) = Key::parse(key_name) else {
        warn!(key_name, "unknown keyboard accessory key");
        return false;
    };
    let mods = Mods::from_bits(mod_bits);

    // Clone the `Arc` under the lock, then release before invoking the closure.
    // This keeps the closure alive even if another thread clears or replaces
    // the active consumer mid-dispatch, and avoids holding the slot across a
    // re-entrant call (a consumer free to install a different handler).
    let active = slot().lock().ok().and_then(|guard| guard.clone());
    let Some(active) = active else {
        return false;
    };
    let consumed = (active.consumer)(&key, mods);
    if !consumed {
        warn!(
            label = active.label,
            key_name, "active consumer rejected accessory key"
        );
    }
    consumed
}

/// Cross-module serialization for tests that touch the global active-consumer
/// slot (this module) or the active-input slot (`active_terminal`). Both
/// modules share state — they must run one at a time.
#[cfg(test)]
pub(crate) fn test_serialize_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn dispatch_calls_active_consumer_with_parsed_key_and_mods() {
        let _guard = test_serialize_lock().lock().unwrap();
        clear_active_consumer();

        let captured: Arc<Mutex<Vec<(Key, Mods)>>> = Arc::new(Mutex::new(Vec::new()));
        let captured_clone = captured.clone();
        set_active_consumer(
            "test",
            Arc::new(move |key, mods| {
                captured_clone.lock().unwrap().push((key.clone(), mods));
                true
            }),
        );

        assert!(dispatch("char:c", Mods::CTRL.0));
        assert!(dispatch("page_up", 0));
        assert!(!dispatch("bogus", 0));

        let log = captured.lock().unwrap();
        assert_eq!(
            log.as_slice(),
            &[(Key::Char('c'), Mods::CTRL), (Key::PgUp, Mods::NONE)]
        );

        clear_active_consumer();
        assert!(!dispatch("char:c", 0));
    }

    #[test]
    fn replacing_consumer_drops_the_previous_one() {
        let _guard = test_serialize_lock().lock().unwrap();
        clear_active_consumer();

        let first_hits = Arc::new(Mutex::new(0u32));
        let second_hits = Arc::new(Mutex::new(0u32));
        let first_clone = first_hits.clone();
        let second_clone = second_hits.clone();

        set_active_consumer(
            "first",
            Arc::new(move |_, _| {
                *first_clone.lock().unwrap() += 1;
                true
            }),
        );
        set_active_consumer(
            "second",
            Arc::new(move |_, _| {
                *second_clone.lock().unwrap() += 1;
                true
            }),
        );

        assert!(dispatch("escape", 0));
        assert_eq!(*first_hits.lock().unwrap(), 0);
        assert_eq!(*second_hits.lock().unwrap(), 1);

        clear_active_consumer();
    }
}
