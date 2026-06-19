/// Android implementation of `PlatformBridge`.
///
/// Delegates every call to the corresponding function in `super::jni`.
use crate::android::jni;
use crate::platform_bridge::{
    AlertButton, CustomSheetOptions, HapticFeedback, ListPickerItem, NativeDictationPreviewOptions,
    NativeEditMenuItem, NativeFloatingButtonOptions, NativeNotificationOptions, PlatformBridge,
    SoundEffect, SystemTheme,
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

    fn os_version(&self) -> Option<String> {
        let version = jni::get_os_version();
        if version.trim().is_empty() {
            None
        } else {
            Some(version)
        }
    }

    fn data_directory(&self) -> Option<String> {
        jni::get_files_dir()
    }

    fn device_name(&self) -> Option<String> {
        let name = jni::get_delta_device_name();
        if name.trim().is_empty() {
            None
        } else {
            Some(name)
        }
    }

    fn open_url(&self, url: &str) {
        jni::open_url(url);
    }

    fn open_webview(&self, url: &str, title: &str, proxy_url: Option<&str>) {
        jni::open_webview(url, title, proxy_url);
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

    fn present_native_edit_menu(
        &self,
        id: u32,
        position: gpui::Point<gpui::Pixels>,
        items: &[NativeEditMenuItem],
    ) {
        jni::show_native_edit_menu(id, position, items);
    }

    fn present_custom_sheet(&self, options: &CustomSheetOptions) {
        jni::present_custom_sheet(options);
    }

    fn dismiss_custom_sheet(&self) {
        jni::dismiss_custom_sheet();
    }

    fn trigger_haptic(&self, feedback: HapticFeedback) {
        jni::trigger_haptic(feedback);
    }

    fn play_sound(&self, sound: SoundEffect) {
        jni::play_sound(sound);
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

    fn request_delta_push_token(&self, id: u32) {
        jni::request_delta_push_token(id);
    }

    fn start_delta_google_sign_in(&self, id: u32) {
        jni::start_delta_google_sign_in(id);
    }
}
