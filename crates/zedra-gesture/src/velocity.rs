//! Velocity tracking and momentum scrolling with iOS-style deceleration

use crate::types::{Point, Vector2};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Sample for velocity calculation
#[derive(Clone, Copy, Debug)]
struct VelocitySample {
    position: Point,
    timestamp: Instant,
}

/// Tracks touch velocity over time for momentum calculations
#[derive(Clone, Debug)]
pub struct VelocityTracker {
    samples: VecDeque<VelocitySample>,
    max_samples: usize,
    sample_duration: Duration,
}

impl Default for VelocityTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl VelocityTracker {
    /// Create a new velocity tracker
    pub fn new() -> Self {
        Self {
            samples: VecDeque::with_capacity(20),
            max_samples: 20,
            // Only consider samples from the last 100ms
            sample_duration: Duration::from_millis(100),
        }
    }

    /// Add a position sample
    pub fn add_sample(&mut self, position: Point) {
        let now = Instant::now();

        // Remove old samples
        self.prune_old_samples(now);

        // Add new sample
        self.samples.push_back(VelocitySample {
            position,
            timestamp: now,
        });

        // Limit sample count
        while self.samples.len() > self.max_samples {
            self.samples.pop_front();
        }
    }

    /// Calculate current velocity in pixels per second
    pub fn velocity(&self) -> Vector2 {
        if self.samples.len() < 2 {
            return Vector2::zero();
        }

        let now = Instant::now();
        let cutoff = now - self.sample_duration;

        // Get recent samples
        let recent: Vec<_> = self
            .samples
            .iter()
            .filter(|s| s.timestamp >= cutoff)
            .collect();

        if recent.len() < 2 {
            return Vector2::zero();
        }

        // Use weighted least squares for smoother velocity
        // Weight more recent samples higher
        let mut sum_weight = 0.0f32;
        let mut sum_x = 0.0f32;
        let mut sum_y = 0.0f32;

        for i in 1..recent.len() {
            let dt = recent[i]
                .timestamp
                .duration_since(recent[i - 1].timestamp)
                .as_secs_f32();

            if dt > 0.0 {
                let dx = recent[i].position.x - recent[i - 1].position.x;
                let dy = recent[i].position.y - recent[i - 1].position.y;

                // Weight increases for more recent samples
                let weight = i as f32;
                sum_weight += weight;
                sum_x += (dx / dt) * weight;
                sum_y += (dy / dt) * weight;
            }
        }

        if sum_weight > 0.0 {
            Vector2::new(sum_x / sum_weight, sum_y / sum_weight)
        } else {
            Vector2::zero()
        }
    }

    /// Clear all samples
    pub fn clear(&mut self) {
        self.samples.clear();
    }

    fn prune_old_samples(&mut self, now: Instant) {
        let cutoff = now - self.sample_duration * 2;
        while let Some(front) = self.samples.front() {
            if front.timestamp < cutoff {
                self.samples.pop_front();
            } else {
                break;
            }
        }
    }
}

/// Deceleration curve type
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DecelerationCurve {
    /// iOS-style deceleration (default) - natural feeling scroll
    IOS,
    /// Android-style deceleration - slightly different feel
    Android,
    /// Fast deceleration - stops quickly
    Fast,
    /// Custom deceleration rate
    Custom(f32),
}

impl Default for DecelerationCurve {
    fn default() -> Self {
        DecelerationCurve::IOS
    }
}

impl DecelerationCurve {
    /// Get the deceleration rate (multiplier per frame)
    pub fn rate(&self) -> f32 {
        match self {
            DecelerationCurve::IOS => 0.998,     // iOS UIScrollView default
            DecelerationCurve::Android => 0.985, // Slightly faster decel
            DecelerationCurve::Fast => 0.95,     // Quick stop
            DecelerationCurve::Custom(rate) => *rate,
        }
    }
}

/// Momentum animation state for fling/scroll
#[derive(Clone, Debug)]
pub struct MomentumAnimator {
    /// Current velocity
    velocity: Vector2,
    /// Current position offset (from start)
    offset: Vector2,
    /// Deceleration curve
    deceleration: DecelerationCurve,
    /// Minimum velocity to continue animation (pixels/second)
    min_velocity: f32,
    /// Whether animation is active
    active: bool,
    /// Last update time
    last_update: Instant,
    /// Optional bounds for rubber-banding
    bounds: Option<MomentumBounds>,
    /// Rubber band tension (iOS-style)
    rubber_band_tension: f32,
}

/// Bounds for momentum scrolling with rubber-banding
#[derive(Clone, Copy, Debug)]
pub struct MomentumBounds {
    pub min_x: f32,
    pub max_x: f32,
    pub min_y: f32,
    pub max_y: f32,
}

impl Default for MomentumAnimator {
    fn default() -> Self {
        Self::new()
    }
}

impl MomentumAnimator {
    pub fn new() -> Self {
        Self {
            velocity: Vector2::zero(),
            offset: Vector2::zero(),
            deceleration: DecelerationCurve::IOS,
            min_velocity: 20.0, // Stop when below 20 px/s
            active: false,
            last_update: Instant::now(),
            bounds: None,
            rubber_band_tension: 0.55, // iOS default
        }
    }

    /// Set the deceleration curve
    pub fn with_deceleration(mut self, curve: DecelerationCurve) -> Self {
        self.deceleration = curve;
        self
    }

    /// Set bounds for rubber-banding
    pub fn with_bounds(mut self, bounds: MomentumBounds) -> Self {
        self.bounds = Some(bounds);
        self
    }

    /// Set rubber band tension (0.0 = no rubber band, 1.0 = full resistance)
    pub fn with_rubber_band_tension(mut self, tension: f32) -> Self {
        self.rubber_band_tension = tension.clamp(0.0, 1.0);
        self
    }

    /// Start momentum animation with initial velocity
    pub fn start(&mut self, velocity: Vector2) {
        self.velocity = velocity;
        self.offset = Vector2::zero();
        self.active = true;
        self.last_update = Instant::now();
    }

    /// Stop the animation
    pub fn stop(&mut self) {
        self.active = false;
        self.velocity = Vector2::zero();
    }

    /// Whether animation is currently active
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Current velocity
    pub fn velocity(&self) -> Vector2 {
        self.velocity
    }

    /// Total offset from start
    pub fn offset(&self) -> Vector2 {
        self.offset
    }

    /// Update animation state, returns delta offset since last update
    pub fn update(&mut self) -> Vector2 {
        if !self.active {
            return Vector2::zero();
        }

        let now = Instant::now();
        let dt = now.duration_since(self.last_update).as_secs_f32();
        self.last_update = now;

        if dt <= 0.0 {
            return Vector2::zero();
        }

        // Calculate delta position
        let delta = self.velocity * dt;

        // Update offset
        self.offset = self.offset + delta;

        // Apply deceleration
        let rate = self.deceleration.rate();
        // Deceleration is per-frame at 60fps, adjust for actual dt
        let frames = dt * 60.0;
        let decay = rate.powf(frames);
        self.velocity = self.velocity * decay;

        // Apply rubber-banding if we have bounds
        if let Some(bounds) = self.bounds {
            self.apply_rubber_band(&bounds);
        }

        // Check if we should stop
        if self.velocity.magnitude() < self.min_velocity {
            self.stop();
        }

        delta
    }

    /// Apply iOS-style rubber-band effect at bounds
    fn apply_rubber_band(&mut self, bounds: &MomentumBounds) {
        let tension = self.rubber_band_tension;

        // Check X bounds
        if self.offset.x < bounds.min_x {
            let over = bounds.min_x - self.offset.x;
            let resistance = 1.0 / (1.0 + over * 0.01 * tension);
            self.velocity.x *= resistance;
            // Pull back towards bound
            self.velocity.x += over * 0.1;
        } else if self.offset.x > bounds.max_x {
            let over = self.offset.x - bounds.max_x;
            let resistance = 1.0 / (1.0 + over * 0.01 * tension);
            self.velocity.x *= resistance;
            self.velocity.x -= over * 0.1;
        }

        // Check Y bounds
        if self.offset.y < bounds.min_y {
            let over = bounds.min_y - self.offset.y;
            let resistance = 1.0 / (1.0 + over * 0.01 * tension);
            self.velocity.y *= resistance;
            self.velocity.y += over * 0.1;
        } else if self.offset.y > bounds.max_y {
            let over = self.offset.y - bounds.max_y;
            let resistance = 1.0 / (1.0 + over * 0.01 * tension);
            self.velocity.y *= resistance;
            self.velocity.y -= over * 0.1;
        }
    }

    /// Snap back to bounds (call when touch ends outside bounds)
    pub fn snap_to_bounds(&mut self) -> Option<Vector2> {
        let bounds = self.bounds?;
        let mut target = self.offset;
        let mut needs_snap = false;

        if self.offset.x < bounds.min_x {
            target.x = bounds.min_x;
            needs_snap = true;
        } else if self.offset.x > bounds.max_x {
            target.x = bounds.max_x;
            needs_snap = true;
        }

        if self.offset.y < bounds.min_y {
            target.y = bounds.min_y;
            needs_snap = true;
        } else if self.offset.y > bounds.max_y {
            target.y = bounds.max_y;
            needs_snap = true;
        }

        if needs_snap {
            // Animate back to bounds
            let delta = Vector2::new(target.x - self.offset.x, target.y - self.offset.y);
            self.velocity = delta * 5.0; // Spring-like return
            self.active = true;
            Some(target)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    #[test]
    fn test_velocity_tracker() {
        let mut tracker = VelocityTracker::new();

        // Simulate moving right at ~100 px/s
        for i in 0..10 {
            tracker.add_sample(Point::new(i as f32 * 10.0, 0.0));
            sleep(Duration::from_millis(10));
        }

        let velocity = tracker.velocity();
        // Should be roughly 1000 px/s (10px per 10ms)
        assert!(velocity.x > 500.0, "velocity.x = {}", velocity.x);
        assert!(velocity.y.abs() < 10.0);
    }

    #[test]
    fn test_momentum_animator() {
        let mut animator = MomentumAnimator::new();
        animator.start(Vector2::new(1000.0, 0.0));

        assert!(animator.is_active());

        // Simulate a few frames
        sleep(Duration::from_millis(16));
        let delta1 = animator.update();
        assert!(delta1.x > 0.0);

        sleep(Duration::from_millis(16));
        let delta2 = animator.update();
        // Should be decelerating
        assert!(delta2.x > 0.0);
        assert!(delta2.x < delta1.x || (delta1.x - delta2.x).abs() < 1.0);
    }
}
