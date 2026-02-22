/// iOS implementation of PlatformBridge.
///
/// Provides density, insets, keyboard control, and QR scanner via FFI to UIKit.
use crate::platform_bridge::PlatformBridge;

pub struct IosBridge;

unsafe extern "C" {
    fn gpui_ios_get_window() -> *mut std::ffi::c_void;
    fn gpui_ios_show_keyboard(window_ptr: *mut std::ffi::c_void);
    fn gpui_ios_hide_keyboard(window_ptr: *mut std::ffi::c_void);
}

impl PlatformBridge for IosBridge {
    fn density(&self) -> f32 {
        // iOS reports scale factor via UIScreen.main.scale
        // Default to 3.0 for modern iPhones
        3.0
    }

    fn system_inset_top(&self) -> u32 {
        // Safe area top inset (notch/Dynamic Island)
        // Could read from UIApplication.shared.windows.first?.safeAreaInsets.top
        // For now return a reasonable default for modern iPhones
        59
    }

    fn system_inset_bottom(&self) -> u32 {
        // Safe area bottom inset (home indicator)
        34
    }

    fn keyboard_height(&self) -> u32 {
        0
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
        log::warn!("QR scanner not yet implemented on iOS");
    }
}
