//! Gesture state machine

/// State of a gesture recognizer
///
/// Based on the state machine from react-native-gesture-handler / UIKit
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GestureState {
    /// Initial state - gesture has not started recognizing
    #[default]
    Possible,

    /// Gesture recognition has begun (e.g., finger down for pan)
    Began,

    /// Gesture is in progress and values are changing
    Changed,

    /// Gesture completed successfully
    Ended,

    /// Gesture was cancelled (e.g., interrupted by system)
    Cancelled,

    /// Gesture recognition failed (not this gesture type)
    Failed,
}

impl GestureState {
    /// Whether the gesture is currently active (began or changed)
    pub fn is_active(&self) -> bool {
        matches!(self, GestureState::Began | GestureState::Changed)
    }

    /// Whether the gesture has finished (ended, cancelled, or failed)
    pub fn is_finished(&self) -> bool {
        matches!(
            self,
            GestureState::Ended | GestureState::Cancelled | GestureState::Failed
        )
    }

    /// Whether we can transition to the given state
    pub fn can_transition_to(&self, next: GestureState) -> bool {
        use GestureState::*;
        match (self, next) {
            // From Possible
            (Possible, Began) => true,
            (Possible, Failed) => true,
            (Possible, Cancelled) => true,

            // From Began
            (Began, Changed) => true,
            (Began, Ended) => true,
            (Began, Cancelled) => true,
            (Began, Failed) => true,

            // From Changed
            (Changed, Changed) => true,
            (Changed, Ended) => true,
            (Changed, Cancelled) => true,

            // Terminal states can reset to Possible
            (Ended, Possible) => true,
            (Cancelled, Possible) => true,
            (Failed, Possible) => true,

            _ => false,
        }
    }

    /// Reset to initial state
    pub fn reset(&mut self) {
        *self = GestureState::Possible;
    }
}

/// Configuration for gesture behavior
#[derive(Clone, Debug)]
pub struct GestureConfig {
    /// Whether the gesture is enabled
    pub enabled: bool,

    /// Minimum number of pointers required
    pub min_pointers: usize,

    /// Maximum number of pointers allowed
    pub max_pointers: usize,

    /// Whether to cancel touches in the view when gesture begins
    pub cancel_touches_in_view: bool,
}

impl Default for GestureConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_pointers: 1,
            max_pointers: usize::MAX,
            cancel_touches_in_view: true,
        }
    }
}
