//! Tap and Long Press gesture recognizers

use std::time::{Duration, Instant};

use crate::state::GestureState;
use crate::types::{Point, TouchAction, TouchEvent};

use super::GestureRecognizer;

/// Configuration for tap gesture
#[derive(Clone, Debug)]
pub struct TapConfig {
    /// Maximum distance finger can move and still be a tap (in pixels)
    pub max_distance: f32,
    /// Maximum duration for a tap (in ms)
    pub max_duration: Duration,
    /// Number of taps required
    pub number_of_taps: u8,
    /// Maximum time between taps for multi-tap
    pub max_delay_between_taps: Duration,
    /// Number of fingers required
    pub number_of_fingers: usize,
}

impl Default for TapConfig {
    fn default() -> Self {
        Self {
            max_distance: 10.0,
            max_duration: Duration::from_millis(500),
            number_of_taps: 1,
            max_delay_between_taps: Duration::from_millis(300),
            number_of_fingers: 1,
        }
    }
}

/// Event emitted by tap gesture
#[derive(Clone, Debug)]
pub struct TapGestureEvent {
    pub position: Point,
    pub tap_count: u8,
    pub state: GestureState,
}

/// Tap gesture recognizer
pub struct TapGesture {
    config: TapConfig,
    state: GestureState,
    enabled: bool,

    // Internal state
    start_position: Option<Point>,
    start_time: Option<Instant>,
    tap_count: u8,
    last_tap_time: Option<Instant>,
    current_event: Option<TapGestureEvent>,
}

impl TapGesture {
    pub fn new() -> Self {
        Self::with_config(TapConfig::default())
    }

    pub fn with_config(config: TapConfig) -> Self {
        Self {
            config,
            state: GestureState::Possible,
            enabled: true,
            start_position: None,
            start_time: None,
            tap_count: 0,
            last_tap_time: None,
            current_event: None,
        }
    }

    /// Set number of taps required
    pub fn number_of_taps(mut self, count: u8) -> Self {
        self.config.number_of_taps = count;
        self
    }

    /// Set number of fingers required
    pub fn number_of_fingers(mut self, count: usize) -> Self {
        self.config.number_of_fingers = count;
        self
    }

    /// Get the current tap event if any
    pub fn event(&self) -> Option<&TapGestureEvent> {
        self.current_event.as_ref()
    }

    /// Take the current event (consumes it)
    pub fn take_event(&mut self) -> Option<TapGestureEvent> {
        self.current_event.take()
    }
}

impl Default for TapGesture {
    fn default() -> Self {
        Self::new()
    }
}

impl GestureRecognizer for TapGesture {
    fn on_touch(&mut self, event: &TouchEvent) {
        if !self.enabled {
            return;
        }

        self.current_event = None;

        match event.action {
            TouchAction::Down => {
                // Check if this is a continuation of multi-tap
                let now = Instant::now();
                if let Some(last) = self.last_tap_time {
                    if now.duration_since(last) > self.config.max_delay_between_taps {
                        self.tap_count = 0;
                    }
                }

                if event.pointer_count() == self.config.number_of_fingers {
                    self.start_position = Some(event.center());
                    self.start_time = Some(now);
                    self.state = GestureState::Possible;
                } else {
                    self.state = GestureState::Failed;
                }
            }

            TouchAction::Move => {
                if let Some(start) = self.start_position {
                    let current = event.center();
                    if start.distance_to(current) > self.config.max_distance {
                        self.state = GestureState::Failed;
                    }
                }
            }

            TouchAction::Up => {
                if self.state == GestureState::Failed {
                    self.reset();
                    return;
                }

                if let (Some(start_pos), Some(start_time)) = (self.start_position, self.start_time)
                {
                    let now = Instant::now();
                    let duration = now.duration_since(start_time);

                    // Check duration
                    if duration > self.config.max_duration {
                        self.state = GestureState::Failed;
                        self.reset();
                        return;
                    }

                    // Check distance
                    let current = event.center();
                    if start_pos.distance_to(current) > self.config.max_distance {
                        self.state = GestureState::Failed;
                        self.reset();
                        return;
                    }

                    // Valid tap!
                    self.tap_count += 1;
                    self.last_tap_time = Some(now);

                    if self.tap_count >= self.config.number_of_taps {
                        self.state = GestureState::Ended;
                        self.current_event = Some(TapGestureEvent {
                            position: current,
                            tap_count: self.tap_count,
                            state: GestureState::Ended,
                        });
                        self.tap_count = 0;
                    }
                }

                self.start_position = None;
                self.start_time = None;
            }

            TouchAction::Cancel => {
                self.state = GestureState::Cancelled;
                self.reset();
            }

            _ => {}
        }
    }

    fn state(&self) -> GestureState {
        self.state
    }

    fn reset(&mut self) {
        self.state = GestureState::Possible;
        self.start_position = None;
        self.start_time = None;
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

/// Configuration for long press gesture
#[derive(Clone, Debug)]
pub struct LongPressConfig {
    /// Minimum duration to trigger long press
    pub min_duration: Duration,
    /// Maximum distance finger can move
    pub max_distance: f32,
    /// Number of fingers required
    pub number_of_fingers: usize,
}

impl Default for LongPressConfig {
    fn default() -> Self {
        Self {
            min_duration: Duration::from_millis(500),
            max_distance: 10.0,
            number_of_fingers: 1,
        }
    }
}

/// Event emitted by long press gesture
#[derive(Clone, Debug)]
pub struct LongPressGestureEvent {
    pub position: Point,
    pub state: GestureState,
}

/// Long press gesture recognizer
pub struct LongPressGesture {
    config: LongPressConfig,
    state: GestureState,
    enabled: bool,

    start_position: Option<Point>,
    start_time: Option<Instant>,
    triggered: bool,
    current_event: Option<LongPressGestureEvent>,
}

impl LongPressGesture {
    pub fn new() -> Self {
        Self::with_config(LongPressConfig::default())
    }

    pub fn with_config(config: LongPressConfig) -> Self {
        Self {
            config,
            state: GestureState::Possible,
            enabled: true,
            start_position: None,
            start_time: None,
            triggered: false,
            current_event: None,
        }
    }

    /// Set minimum duration
    pub fn min_duration(mut self, duration: Duration) -> Self {
        self.config.min_duration = duration;
        self
    }

    /// Get the current event if any
    pub fn event(&self) -> Option<&LongPressGestureEvent> {
        self.current_event.as_ref()
    }

    /// Take the current event
    pub fn take_event(&mut self) -> Option<LongPressGestureEvent> {
        self.current_event.take()
    }

    /// Check if long press should trigger (call this on a timer)
    pub fn check_trigger(&mut self) -> bool {
        if self.triggered || self.state == GestureState::Failed {
            return false;
        }

        if let (Some(start_pos), Some(start_time)) = (self.start_position, self.start_time) {
            let now = Instant::now();
            if now.duration_since(start_time) >= self.config.min_duration {
                self.triggered = true;
                self.state = GestureState::Began;
                self.current_event = Some(LongPressGestureEvent {
                    position: start_pos,
                    state: GestureState::Began,
                });
                return true;
            }
        }
        false
    }
}

impl Default for LongPressGesture {
    fn default() -> Self {
        Self::new()
    }
}

impl GestureRecognizer for LongPressGesture {
    fn on_touch(&mut self, event: &TouchEvent) {
        if !self.enabled {
            return;
        }

        self.current_event = None;

        match event.action {
            TouchAction::Down => {
                if event.pointer_count() == self.config.number_of_fingers {
                    self.start_position = Some(event.center());
                    self.start_time = Some(Instant::now());
                    self.triggered = false;
                    self.state = GestureState::Possible;
                } else {
                    self.state = GestureState::Failed;
                }
            }

            TouchAction::Move => {
                if let Some(start) = self.start_position {
                    let current = event.center();
                    if start.distance_to(current) > self.config.max_distance {
                        self.state = GestureState::Failed;
                    }
                }
            }

            TouchAction::Up => {
                if self.triggered {
                    self.state = GestureState::Ended;
                    if let Some(pos) = self.start_position {
                        self.current_event = Some(LongPressGestureEvent {
                            position: pos,
                            state: GestureState::Ended,
                        });
                    }
                }
                self.reset();
            }

            TouchAction::Cancel => {
                if self.triggered {
                    self.state = GestureState::Cancelled;
                    if let Some(pos) = self.start_position {
                        self.current_event = Some(LongPressGestureEvent {
                            position: pos,
                            state: GestureState::Cancelled,
                        });
                    }
                }
                self.reset();
            }

            _ => {}
        }
    }

    fn state(&self) -> GestureState {
        self.state
    }

    fn reset(&mut self) {
        self.state = GestureState::Possible;
        self.start_position = None;
        self.start_time = None;
        self.triggered = false;
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
