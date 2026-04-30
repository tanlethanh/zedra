// Registers the host's GA4 backend as a zedra_telemetry backend.
//
// This allows shared crates (zedra-session, zedra-rpc) to call
// `zedra_telemetry::send(Event::...)` and have events flow through the
// host's GA4 Measurement Protocol pipeline.

use std::sync::Arc;

use crate::ga4::Ga4;

fn params_value(event: &zedra_telemetry::Event) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    for (k, v) in event.to_params() {
        obj.insert(k.to_string(), serde_json::Value::String(v));
    }
    serde_json::Value::Object(obj)
}

/// Wraps the host's `Ga4` transport as a `TelemetryBackend`.
struct HostBackend {
    ga4: Arc<Ga4>,
}

impl zedra_telemetry::TelemetryBackend for HostBackend {
    fn send(&self, event: &zedra_telemetry::Event) {
        let name = event.name();
        self.ga4.track_raw(name, params_value(event));
    }

    fn record_panic(&self, message: &str, location: &str) {
        self.ga4.host_panic_sync(message, location);
    }
}

/// Register the host's GA4 backend as the global telemetry backend.
pub fn init(ga4: Arc<Ga4>) {
    let _ = zedra_telemetry::init(Box::new(HostBackend { ga4 }));
}

/// Send and flush one event for short-lived CLI commands.
pub async fn send_now(ga4: &Ga4, event: zedra_telemetry::Event) {
    ga4.track_raw_now(event.name(), params_value(&event)).await;
}
