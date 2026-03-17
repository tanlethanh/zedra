// Shared analytics API — platform-agnostic call sites.
//
// Dispatches to the platform bridge at compile time via cfg.
// On the host daemon (non-mobile) all calls are no-ops so the crate compiles
// without any Firebase dependency.
//
// Usage:
//   crate::analytics::log_event("screen_view", &[("screen_name", "home")]);
//   crate::analytics::record_error("connection refused");
//   crate::analytics::set_user_id(&session_id);

#[cfg(target_os = "android")]
use crate::android::analytics as android_analytics;
#[cfg(target_os = "ios")]
use crate::ios::analytics as ios_analytics;

/// Log a named event with optional key-value parameters.
///
/// Event names must be ≤ 40 characters (Firebase Analytics limit).
/// Parameter keys must be ≤ 24 characters; values ≤ 100 characters.
pub fn log_event(name: &str, params: &[(&str, &str)]) {
    #[cfg(target_os = "android")]
    android_analytics::log_event(name, params);
    #[cfg(target_os = "ios")]
    ios_analytics::log_event(name, params);
    let _ = (name, params);
}

/// Record a non-fatal error visible in Crashlytics > Non-fatals.
///
/// Use this for recoverable errors at system boundaries (transport failures,
/// RPC errors, parse failures) that are worth tracking without crashing.
pub fn record_error(message: &str) {
    #[cfg(target_os = "android")]
    android_analytics::record_error(message, "", 0);
    #[cfg(target_os = "ios")]
    ios_analytics::record_error(message, "", 0);
    let _ = message;
}

/// Record a Rust panic as a non-fatal Crashlytics event.
///
/// Called from install_panic_hook() in lib.rs.  In release builds
/// (panic = "abort") this is never reached — the Crashlytics NDK / iOS
/// crash handler captures the abort signal directly as a fatal crash.
pub fn record_panic(message: &str, location: &str) {
    #[cfg(target_os = "android")]
    android_analytics::record_panic(message, location);
    #[cfg(target_os = "ios")]
    ios_analytics::record_panic(message, location);
    let _ = (message, location);
}

/// Associate subsequent events and crashes with a session or user identity.
///
/// Call this once a session is established (e.g. after QR pairing succeeds).
/// Pass an empty string to clear the association.
pub fn set_user_id(id: &str) {
    #[cfg(target_os = "android")]
    android_analytics::set_user_id(id);
    #[cfg(target_os = "ios")]
    ios_analytics::set_user_id(id);
    let _ = id;
}
