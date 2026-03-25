// App telemetry — registers the platform Firebase backend with zedra-telemetry.
//
// Call `init()` once at app startup (before any events fire).
// After init, all crates can use `zedra_telemetry::send(Event::...)` etc.

#[cfg(target_os = "ios")]
use crate::ios::telemetry as ios_telemetry;

/// Platform-specific Firebase backend that implements TelemetryBackend.
struct FirebaseBackend;

impl zedra_telemetry::TelemetryBackend for FirebaseBackend {
    fn send(&self, event: &zedra_telemetry::Event) {
        let name = event.name();
        let params = event.to_params();
        #[cfg(feature = "debug-telemetry")]
        {
            let kv: Vec<String> = params.iter().map(|(k, v)| format!("{}={}", k, v)).collect();
            eprintln!("[telemetry] >> {} {}", name, kv.join(" "));
        }
        let param_refs: Vec<(&str, &str)> = params.iter().map(|(k, v)| (*k, v.as_str())).collect();
        #[cfg(target_os = "ios")]
        ios_telemetry::log_event(name, &param_refs);
        // Android Firebase not yet implemented; events are no-ops on Android.
        let _ = (name, param_refs);
    }

    fn record_error(&self, message: &str, file: &str, line: u32) {
        #[cfg(target_os = "ios")]
        ios_telemetry::record_error(message, file, line);
        let _ = (message, file, line);
    }

    fn record_panic(&self, message: &str, location: &str) {
        #[cfg(feature = "debug-telemetry")]
        eprintln!("[telemetry] panic: {} at {}", message, location);
        #[cfg(target_os = "ios")]
        ios_telemetry::record_panic(message, location);
        let _ = (message, location);
    }

    fn set_user_id(&self, id: &str) {
        #[cfg(target_os = "ios")]
        ios_telemetry::set_user_id(id);
        let _ = id;
    }

    fn set_custom_key(&self, key: &str, value: &str) {
        #[cfg(target_os = "ios")]
        ios_telemetry::set_custom_key(key, value);
        let _ = (key, value);
    }

    fn set_collection_enabled(&self, enabled: bool) {
        #[cfg(target_os = "ios")]
        ios_telemetry::set_collection_enabled(enabled);
        let _ = enabled;
    }
}

/// Register the Firebase backend with the shared telemetry crate.
/// Call once at app startup before any events fire.
pub fn init() {
    let _ = zedra_telemetry::init(Box::new(FirebaseBackend));
}

// Re-export for convenience so existing call sites don't need to change imports.
pub use zedra_telemetry::{
    is_enabled, record_error, record_panic, set_custom_key, set_enabled, set_user_id,
};
