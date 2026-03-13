/// iOS implementation of PlatformBridge.
///
/// Provides density, insets, keyboard control, and QR scanner via FFI to UIKit.
///
/// Safe area insets and screen scale are pushed from Obj-C via
/// `zedra_ios_set_safe_area_insets` / `zedra_ios_set_screen_scale` and cached
/// in atomics, mirroring the Android JNI push model.
use crate::platform_bridge::PlatformBridge;
use std::sync::atomic::{AtomicU32, Ordering};

/// Screen scale factor (e.g. 3.0 for @3x), stored as f32 bits.
/// Default 3.0 covers most modern iPhones until Obj-C pushes the real value.
static SCREEN_SCALE: AtomicU32 = AtomicU32::new(f32::to_bits(3.0));

/// Keyboard height in physical pixels. 0 = hidden.
/// Updated by UIKeyboardWillShow/WillHide notifications via Obj-C → FFI.
static KEYBOARD_HEIGHT_PX: AtomicU32 = AtomicU32::new(0);

/// Safe area insets in physical pixels (points × scale), matching the Android convention.
static SAFE_AREA_TOP: AtomicU32 = AtomicU32::new(0);
static SAFE_AREA_BOTTOM: AtomicU32 = AtomicU32::new(0);

/// Called from Obj-C whenever the screen scale is known (once, at launch).
///
/// Pass `[UIScreen mainScreen].scale`.
#[unsafe(no_mangle)]
pub extern "C" fn zedra_ios_set_screen_scale(scale: f32) {
    SCREEN_SCALE.store(scale.to_bits(), Ordering::Relaxed);
    // Sync display density to zedra-terminal for keyboard-avoiding-view row math.
    zedra_terminal::set_display_density(scale);
    log::debug!("iOS screen scale: {}", scale);
}

/// Called from Obj-C when the software keyboard is about to appear or change height.
///
/// `height_px` is `endFrame.size.height × UIScreen.scale` (physical pixels).
/// Call with 0 when the keyboard is dismissed.
#[unsafe(no_mangle)]
pub extern "C" fn zedra_ios_set_keyboard_height(height_px: u32) {
    KEYBOARD_HEIGHT_PX.store(height_px, Ordering::Relaxed);
    zedra_terminal::set_keyboard_height(height_px);
    // Signal a forced render so the terminal resizes immediately on the next
    // CADisplayLink tick rather than waiting for the next user interaction.
    zedra_session::push_callback(Box::new(|| {}));
    log::debug!("iOS keyboard height: {}px", height_px);
}

/// Called from Obj-C with the current safe area insets in physical pixels
/// (UIEdgeInsets × UIScreen.scale). Re-called on orientation change.
///
/// `left` and `right` are stored for future use (landscape support).
#[unsafe(no_mangle)]
pub extern "C" fn zedra_ios_set_safe_area_insets(top: f32, bottom: f32, _left: f32, _right: f32) {
    SAFE_AREA_TOP.store(top as u32, Ordering::Relaxed);
    SAFE_AREA_BOTTOM.store(bottom as u32, Ordering::Relaxed);
    log::debug!(
        "iOS safe area insets: top={}px bottom={}px",
        top as u32,
        bottom as u32
    );
}

pub struct IosBridge;

unsafe extern "C" {
    fn gpui_ios_get_window() -> *mut std::ffi::c_void;
    fn gpui_ios_is_keyboard_visible(window_ptr: *mut std::ffi::c_void) -> bool;
    fn gpui_ios_show_keyboard(window_ptr: *mut std::ffi::c_void);
    fn gpui_ios_hide_keyboard(window_ptr: *mut std::ffi::c_void);
    /// Present the AVFoundation QR scanner (defined in ZedraQRScanner.m).
    fn ios_present_qr_scanner();
    /// Returns the app's Documents directory path (from NSSearchPathForDirectoriesInDomains).
    fn ios_get_documents_directory() -> *const std::ffi::c_char;
    /// Present a native UIAlertController with dynamic buttons.
    /// `labels` and `styles` are parallel arrays of length `button_count`.
    /// Style values: 0 = default, 1 = cancel, 2 = destructive.
    /// Result delivered via `zedra_ios_alert_result(callback_id, button_index)`.
    fn ios_present_alert(
        callback_id: u32,
        title: *const std::ffi::c_char,
        message: *const std::ffi::c_char,
        button_count: i32,
        labels: *const *const std::ffi::c_char,
        styles: *const i32,
    );
}

impl PlatformBridge for IosBridge {
    fn density(&self) -> f32 {
        f32::from_bits(SCREEN_SCALE.load(Ordering::Relaxed))
    }

    fn system_inset_top(&self) -> u32 {
        SAFE_AREA_TOP.load(Ordering::Relaxed)
    }

    fn system_inset_bottom(&self) -> u32 {
        SAFE_AREA_BOTTOM.load(Ordering::Relaxed)
    }

    fn keyboard_height(&self) -> u32 {
        KEYBOARD_HEIGHT_PX.load(Ordering::Relaxed)
    }

    fn is_keyboard_visible(&self) -> bool {
        unsafe {
            let window = gpui_ios_get_window();
            if window.is_null() {
                return false;
            }
            gpui_ios_is_keyboard_visible(window)
        }
    }

    fn show_keyboard(&self) {
        unsafe {
            let window = gpui_ios_get_window();
            if !window.is_null() {
                gpui_ios_show_keyboard(window);
            }
        }
    }

    fn hide_keyboard(&self) {
        unsafe {
            let window = gpui_ios_get_window();
            if !window.is_null() {
                gpui_ios_hide_keyboard(window);
            }
        }
    }

    fn launch_qr_scanner(&self) {
        unsafe { ios_present_qr_scanner() };
    }

    fn data_directory(&self) -> Option<String> {
        unsafe {
            let ptr = ios_get_documents_directory();
            if ptr.is_null() {
                return None;
            }
            let cstr = std::ffi::CStr::from_ptr(ptr);
            let s = cstr.to_str().ok()?.to_string();
            Some(s)
        }
    }

    fn present_alert(
        &self,
        id: u32,
        title: &str,
        message: &str,
        buttons: &[crate::platform_bridge::AlertButton],
    ) {
        use crate::platform_bridge::AlertButtonStyle;
        use std::ffi::CString;

        let c_title =
            CString::new(title).unwrap_or_else(|_| CString::new("").unwrap());
        let c_message =
            CString::new(message).unwrap_or_else(|_| CString::new("").unwrap());
        // Build CString labels and collect raw pointers (kept alive by the Vec).
        let c_labels: Vec<CString> = buttons
            .iter()
            .map(|b| CString::new(b.label.as_str()).unwrap_or_else(|_| CString::new("OK").unwrap()))
            .collect();
        let label_ptrs: Vec<*const std::ffi::c_char> =
            c_labels.iter().map(|s| s.as_ptr()).collect();
        let styles: Vec<i32> = buttons
            .iter()
            .map(|b| match b.style {
                AlertButtonStyle::Default => 0,
                AlertButtonStyle::Cancel => 1,
                AlertButtonStyle::Destructive => 2,
            })
            .collect();
        unsafe {
            ios_present_alert(
                id,
                c_title.as_ptr(),
                c_message.as_ptr(),
                buttons.len() as i32,
                label_ptrs.as_ptr(),
                styles.as_ptr(),
            );
        }
    }
}

/// Called from the UIAlertController handler in main.m after the user taps a button.
///
/// `callback_id` matches the value passed to `ios_present_alert`.
/// `button_index` is the 0-based index of the tapped button (matches the `buttons` array
/// passed to `platform_bridge::show_alert`).
#[unsafe(no_mangle)]
pub extern "C" fn zedra_ios_alert_result(callback_id: u32, button_index: i32) {
    if button_index >= 0 {
        crate::platform_bridge::dispatch_alert_result(callback_id, button_index as usize);
        zedra_session::push_callback(Box::new(|| {}));
    }
}

/// Called from the native keyboard accessory bar when a shortcut key button is tapped.
///
/// `key` is one of: "escape", "tab", "left", "down", "up", "right", "enter".
/// Maps the name to the corresponding terminal escape sequence and sends it via the active session.
#[unsafe(no_mangle)]
pub extern "C" fn zedra_ios_send_key_input(key: *const std::ffi::c_char) {
    if key.is_null() {
        return;
    }
    let key_name = unsafe {
        match std::ffi::CStr::from_ptr(key).to_str() {
            Ok(s) => s,
            Err(_) => return,
        }
    };
    if key_name == "dismiss_keyboard" {
        crate::platform_bridge::bridge().hide_keyboard();
        zedra_session::push_callback(Box::new(|| {}));
        return;
    }

    let bytes: &[u8] = match key_name {
        "escape" => b"\x1b",
        "tab"   => b"\x09",
        "left"  => b"\x1b[D",
        "down"  => b"\x1b[B",
        "up"    => b"\x1b[A",
        "right" => b"\x1b[C",
        "enter" => b"\r",
        _ => return,
    };
    crate::active_terminal::send_to_active(bytes.to_vec());
    zedra_session::push_callback(Box::new(|| {}));
}

/// Called from main.m when the app is opened via a zedra:// URL.
#[unsafe(no_mangle)]
pub extern "C" fn zedra_deeplink_received(url: *const std::ffi::c_char) {
    if url.is_null() {
        return;
    }
    let s = unsafe { std::ffi::CStr::from_ptr(url) };
    match s.to_str() {
        Ok(v) => match crate::deeplink::parse(v) {
            Ok(action) => crate::deeplink::enqueue(action),
            Err(e) => log::error!("Invalid deeplink URL: {}", e),
        },
        Err(e) => log::error!("Deeplink: invalid UTF-8: {}", e),
    }
}

/// Called from ZedraQRScanner.m after a successful QR scan.
///
/// Routes through the unified deeplink path (same as system URL intents).
#[unsafe(no_mangle)]
pub extern "C" fn zedra_qr_scanner_result(qr_string: *const std::ffi::c_char) {
    if qr_string.is_null() {
        return;
    }
    let s = unsafe { std::ffi::CStr::from_ptr(qr_string) };
    match s.to_str() {
        Ok(v) => match crate::deeplink::parse(v) {
            Ok(action) => crate::deeplink::enqueue(action),
            Err(e) => log::error!("QR scan: invalid deeplink: {}", e),
        },
        Err(e) => log::error!("QR result: invalid UTF-8: {}", e),
    }
}
