//! Pinch (scale) and rotation gesture recognizers

use crate::state::GestureState;
use crate::types::{Point, TouchAction, TouchEvent};

use super::GestureRecognizer;

/// Configuration for pinch gesture
#[derive(Clone, Debug)]
pub struct PinchConfig {
    /// Minimum scale change to start recognizing
    pub min_scale_threshold: f32,
    /// Minimum span (distance between fingers) to start
    pub min_span: f32,
}

impl Default for PinchConfig {
    fn default() -> Self {
        Self {
            min_scale_threshold: 0.05, // 5% scale change
            min_span: 20.0,            // pixels
        }
    }
}

/// Event emitted by pinch gesture
#[derive(Clone, Debug)]
pub struct PinchGestureEvent {
    /// Current scale factor (1.0 = no change)
    pub scale: f32,
    /// Scale change since last event
    pub scale_delta: f32,
    /// Focal point (center between fingers)
    pub focal_point: Point,
    /// Current rotation in radians
    pub rotation: f32,
    /// Rotation change since last event
    pub rotation_delta: f32,
    /// Gesture state
    pub state: GestureState,
}

/// Pinch gesture recognizer (also tracks rotation)
pub struct PinchGesture {
    config: PinchConfig,
    state: GestureState,
    enabled: bool,

    // Tracking state
    initial_span: f32,
    initial_rotation: f32,
    last_span: f32,
    last_rotation: f32,
    current_scale: f32,
    current_rotation: f32,
    focal_point: Point,
    current_event: Option<PinchGestureEvent>,
}

impl PinchGesture {
    pub fn new() -> Self {
        Self::with_config(PinchConfig::default())
    }

    pub fn with_config(config: PinchConfig) -> Self {
        Self {
            config,
            state: GestureState::Possible,
            enabled: true,
            initial_span: 0.0,
            initial_rotation: 0.0,
            last_span: 0.0,
            last_rotation: 0.0,
            current_scale: 1.0,
            current_rotation: 0.0,
            focal_point: Point::zero(),
            current_event: None,
        }
    }

    /// Get the current event if any
    pub fn event(&self) -> Option<&PinchGestureEvent> {
        self.current_event.as_ref()
    }

    /// Take the current event
    pub fn take_event(&mut self) -> Option<PinchGestureEvent> {
        self.current_event.take()
    }

    /// Get current scale
    pub fn scale(&self) -> f32 {
        self.current_scale
    }

    /// Get current rotation
    pub fn rotation(&self) -> f32 {
        self.current_rotation
    }

    fn emit_event(&mut self, state: GestureState, scale_delta: f32, rotation_delta: f32) {
        self.current_event = Some(PinchGestureEvent {
            scale: self.current_scale,
            scale_delta,
            focal_point: self.focal_point,
            rotation: self.current_rotation,
            rotation_delta,
            state,
        });
    }

    /// Normalize angle to -PI to PI range
    fn normalize_angle(angle: f32) -> f32 {
        let mut a = angle;
        while a > std::f32::consts::PI {
            a -= 2.0 * std::f32::consts::PI;
        }
        while a < -std::f32::consts::PI {
            a += 2.0 * std::f32::consts::PI;
        }
        a
    }
}

impl Default for PinchGesture {
    fn default() -> Self {
        Self::new()
    }
}

impl GestureRecognizer for PinchGesture {
    fn on_touch(&mut self, event: &TouchEvent) {
        if !self.enabled {
            return;
        }

        self.current_event = None;

        // Pinch requires exactly 2 fingers
        let fingers = event.pointer_count();

        match event.action {
            TouchAction::Down => {
                // First finger down - wait for second
                self.state = GestureState::Possible;
            }

            TouchAction::PointerDown(_) => {
                if fingers == 2 {
                    // Second finger down - start tracking
                    self.initial_span = event.span();
                    self.initial_rotation = event.rotation_angle();
                    self.last_span = self.initial_span;
                    self.last_rotation = self.initial_rotation;
                    self.current_scale = 1.0;
                    self.current_rotation = 0.0;
                    self.focal_point = event.center();
                    self.state = GestureState::Possible;
                } else if fingers > 2 && self.state.is_active() {
                    // Too many fingers
                    self.state = GestureState::Cancelled;
                    self.emit_event(GestureState::Cancelled, 0.0, 0.0);
                    self.reset();
                }
            }

            TouchAction::Move => {
                if fingers != 2 {
                    return;
                }

                let span = event.span();
                let rotation = event.rotation_angle();
                self.focal_point = event.center();

                if span < self.config.min_span {
                    return;
                }

                // Calculate scale
                let new_scale = if self.initial_span > 0.0 {
                    span / self.initial_span
                } else {
                    1.0
                };

                // Calculate rotation delta
                let rotation_delta = Self::normalize_angle(rotation - self.last_rotation);

                // Check if we should begin
                if self.state == GestureState::Possible {
                    let scale_change = (new_scale - 1.0).abs();
                    let rotation_change = rotation_delta.abs();

                    if scale_change >= self.config.min_scale_threshold || rotation_change >= 0.05
                    // ~3 degrees
                    {
                        self.state = GestureState::Began;
                        self.current_scale = new_scale;
                        self.current_rotation =
                            Self::normalize_angle(rotation - self.initial_rotation);

                        let scale_delta = new_scale - self.current_scale;
                        self.emit_event(GestureState::Began, scale_delta, rotation_delta);
                    }
                } else if self.state.is_active() {
                    self.state = GestureState::Changed;

                    let scale_delta = new_scale - self.current_scale;
                    self.current_scale = new_scale;
                    self.current_rotation = Self::normalize_angle(rotation - self.initial_rotation);

                    self.emit_event(GestureState::Changed, scale_delta, rotation_delta);
                }

                self.last_span = span;
                self.last_rotation = rotation;
            }

            TouchAction::PointerUp(_) => {
                if self.state.is_active() {
                    self.state = GestureState::Ended;
                    self.emit_event(GestureState::Ended, 0.0, 0.0);
                    self.reset();
                }
            }

            TouchAction::Up => {
                if self.state.is_active() {
                    self.state = GestureState::Ended;
                    self.emit_event(GestureState::Ended, 0.0, 0.0);
                }
                self.reset();
            }

            TouchAction::Cancel => {
                if self.state.is_active() {
                    self.state = GestureState::Cancelled;
                    self.emit_event(GestureState::Cancelled, 0.0, 0.0);
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
        self.initial_span = 0.0;
        self.initial_rotation = 0.0;
        self.last_span = 0.0;
        self.last_rotation = 0.0;
        self.current_scale = 1.0;
        self.current_rotation = 0.0;
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
