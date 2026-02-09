//! Pan gesture recognizer with velocity tracking

use crate::state::GestureState;
use crate::types::{Point, TouchAction, TouchEvent, Vector2};
use crate::velocity::VelocityTracker;

use super::GestureRecognizer;

/// Configuration for pan gesture
#[derive(Clone, Debug)]
pub struct PanConfig {
    /// Minimum distance to start recognizing (slop)
    pub min_distance: f32,
    /// Minimum number of fingers
    pub min_fingers: usize,
    /// Maximum number of fingers
    pub max_fingers: usize,
    /// Whether to track velocity for fling
    pub track_velocity: bool,
    /// Minimum velocity for fling (pixels per second)
    pub min_fling_velocity: f32,
}

impl Default for PanConfig {
    fn default() -> Self {
        Self {
            min_distance: 10.0,
            min_fingers: 1,
            max_fingers: 1,
            track_velocity: true,
            min_fling_velocity: 50.0,
        }
    }
}

/// Event emitted by pan gesture
#[derive(Clone, Debug)]
pub struct PanGestureEvent {
    /// Current position
    pub position: Point,
    /// Translation from start position
    pub translation: Vector2,
    /// Translation delta since last event
    pub delta: Vector2,
    /// Current velocity (if tracking enabled)
    pub velocity: Vector2,
    /// Gesture state
    pub state: GestureState,
    /// Number of active fingers
    pub number_of_fingers: usize,
}

/// Pan gesture recognizer
pub struct PanGesture {
    config: PanConfig,
    state: GestureState,
    enabled: bool,

    // Tracking state
    start_position: Option<Point>,
    last_position: Option<Point>,
    translation: Vector2,
    velocity_tracker: VelocityTracker,
    current_event: Option<PanGestureEvent>,
}

impl PanGesture {
    pub fn new() -> Self {
        Self::with_config(PanConfig::default())
    }

    pub fn with_config(config: PanConfig) -> Self {
        Self {
            config,
            state: GestureState::Possible,
            enabled: true,
            start_position: None,
            last_position: None,
            translation: Vector2::zero(),
            velocity_tracker: VelocityTracker::new(),
            current_event: None,
        }
    }

    /// Set minimum distance to start recognizing
    pub fn min_distance(mut self, distance: f32) -> Self {
        self.config.min_distance = distance;
        self
    }

    /// Set number of fingers (single value for exact match)
    pub fn number_of_fingers(mut self, count: usize) -> Self {
        self.config.min_fingers = count;
        self.config.max_fingers = count;
        self
    }

    /// Set finger range
    pub fn fingers_range(mut self, min: usize, max: usize) -> Self {
        self.config.min_fingers = min;
        self.config.max_fingers = max;
        self
    }

    /// Get the current event if any
    pub fn event(&self) -> Option<&PanGestureEvent> {
        self.current_event.as_ref()
    }

    /// Take the current event
    pub fn take_event(&mut self) -> Option<PanGestureEvent> {
        self.current_event.take()
    }

    /// Get current translation
    pub fn translation(&self) -> Vector2 {
        self.translation
    }

    /// Get current velocity
    pub fn velocity(&self) -> Vector2 {
        self.velocity_tracker.velocity()
    }

    fn emit_event(&mut self, state: GestureState, position: Point, delta: Vector2, fingers: usize) {
        self.current_event = Some(PanGestureEvent {
            position,
            translation: self.translation,
            delta,
            velocity: self.velocity_tracker.velocity(),
            state,
            number_of_fingers: fingers,
        });
    }
}

impl Default for PanGesture {
    fn default() -> Self {
        Self::new()
    }
}

impl GestureRecognizer for PanGesture {
    fn on_touch(&mut self, event: &TouchEvent) {
        if !self.enabled {
            return;
        }

        self.current_event = None;
        let fingers = event.pointer_count();

        match event.action {
            TouchAction::Down | TouchAction::PointerDown(_) => {
                // Check finger count
                if fingers < self.config.min_fingers || fingers > self.config.max_fingers {
                    if self.state.is_active() {
                        // Too many/few fingers, cancel
                        self.state = GestureState::Cancelled;
                        self.emit_event(
                            GestureState::Cancelled,
                            event.center(),
                            Vector2::zero(),
                            fingers,
                        );
                    }
                    return;
                }

                let center = event.center();

                if self.start_position.is_none() {
                    self.start_position = Some(center);
                    self.last_position = Some(center);
                    self.translation = Vector2::zero();
                    self.velocity_tracker.clear();
                    self.state = GestureState::Possible;
                }

                if self.config.track_velocity {
                    self.velocity_tracker.add_sample(center);
                }
            }

            TouchAction::Move => {
                if fingers < self.config.min_fingers || fingers > self.config.max_fingers {
                    return;
                }

                let Some(start) = self.start_position else {
                    return;
                };
                let Some(last) = self.last_position else {
                    return;
                };

                let current = event.center();
                let delta = current - last;

                // Update velocity tracking
                if self.config.track_velocity {
                    self.velocity_tracker.add_sample(current);
                }

                // Check if we should start the gesture
                if self.state == GestureState::Possible {
                    let distance = start.distance_to(current);
                    if distance >= self.config.min_distance {
                        self.state = GestureState::Began;
                        self.translation = current - start;
                        self.emit_event(GestureState::Began, current, delta, fingers);
                    }
                } else if self.state == GestureState::Began || self.state == GestureState::Changed {
                    self.state = GestureState::Changed;
                    self.translation = current - start;
                    self.emit_event(GestureState::Changed, current, delta, fingers);
                }

                self.last_position = Some(current);
            }

            TouchAction::Up | TouchAction::PointerUp(_) => {
                let remaining = match event.action {
                    TouchAction::Up => 0,
                    TouchAction::PointerUp(_) => fingers.saturating_sub(1),
                    _ => fingers,
                };

                if self.state.is_active() {
                    if remaining == 0 {
                        // All fingers up
                        self.state = GestureState::Ended;
                        let position = self.last_position.unwrap_or(event.center());
                        self.emit_event(GestureState::Ended, position, Vector2::zero(), 0);
                        self.reset();
                    } else if remaining < self.config.min_fingers {
                        // Not enough fingers remaining
                        self.state = GestureState::Cancelled;
                        let position = self.last_position.unwrap_or(event.center());
                        self.emit_event(GestureState::Cancelled, position, Vector2::zero(), remaining);
                        self.reset();
                    }
                    // Otherwise continue with remaining fingers
                } else if remaining == 0 {
                    self.reset();
                }
            }

            TouchAction::Cancel => {
                if self.state.is_active() {
                    self.state = GestureState::Cancelled;
                    let position = self.last_position.unwrap_or_default();
                    self.emit_event(GestureState::Cancelled, position, Vector2::zero(), 0);
                }
                self.reset();
            }
        }
    }

    fn state(&self) -> GestureState {
        self.state
    }

    fn reset(&mut self) {
        self.state = GestureState::Possible;
        self.start_position = None;
        self.last_position = None;
        self.translation = Vector2::zero();
        self.velocity_tracker.clear();
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.reset();
        }
    }
}
