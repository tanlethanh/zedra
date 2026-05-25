/// Android implementation of `PlatformBridge`.
///
/// Delegates every call to the corresponding function in `super::jni`.
use crate::android::jni;
use crate::platform_bridge::{
    AlertButton, CustomSheetOptions, HapticFeedback, ListPickerItem, NativeDictationPreviewOptions,
    NativeFloatingButtonOptions, NativeNotificationOptions, PlatformBridge, SystemTheme,
};

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

    fn launch_qr_scanner(&self) {
        jni::launch_qr_scanner()
    }

    fn app_version(&self) -> Option<String> {
        let version = jni::get_app_version();
        if version.trim().is_empty() {
            None
        } else {
            Some(version)
        }
    }

    fn app_build_number(&self) -> Option<String> {
        let build_number = jni::get_app_build_number();
        if build_number.trim().is_empty() {
            None
        } else {
            Some(build_number)
        }
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

    fn present_list_picker(&self, id: u32, title: &str, message: &str, items: &[ListPickerItem]) {
        jni::show_list_picker(id, title, message, items);
    }

    fn present_custom_sheet(&self, options: &CustomSheetOptions) {
        jni::present_custom_sheet(options);
    }

    fn trigger_haptic(&self, feedback: HapticFeedback) {
        jni::trigger_haptic(feedback);
    }

    fn update_native_floating_button(&self, id: u32, options: &NativeFloatingButtonOptions) {
        jni::update_native_floating_button(id, options);
    }

    fn hide_native_floating_button(&self, id: u32) {
        jni::hide_native_floating_button(id);
    }

    fn update_native_dictation_preview(&self, id: u32, options: &NativeDictationPreviewOptions) {
        jni::update_native_dictation_preview(id, options);
    }

    fn hide_native_dictation_preview(&self, id: u32) {
        jni::hide_native_dictation_preview(id);
    }

    fn present_native_notification(&self, id: u32, options: &NativeNotificationOptions) {
        jni::present_native_notification(id, options);
    }

    fn present_text_input(&self, id: u32, title: &str, placeholder: &str, initial_value: &str) {
        jni::show_text_input(id, title, placeholder, initial_value);
    }

    fn system_prefers_theme(&self) -> SystemTheme {
        jni::system_prefers_theme()
    }

    fn set_native_theme(&self, is_dark: bool) {
        jni::set_native_theme(is_dark);
    }
}
