/// Generic async-to-main-thread one-shot channel.
///
/// Each `PendingSlot<T>` is a `Mutex<Option<T>>` that can be written from any
/// thread (e.g. a tokio task) and consumed on the main/render thread.
use std::sync::Mutex;

pub struct PendingSlot<T>(Mutex<Option<T>>);

impl<T> PendingSlot<T> {
    pub const fn new() -> Self {
        Self(Mutex::new(None))
    }

    /// Store a value, overwriting any previous unconsumed value.
    pub fn set(&self, val: T) {
        *self.0.lock().unwrap() = Some(val);
    }

    /// Take the value if present, leaving `None`.
    pub fn take(&self) -> Option<T> {
        self.0.lock().unwrap().take()
    }
}

/// Arc-wrapped variant for per-instance (non-static) pending state.
pub type SharedPendingSlot<T> = std::sync::Arc<PendingSlot<T>>;

pub fn shared_pending_slot<T>() -> SharedPendingSlot<T> {
    std::sync::Arc::new(PendingSlot::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_take_round_trip() {
        let slot = PendingSlot::new();
        slot.set(42);
        assert_eq!(slot.take(), Some(42));
    }

    #[test]
    fn take_when_empty_returns_none() {
        let slot: PendingSlot<i32> = PendingSlot::new();
        assert_eq!(slot.take(), None);
    }

    #[test]
    fn set_overwrites_previous() {
        let slot = PendingSlot::new();
        slot.set(1);
        slot.set(2);
        assert_eq!(slot.take(), Some(2));
    }

    #[test]
    fn take_clears_slot() {
        let slot = PendingSlot::new();
        slot.set(99);
        assert_eq!(slot.take(), Some(99));
        assert_eq!(slot.take(), None);
    }

    #[test]
    fn shared_pending_slot_works() {
        let slot = shared_pending_slot::<String>();
        slot.set("hello".to_string());
        assert_eq!(slot.take(), Some("hello".to_string()));
    }
}
