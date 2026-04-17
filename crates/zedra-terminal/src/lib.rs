pub mod element;
pub mod input;
pub mod keys;
pub mod terminal;
pub mod view;

pub use element::*;
pub use input::*;
pub use keys::*;
pub use terminal::*;
pub use view::*;

use std::sync::atomic::{AtomicU32, Ordering};

// Global keyboard height in physical pixels, set by the JNI layer.
// 0 means the keyboard is hidden.
static KEYBOARD_HEIGHT_PX: AtomicU32 = AtomicU32::new(0);

// Global display density (scale factor × 100, stored as integer).
// Default 300 = 3.0× scale. Set by the JNI layer.
static DISPLAY_DENSITY_X100: AtomicU32 = AtomicU32::new(300);

/// Set the current soft keyboard height (physical pixels). Called from JNI layer.
pub fn set_keyboard_height(px: u32) {
    KEYBOARD_HEIGHT_PX.store(px, Ordering::Relaxed);
}

/// Get the current soft keyboard height in physical pixels (0 = hidden).
pub fn get_keyboard_height() -> u32 {
    KEYBOARD_HEIGHT_PX.load(Ordering::Relaxed)
}

/// Set the display density (scale factor). Called from JNI layer.
pub fn set_display_density(density: f32) {
    DISPLAY_DENSITY_X100.store((density * 100.0) as u32, Ordering::Relaxed);
}

/// Get the display density (scale factor).
pub fn get_display_density() -> f32 {
    DISPLAY_DENSITY_X100.load(Ordering::Relaxed) as f32 / 100.0
}

/// The font family name for the embedded terminal font.
/// The font bytes and loader live in the `zedra` crate (`fonts` module).
pub const MONO_FONT_FAMILY: &str = "JetBrainsMonoNL Nerd Font Mono";
