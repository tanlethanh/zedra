//! zedra-gesture - Gesture recognition library for GPUI on Android
//!
//! Inspired by react-native-gesture-handler, this crate provides:
//! - Gesture recognizers (Tap, Pan, Pinch, LongPress)
//! - Velocity tracking with iOS-style momentum scrolling
//! - Element wrappers for easy integration with GPUI
//! - Gesture compositor for handling multiple recognizers
//!
//! # Quick Start
//!
//! ```ignore
//! use zedra_gesture::{pan_gesture, PanGestureEvent};
//!
//! fn my_view(cx: &mut Context<Self>) -> impl IntoElement {
//!     pan_gesture()
//!         .min_distance(10.0)
//!         .on_pan(|event, cx| {
//!             println!("Pan: {:?}", event.translation);
//!         })
//!         .child(
//!             div()
//!                 .size_full()
//!                 .bg(rgb(0x282c34))
//!                 .child("Drag me!")
//!         )
//! }
//! ```
//!
//! # Gesture Types
//!
//! - **Tap**: Single or multi-tap detection
//! - **LongPress**: Press and hold detection
//! - **Pan**: Drag/scroll with velocity tracking
//! - **Pinch**: Two-finger scale and rotation
//!
//! # Momentum Scrolling
//!
//! ```ignore
//! use zedra_gesture::{MomentumAnimator, DecelerationCurve};
//!
//! let mut animator = MomentumAnimator::new()
//!     .with_deceleration(DecelerationCurve::IOS);
//!
//! // Start with fling velocity
//! animator.start(velocity);
//!
//! // In animation loop
//! let delta = animator.update();
//! ```

pub mod compose;
pub mod compositor;
pub mod element;
pub mod recognizers;
pub mod state;
pub mod types;
pub mod velocity;

// Re-export main types
pub use compose::{
    exclusive, race, simultaneous, GestureExclusive, GestureRace, GestureSimultaneous,
};
pub use compositor::GestureCompositor;
pub use element::{pan_gesture, pinch_gesture, tap_gesture};
pub use element::{PanGestureElement, PinchGestureElement, TapGestureElement};
pub use recognizers::{
    GestureRecognizer, LongPressGesture, LongPressGestureEvent, PanGesture, PanGestureEvent,
    PinchGesture, PinchGestureEvent, TapGesture, TapGestureEvent,
};
pub use state::{GestureConfig, GestureState};
pub use types::{FlingDirection, GestureId, Point, TouchAction, TouchEvent, TouchPointer, Vector2};
pub use velocity::{DecelerationCurve, MomentumAnimator, MomentumBounds, VelocityTracker};
