/// Platform abstraction layer for Android/iOS integration.
///
/// Consolidates all platform-specific calls (density, insets, keyboard, QR scanner)
/// behind a single trait. Android delegates to `android_jni`; the `StubBridge` fallback
/// lets non-Android targets compile and run `cargo check`.
use std::sync::OnceLock;

pub trait PlatformBridge: Send + Sync + 'static {
    fn density(&self) -> f32;
    fn system_inset_top(&self) -> u32;
    fn system_inset_bottom(&self) -> u32;
    fn keyboard_height(&self) -> u32;
    fn is_keyboard_visible(&self) -> bool;
    fn show_keyboard(&self);
    fn hide_keyboard(&self);
    fn launch_qr_scanner(&self);
}

static BRIDGE: OnceLock<Box<dyn PlatformBridge>> = OnceLock::new();

pub fn set_bridge(bridge: impl PlatformBridge) {
    let _ = BRIDGE.set(Box::new(bridge));
}

pub fn bridge() -> &'static dyn PlatformBridge {
    BRIDGE.get().map(|b| &**b).unwrap_or(&StubBridge)
}

/// Status bar top inset in logical pixels.
/// Deduplicates the `if density > 0 { inset / density } else { 0 }` pattern.
pub fn status_bar_inset() -> f32 {
    let b = bridge();
    let density = b.density();
    if density > 0.0 {
        b.system_inset_top() as f32 / density
    } else {
        0.0
    }
}

/// Home indicator / gesture bar bottom inset in logical pixels.
pub fn home_indicator_inset() -> f32 {
    let b = bridge();
    let density = b.density();
    if density > 0.0 {
        b.system_inset_bottom() as f32 / density
    } else {
        0.0
    }
}

/// Fallback bridge for non-Android platforms (and before `set_bridge` is called).
struct StubBridge;

impl PlatformBridge for StubBridge {
    fn density(&self) -> f32 {
        3.0
    }
    fn system_inset_top(&self) -> u32 {
        0
    }
    fn system_inset_bottom(&self) -> u32 {
        0
    }
    fn keyboard_height(&self) -> u32 {
        0
    }
    fn is_keyboard_visible(&self) -> bool { false }
    fn show_keyboard(&self) {}
    fn hide_keyboard(&self) {}
    fn launch_qr_scanner(&self) {}
}
