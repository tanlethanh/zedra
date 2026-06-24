// No-op telemetry backend, selected when the `telemetry` feature is disabled.
//
// Building with the `no-telemetry` feature swaps `ga4.rs` for this stub, so no
// GA4 credentials (`option_env!`), HTTP client, or send paths are compiled in.
// The public surface mirrors `ga4::Ga4` so `main.rs` and `telemetry.rs` need no
// `cfg` gating; every method is inert.

use serde_json::Value;

pub struct Ga4 {
    /// Always false: with telemetry compiled out there is no first-run signal
    /// and no telemetry-id file is touched.
    pub is_first_run: bool,
}

impl Ga4 {
    pub fn new(_telemetry_id_path: &std::path::Path, _debug: bool) -> Self {
        Self {
            is_first_run: false,
        }
    }

    pub fn disabled() -> Self {
        Self {
            is_first_run: false,
        }
    }

    pub fn host_panic_sync(&self, _message: &str, _location: &str) {}

    pub(crate) fn track_raw(&self, _name: &'static str, _params: Value) {}

    pub(crate) async fn track_raw_now(&self, _name: &'static str, _params: Value) {}
}
