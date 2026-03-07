/// Android implementation of `PlatformBridge`.
///
/// Delegates every call to the corresponding function in `super::jni`.

use crate::platform_bridge::PlatformBridge;

pub struct AndroidBridge;

impl PlatformBridge for AndroidBridge {
    fn density(&self) -> f32 {
        crate::android::jni::get_density()
    }

    fn system_inset_top(&self) -> u32 {
        crate::android::jni::get_system_inset_top()
    }

    fn system_inset_bottom(&self) -> u32 {
        crate::android::jni::get_system_inset_bottom()
    }

    fn keyboard_height(&self) -> u32 {
        crate::android::jni::get_keyboard_height()
    }

    fn is_keyboard_visible(&self) -> bool {
        // Android tracks keyboard visibility via keyboard_height > 0
        crate::android::jni::get_keyboard_height() > 0
    }

    fn show_keyboard(&self) {
        crate::android::jni::show_keyboard()
    }

    fn hide_keyboard(&self) {
        crate::android::jni::hide_keyboard()
    }

    fn launch_qr_scanner(&self) {
        crate::android::jni::launch_qr_scanner()
    }

    fn data_directory(&self) -> Option<String> {
        crate::android::jni::get_files_dir()
    }
}
