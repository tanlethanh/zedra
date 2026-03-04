/// Android platform integration: JNI bridge, command queue, GPUI app, and platform bridge.

pub mod jni;
pub mod app;
pub mod command_queue;
pub(crate) mod bridge;

// Legacy JNI stubs (kept for ABI compatibility with existing Java code)
mod legacy_jni {
    use ::jni::{JNIEnv, objects::JClass};

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_dev_zedra_app_MainActivity_rustOnResume(_: JNIEnv, _: JClass) {}

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_dev_zedra_app_MainActivity_rustOnPause(_: JNIEnv, _: JClass) {}
}
