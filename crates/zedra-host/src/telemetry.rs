// Registers the host's GA4 Analytics as a zedra_telemetry backend.
//
// This allows shared crates (zedra-session, zedra-rpc) to call
// `zedra_telemetry::send(Event::...)` and have events flow through the
// host's GA4 Measurement Protocol pipeline.

use std::sync::Arc;

use crate::analytics::Analytics;

/// Wraps the host's GA4 `Analytics` as a `TelemetryBackend`.
struct HostBackend {
    analytics: Arc<Analytics>,
}

impl zedra_telemetry::TelemetryBackend for HostBackend {
    fn send(&self, event: &zedra_telemetry::Event) {
        let name = event.name();
        let params = event.to_params();
        let mut obj = serde_json::Map::new();
        for (k, v) in params {
            obj.insert(k.to_string(), serde_json::Value::String(v));
        }
        self.analytics
            .track_raw(name, serde_json::Value::Object(obj));
    }

    fn record_panic(&self, message: &str, location: &str) {
        self.analytics.host_panic_sync(message, location);
    }
}

/// Register the host's GA4 analytics as the global telemetry backend.
pub fn init(analytics: Arc<Analytics>) {
    let _ = zedra_telemetry::init(Box::new(HostBackend { analytics }));
}
