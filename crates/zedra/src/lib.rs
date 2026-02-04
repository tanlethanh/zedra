// Zedra Android application - GPUI on Android via Blade/Vulkan

// Shared Zedra app (connection UI + terminal)
pub mod zedra_app;

// File explorer view
pub mod file_explorer;

// File preview card grid
pub mod file_preview_list;

// GPUI Android JNI bridge
pub mod android_jni;

// Android app bridge
pub mod android_app;

// Android command queue for threading
pub mod android_command_queue;

// Legacy JNI stubs (called by Java but no longer used)
mod legacy_jni {
    use jni::{JNIEnv, objects::JClass};

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_dev_zedra_app_MainActivity_rustOnResume(_: JNIEnv, _: JClass) {}

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_dev_zedra_app_MainActivity_rustOnPause(_: JNIEnv, _: JClass) {}
}
