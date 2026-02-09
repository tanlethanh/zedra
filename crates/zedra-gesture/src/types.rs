//! Core types for gesture recognition

use gpui::Pixels;
use std::time::Instant;

/// A 2D point in logical pixels
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub fn zero() -> Self {
        Self { x: 0.0, y: 0.0 }
    }

    pub fn distance_to(&self, other: Point) -> f32 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        (dx * dx + dy * dy).sqrt()
    }

    pub fn to_gpui(&self) -> gpui::Point<Pixels> {
        gpui::point(gpui::px(self.x), gpui::px(self.y))
    }
}

impl From<(f32, f32)> for Point {
    fn from((x, y): (f32, f32)) -> Self {
        Self { x, y }
    }
}

impl std::ops::Sub for Point {
    type Output = Vector2;

    fn sub(self, rhs: Point) -> Vector2 {
        Vector2 {
            x: self.x - rhs.x,
            y: self.y - rhs.y,
        }
    }
}

/// A 2D vector (for translation, velocity, etc.)
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vector2 {
    pub x: f32,
    pub y: f32,
}

impl Vector2 {
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub fn zero() -> Self {
        Self { x: 0.0, y: 0.0 }
    }

    pub fn magnitude(&self) -> f32 {
        (self.x * self.x + self.y * self.y).sqrt()
    }

    pub fn normalized(&self) -> Self {
        let mag = self.magnitude();
        if mag > 0.0 {
            Self {
                x: self.x / mag,
                y: self.y / mag,
            }
        } else {
            Self::zero()
        }
    }

    /// Angle in radians from positive X axis
    pub fn angle(&self) -> f32 {
        self.y.atan2(self.x)
    }
}

impl std::ops::Add for Vector2 {
    type Output = Vector2;

    fn add(self, rhs: Vector2) -> Vector2 {
        Vector2 {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
        }
    }
}

impl std::ops::Mul<f32> for Vector2 {
    type Output = Vector2;

    fn mul(self, scalar: f32) -> Vector2 {
        Vector2 {
            x: self.x * scalar,
            y: self.y * scalar,
        }
    }
}

/// Touch pointer information
#[derive(Clone, Copy, Debug)]
pub struct TouchPointer {
    pub id: i32,
    pub position: Point,
    pub pressure: f32,
}

impl TouchPointer {
    pub fn new(id: i32, x: f32, y: f32) -> Self {
        Self {
            id,
            position: Point::new(x, y),
            pressure: 1.0,
        }
    }
}

/// Raw touch event from the platform
#[derive(Clone, Debug)]
pub struct TouchEvent {
    pub action: TouchAction,
    pub pointers: Vec<TouchPointer>,
    pub timestamp: Instant,
}

impl TouchEvent {
    pub fn new(action: TouchAction, pointers: Vec<TouchPointer>) -> Self {
        Self {
            action,
            pointers,
            timestamp: Instant::now(),
        }
    }

    /// Get the primary (first) pointer
    pub fn primary_pointer(&self) -> Option<&TouchPointer> {
        self.pointers.first()
    }

    /// Get pointer by ID
    pub fn pointer(&self, id: i32) -> Option<&TouchPointer> {
        self.pointers.iter().find(|p| p.id == id)
    }

    /// Number of active pointers
    pub fn pointer_count(&self) -> usize {
        self.pointers.len()
    }

    /// Calculate center point of all pointers (for pinch/rotation)
    pub fn center(&self) -> Point {
        if self.pointers.is_empty() {
            return Point::zero();
        }
        let sum_x: f32 = self.pointers.iter().map(|p| p.position.x).sum();
        let sum_y: f32 = self.pointers.iter().map(|p| p.position.y).sum();
        let count = self.pointers.len() as f32;
        Point::new(sum_x / count, sum_y / count)
    }

    /// Calculate span between two pointers (for pinch)
    pub fn span(&self) -> f32 {
        if self.pointers.len() < 2 {
            return 0.0;
        }
        self.pointers[0].position.distance_to(self.pointers[1].position)
    }

    /// Calculate angle between two pointers (for rotation)
    pub fn rotation_angle(&self) -> f32 {
        if self.pointers.len() < 2 {
            return 0.0;
        }
        let p1 = self.pointers[0].position;
        let p2 = self.pointers[1].position;
        (p2.y - p1.y).atan2(p2.x - p1.x)
    }
}

/// Touch action types (maps to Android MotionEvent actions)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchAction {
    /// First finger down
    Down,
    /// Additional finger down (pointer index in action)
    PointerDown(i32),
    /// Finger moved
    Move,
    /// Last finger up
    Up,
    /// Finger up but not the last one
    PointerUp(i32),
    /// Touch cancelled (e.g., phone call, gesture intercepted)
    Cancel,
}

/// Direction for fling gestures
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlingDirection {
    Left,
    Right,
    Up,
    Down,
}

impl FlingDirection {
    pub fn from_velocity(velocity: Vector2) -> Self {
        if velocity.x.abs() > velocity.y.abs() {
            if velocity.x > 0.0 {
                FlingDirection::Right
            } else {
                FlingDirection::Left
            }
        } else {
            if velocity.y > 0.0 {
                FlingDirection::Down
            } else {
                FlingDirection::Up
            }
        }
    }
}
