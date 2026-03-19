/// Android implementation of `PlatformBridge`.
///
/// Delegates every call to the corresponding function in `super::jni`.
use crate::android::jni;
use crate::platform_bridge::{AlertButton, PlatformBridge};

pub struct AndroidBridge;

impl PlatformBridge for AndroidBridge {
    fn density(&self) -> f32 {
        jni::get_density()
    }

    fn system_inset_top(&self) -> u32 {
        jni::get_system_inset_top()
    }

    fn system_inset_bottom(&self) -> u32 {
        jni::get_system_inset_bottom()
    }

    fn keyboard_height(&self) -> u32 {
        jni::get_keyboard_height()
    }

    fn is_keyboard_visible(&self) -> bool {
        // Android tracks keyboard visibility via keyboard_height > 0
        jni::get_keyboard_height() > 0
    }

    fn show_keyboard(&self) {
        jni::show_keyboard()
    }

    fn hide_keyboard(&self) {
        jni::hide_keyboard()
    }

    fn launch_qr_scanner(&self) {
        jni::launch_qr_scanner()
    }

    fn data_directory(&self) -> Option<String> {
        jni::get_files_dir()
    }

    fn open_url(&self, url: &str) {
        jni::open_url(url);
    }

    fn present_alert(&self, id: u32, title: &str, message: &str, buttons: &[AlertButton]) {
        jni::show_alert(id, title, message, buttons);
    }

    fn present_selection(&self, id: u32, title: &str, message: &str, buttons: &[AlertButton]) {
        jni::show_selection(id, title, message, buttons);
    }
}
