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
static ALERT_CALLBACKS: OnceLock<Mutex<HashMap<u32, Box<dyn FnOnce(Option<usize>) + Send>>>> =
    OnceLock::new();
static NEXT_SELECTION_ID: AtomicU32 = AtomicU32::new(1);
static SELECTION_CALLBACKS: OnceLock<Mutex<HashMap<u32, Box<dyn FnOnce(Option<usize>) + Send>>>> =
    OnceLock::new();

fn alert_callbacks() -> &'static Mutex<HashMap<u32, Box<dyn FnOnce(Option<usize>) + Send>>> {
    ALERT_CALLBACKS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn selection_callbacks() -> &'static Mutex<HashMap<u32, Box<dyn FnOnce(Option<usize>) + Send>>> {
    SELECTION_CALLBACKS.get_or_init(|| Mutex::new(HashMap::new()))
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
    alert_callbacks().lock().unwrap().insert(
        id,
        Box::new(move |result| {
            if let Some(index) = result {
                on_result(index);
            }
        }),
    );
    bridge().present_alert(id, title, message, &buttons);
}

/// Present a native dismissible selection sheet.
///
/// `on_result` receives `Some(index)` when the user picks an item, or `None`
/// when the sheet is dismissed without a selection.
pub fn show_selection(
    title: &str,
    message: &str,
    buttons: Vec<AlertButton>,
    on_result: impl FnOnce(Option<usize>) + Send + 'static,
) {
    let id = NEXT_SELECTION_ID.fetch_add(1, Ordering::Relaxed);
    selection_callbacks()
        .lock()
        .unwrap()
        .insert(id, Box::new(on_result));
    bridge().present_selection(id, title, message, &buttons);
}

/// Called from platform code after the user taps a button.
/// Dispatches the stored callback and removes it from the registry.
pub fn dispatch_alert_result(callback_id: u32, button_index: usize) {
    let cb = alert_callbacks().lock().unwrap().remove(&callback_id);
    if let Some(cb) = cb {
        cb(Some(button_index));
    }
}

/// Called from platform code after an alert is dismissed without a button tap.
pub fn dispatch_alert_dismiss(callback_id: u32) {
    let cb = alert_callbacks().lock().unwrap().remove(&callback_id);
    if let Some(cb) = cb {
        cb(None);
    }
}

/// Called from platform code after the user picks an item from a selection sheet.
pub fn dispatch_selection_result(callback_id: u32, button_index: usize) {
    let cb = selection_callbacks().lock().unwrap().remove(&callback_id);
    if let Some(cb) = cb {
        cb(Some(button_index));
    }
}

/// Called from platform code after a selection sheet is dismissed without a choice.
pub fn dispatch_selection_dismiss(callback_id: u32) {
    let cb = selection_callbacks().lock().unwrap().remove(&callback_id);
    if let Some(cb) = cb {
        cb(None);
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
            tracing::debug!(
                "clear_pending_alerts: dropped {} unacknowledged alert(s)",
                count
            );
        }
    }
    if let Ok(mut map) = selection_callbacks().lock() {
        let count = map.len();
        map.clear();
        if count > 0 {
            tracing::debug!(
                "clear_pending_alerts: dropped {} unacknowledged selection sheet(s)",
                count
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Haptic feedback API
// ---------------------------------------------------------------------------

/// Haptic feedback patterns, mapped to native equivalents on each platform.
///
/// iOS: UIImpactFeedbackGenerator, UISelectionFeedbackGenerator, UINotificationFeedbackGenerator.
/// Android: View.performHapticFeedback with HapticFeedbackConstants (no VIBRATE permission needed).
#[derive(Clone, Copy, Debug)]
pub enum HapticFeedback {
    ImpactLight,
    ImpactMedium,
    ImpactHeavy,
    ImpactSoft,
    ImpactRigid,
    SelectionChanged,
    NotificationSuccess,
    NotificationWarning,
    NotificationError,
}

impl HapticFeedback {
    /// Stable integer encoding shared between Rust, C FFI, and JNI.
    pub fn to_i32(self) -> i32 {
        match self {
            HapticFeedback::ImpactLight => 0,
            HapticFeedback::ImpactMedium => 1,
            HapticFeedback::ImpactHeavy => 2,
            HapticFeedback::ImpactSoft => 3,
            HapticFeedback::ImpactRigid => 4,
            HapticFeedback::SelectionChanged => 5,
            HapticFeedback::NotificationSuccess => 6,
            HapticFeedback::NotificationWarning => 7,
            HapticFeedback::NotificationError => 8,
        }
    }
}

pub fn trigger_haptic(feedback: HapticFeedback) {
    bridge().trigger_haptic(feedback);
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
    /// Returns the native user-facing app version (e.g. Android versionName / iOS CFBundleShortVersionString).
    fn app_version(&self) -> Option<String> {
        None
    }
    /// Returns the native app build number (e.g. Android versionCode / iOS CFBundleVersion).
    fn app_build_number(&self) -> Option<String> {
        None
    }
    /// Returns the app's writable data directory for persisting workspace state.
    /// On iOS: Documents directory. On Android: internal files directory.
    fn data_directory(&self) -> Option<String> {
        None
    }
    /// Display a native alert dialog.
    /// The platform implementation should present the dialog and call
    /// `platform_bridge::dispatch_alert_result(id, button_index)` when the user responds.
    fn present_alert(&self, _id: u32, _title: &str, _message: &str, _buttons: &[AlertButton]) {}
    /// Display a native selection sheet.
    /// The platform implementation should call
    /// `platform_bridge::dispatch_selection_result(id, button_index)` on selection,
    /// or `platform_bridge::dispatch_selection_dismiss(id)` if dismissed.
    fn present_selection(&self, _id: u32, _title: &str, _message: &str, _buttons: &[AlertButton]) {}
    /// Open a URL in the system browser.
    fn open_url(&self, _url: &str) {}
    /// Trigger a haptic feedback pattern. No-op on platforms without haptic hardware.
    fn trigger_haptic(&self, _feedback: HapticFeedback) {}
}

static BRIDGE: OnceLock<Box<dyn PlatformBridge>> = OnceLock::new();

pub fn set_bridge(bridge: impl PlatformBridge) {
    let _ = BRIDGE.set(Box::new(bridge));
}

pub fn bridge() -> &'static dyn PlatformBridge {
    BRIDGE.get().map(|b| &**b).unwrap_or(&StubBridge)
}

/// Returns a normalized app version label as `version(buildNumber)` when both values exist.
pub fn app_version_with_build_number() -> String {
    let bridge = bridge();
    let version = bridge
        .app_version()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let build_number = bridge
        .app_build_number()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());

    match (version, build_number) {
        (Some(version), Some(build_number)) if version != build_number => {
            format!("{version}({build_number})")
        }
        (Some(version), _) => version,
        (None, Some(build_number)) => build_number,
        (None, None) => env!("CARGO_PKG_VERSION").to_string(),
    }
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
