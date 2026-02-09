//! Gesture recognizers

mod pan;
mod pinch;
mod tap;

pub use pan::{PanGesture, PanGestureEvent};
pub use pinch::{PinchGesture, PinchGestureEvent};
pub use tap::{LongPressGesture, LongPressGestureEvent, TapGesture, TapGestureEvent};

use crate::state::GestureState;
use crate::types::TouchEvent;

/// Trait for gesture recognizers
pub trait GestureRecognizer: Send + Sync {
    /// Process a touch event and update state
    fn on_touch(&mut self, event: &TouchEvent);

    /// Get current gesture state
    fn state(&self) -> GestureState;

    /// Reset the recognizer to initial state
    fn reset(&mut self);

    /// Whether this recognizer is enabled
    fn is_enabled(&self) -> bool;

    /// Enable or disable the recognizer
    fn set_enabled(&mut self, enabled: bool);
}
