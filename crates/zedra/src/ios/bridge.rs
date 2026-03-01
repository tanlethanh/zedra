/// iOS implementation of PlatformBridge.
///
/// Provides density, insets, keyboard control, and QR scanner via FFI to UIKit.
use crate::platform_bridge::PlatformBridge;

pub struct IosBridge;

unsafe extern "C" {
    fn gpui_ios_get_window() -> *mut std::ffi::c_void;
    fn gpui_ios_show_keyboard(window_ptr: *mut std::ffi::c_void);
    fn gpui_ios_hide_keyboard(window_ptr: *mut std::ffi::c_void);
    /// Present the AVFoundation QR scanner (defined in ZedraQRScanner.m).
    fn ios_present_qr_scanner();
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
        unsafe { ios_present_qr_scanner() };
    }
}

/// Called from ZedraQRScanner.m after a successful QR scan.
///
/// `qr_string` is a base64-url encoded iroh::EndpointAddr produced by
/// `zedra_rpc::pairing::encode_endpoint_addr()` on the host side.
#[unsafe(no_mangle)]
pub extern "C" fn zedra_qr_scanner_result(qr_string: *const std::ffi::c_char) {
    if qr_string.is_null() {
        return;
    }
    let s = unsafe { std::ffi::CStr::from_ptr(qr_string) };
    let s = match s.to_str() {
        Ok(v) => v,
        Err(e) => {
            log::error!("QR result: invalid UTF-8: {}", e);
            return;
        }
    };
    match zedra_rpc::pairing::decode_endpoint_addr(s) {
        Ok(addr) => {
            log::info!("QR scan: decoded EndpointAddr successfully");
            crate::app::set_pending_qr_addr(addr);
        }
        Err(e) => {
            log::error!("QR scan: failed to decode EndpointAddr: {}", e);
        }
    }
}
