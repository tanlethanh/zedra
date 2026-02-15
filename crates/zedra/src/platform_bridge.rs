// Unified platform bridge — provides the same interface on Android and iOS.
//
// Shared UI code calls these functions instead of platform-specific modules directly.

#[cfg(target_os = "android")]
pub fn show_keyboard() {
    crate::android_jni::show_keyboard();
}

#[cfg(target_os = "android")]
pub fn hide_keyboard() {
    crate::android_jni::hide_keyboard();
}

#[cfg(target_os = "android")]
pub fn launch_qr_scanner() {
    crate::android_jni::launch_qr_scanner();
}

#[cfg(target_os = "ios")]
unsafe extern "C" {
    fn gpui_ios_get_window() -> *mut std::ffi::c_void;
    fn gpui_ios_show_keyboard(window_ptr: *mut std::ffi::c_void);
    fn gpui_ios_hide_keyboard(window_ptr: *mut std::ffi::c_void);
}

#[cfg(target_os = "ios")]
pub fn show_keyboard() {
    unsafe {
        let window = gpui_ios_get_window();
        if !window.is_null() {
            gpui_ios_show_keyboard(window);
        }
    }
}

#[cfg(target_os = "ios")]
pub fn hide_keyboard() {
    unsafe {
        let window = gpui_ios_get_window();
        if !window.is_null() {
            gpui_ios_hide_keyboard(window);
        }
    }
}

#[cfg(target_os = "ios")]
pub fn launch_qr_scanner() {
    log::warn!("QR scanner not yet implemented on iOS");
}
