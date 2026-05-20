pub(crate) mod bridge;
pub mod entry;
/// App-specific JNI exports + Rust→Java helpers (alerts, sheets, presentations,
/// QR scanner, deeplinks). Framework-level JNI lives in `gpui_android`.
pub mod jni;
pub mod sheet;
pub mod telemetry;

// Legacy JNI stubs (kept for ABI compatibility with existing Java code).
mod legacy_jni {
    use ::jni::{JNIEnv, objects::JClass};

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_dev_zedra_app_MainActivity_rustOnResume(_: JNIEnv, _: JClass) {}

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_dev_zedra_app_MainActivity_rustOnPause(_: JNIEnv, _: JClass) {}
}
