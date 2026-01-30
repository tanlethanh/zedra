// Shared Zedra app (used by both Android and iOS)
pub mod zedra_app;

// GPUI Android JNI bridge
#[cfg(target_os = "android")]
pub mod android_jni;

// Android app bridge
#[cfg(target_os = "android")]
pub mod android_app;

// Android command queue for threading
#[cfg(target_os = "android")]
pub mod android_command_queue;

// Legacy JNI stubs (called by Java but no longer used)
#[cfg(target_os = "android")]
mod legacy_jni {
    use jni::{JNIEnv, objects::JClass};

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_dev_zedra_app_MainActivity_rustOnResume(_: JNIEnv, _: JClass) {}

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_dev_zedra_app_MainActivity_rustOnPause(_: JNIEnv, _: JClass) {}
}
