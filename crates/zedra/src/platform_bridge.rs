/// Platform abstraction layer for Android/iOS integration.
///
/// Consolidates all platform-specific calls (density, insets, keyboard, QR scanner)
/// behind a single trait. Android delegates to `android_jni`; the `StubBridge` fallback
/// lets non-Android targets compile and run `cargo check`.
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

// ---------------------------------------------------------------------------
// Native alert API
// ---------------------------------------------------------------------------

/// Style hint for a button in a native alert dialog.
#[derive(Clone, Copy, Debug)]
pub enum AlertButtonStyle {
    Default,
    Cancel,
    Destructive,
}

/// A button to display in a native alert dialog.
pub struct AlertButton {
    pub label: String,
    pub style: AlertButtonStyle,
}

impl AlertButton {
    pub fn default(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            style: AlertButtonStyle::Default,
        }
    }
    pub fn cancel(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            style: AlertButtonStyle::Cancel,
        }
    }
    pub fn destructive(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            style: AlertButtonStyle::Destructive,
        }
    }
}

static NEXT_ALERT_ID: AtomicU32 = AtomicU32::new(1);
static ALERT_CALLBACKS: OnceLock<Mutex<HashMap<u32, Box<dyn FnOnce(usize) + Send>>>> =
    OnceLock::new();

fn alert_callbacks() -> &'static Mutex<HashMap<u32, Box<dyn FnOnce(usize) + Send>>> {
    ALERT_CALLBACKS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Present a native alert dialog with the given title, message, and buttons.
///
/// `on_result` is called (off the GPUI thread) with the index of the tapped button.
/// Use a `PendingSlot` or similar if you need to update GPUI state in response.
pub fn show_alert(
    title: &str,
    message: &str,
    buttons: Vec<AlertButton>,
    on_result: impl FnOnce(usize) + Send + 'static,
) {
    let id = NEXT_ALERT_ID.fetch_add(1, Ordering::Relaxed);
    alert_callbacks()
        .lock()
        .unwrap()
        .insert(id, Box::new(on_result));
    bridge().present_alert(id, title, message, &buttons);
}

/// Called from platform code after the user taps a button.
/// Dispatches the stored callback and removes it from the registry.
pub fn dispatch_alert_result(callback_id: u32, button_index: usize) {
    let cb = alert_callbacks().lock().unwrap().remove(&callback_id);
    if let Some(cb) = cb {
        cb(button_index);
    }
}

/// Discard all pending alert callbacks without invoking them.
///
/// Call this when the app enters the background or is paused, so closures
/// captured in the callbacks (e.g. `PendingSlot` clones) are released and
/// do not accumulate over a long session.
pub fn clear_pending_alerts() {
    if let Ok(mut map) = alert_callbacks().lock() {
        let count = map.len();
        map.clear();
        if count > 0 {
            log::debug!("clear_pending_alerts: dropped {} unacknowledged alert(s)", count);
        }
    }
}

// ---------------------------------------------------------------------------
// PlatformBridge trait
// ---------------------------------------------------------------------------

pub trait PlatformBridge: Send + Sync + 'static {
    fn density(&self) -> f32;
    fn system_inset_top(&self) -> u32;
    fn system_inset_bottom(&self) -> u32;
    fn keyboard_height(&self) -> u32;
    fn is_keyboard_visible(&self) -> bool;
    fn show_keyboard(&self);
    fn hide_keyboard(&self);
    fn launch_qr_scanner(&self);
    /// Returns the app's writable data directory for persisting workspace state.
    /// On iOS: Documents directory. On Android: internal files directory.
    fn data_directory(&self) -> Option<String> {
        None
    }
    /// Display a native alert dialog.
    /// The platform implementation should present the dialog and call
    /// `platform_bridge::dispatch_alert_result(id, button_index)` when the user responds.
    fn present_alert(&self, _id: u32, _title: &str, _message: &str, _buttons: &[AlertButton]) {}
    /// Open a URL in the system browser.
    fn open_url(&self, _url: &str) {}
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
    fn is_keyboard_visible(&self) -> bool {
        false
    }
    fn show_keyboard(&self) {}
    fn hide_keyboard(&self) {}
    fn launch_qr_scanner(&self) {}
}
