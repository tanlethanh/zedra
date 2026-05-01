// GA4 Measurement Protocol backend — sends telemetry events via GA4.
//
// Credentials are compiled in at build time via environment variables:
//   ZEDRA_GA_MEASUREMENT_ID   e.g. "G-XXXXXXXXXX"
//   ZEDRA_GA_API_SECRET       from Firebase console → Data Streams → Measurement Protocol
//
// If either variable is absent or empty the module is silently disabled —
// binaries built from source without credentials send no data.
//
// Privacy: no personal data is collected. The telemetry_id is a random UUID
// stored at ~/.config/zedra/telemetry_id, separate from the cryptographic
// host identity and never transmitted alongside it.

use serde_json::{json, Value};
use std::sync::Arc;

const GA_ENDPOINT: &str = "https://www.google-analytics.com/mp/collect";
const GA_DEBUG_ENDPOINT: &str = "https://www.google-analytics.com/debug/mp/collect";

const GA_MEASUREMENT_ID: Option<&str> = option_env!("ZEDRA_GA_MEASUREMENT_ID");
const GA_API_SECRET: Option<&str> = option_env!("ZEDRA_GA_API_SECRET");

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

pub struct Ga4 {
    inner: Option<Arc<Inner>>,
    /// True if the telemetry_id file did not exist before this run (first ever start).
    pub is_first_run: bool,
}

struct Inner {
    /// Pre-built GA4 URL (includes measurement_id + api_secret query params).
    url: String,
    /// Stable, opaque, machine-level ID (random UUID, persisted to disk).
    host_id: String,
    http: reqwest::Client,
    /// Common fields appended to every event.
    host_version: &'static str,
    os: &'static str,
    arch: &'static str,
    /// When true, use the GA4 validation endpoint and log request/response.
    debug: bool,
}

impl Ga4 {
    /// Build from compile-time credentials.
    ///
    /// `telemetry_id_path` points to `~/.config/zedra/telemetry_id`.
    /// A random UUID is generated on first run and reused on subsequent runs.
    /// Returns a no-op instance if credentials were not compiled in.
    ///
    /// When `debug` is true the GA4 validation endpoint is used and every
    /// request/response is printed to stderr. Events are NOT recorded in GA4.
    pub fn new(telemetry_id_path: &std::path::Path, debug: bool) -> Self {
        // Detect first run before load_or_generate_id creates the file.
        let is_first_run = !telemetry_id_path.exists();
        let (Some(mid), Some(secret)) = (GA_MEASUREMENT_ID, GA_API_SECRET) else {
            return Self {
                inner: None,
                is_first_run,
            };
        };
        if mid.is_empty() || secret.is_empty() {
            return Self {
                inner: None,
                is_first_run,
            };
        }
        let host_id = load_or_generate_id(telemetry_id_path).unwrap_or_else(|_| random_uuid());
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap_or_default();
        let endpoint = if debug {
            GA_DEBUG_ENDPOINT
        } else {
            GA_ENDPOINT
        };
        let url = format!("{}?measurement_id={}&api_secret={}", endpoint, mid, secret);
        Self {
            inner: Some(Arc::new(Inner {
                url,
                host_id,
                http,
                host_version: env!("CARGO_PKG_VERSION"),
                os: std::env::consts::OS,
                arch: std::env::consts::ARCH,
                debug,
            })),
            is_first_run,
        }
    }

    /// No-op instance (telemetry opted out or credentials absent).
    pub fn disabled() -> Self {
        Self {
            inner: None,
            is_first_run: false,
        }
    }

    /// Record a host panic. Uses **synchronous** HTTP so the event is sent
    /// before the process aborts. Call from a panic hook only.
    ///
    /// `message`: first 100 chars of the panic payload (paths stripped).
    /// `location`: file:line of the panic site.
    pub fn host_panic_sync(&self, message: &str, location: &str) {
        let Some(inner) = &self.inner else { return };
        let message = sanitize_panic_message(message);
        let location = sanitize_panic_message(location);
        let params = json!({
            "message": &message[..message.len().min(100)],
            "location": &location[..location.len().min(100)],
        });
        inner.send_sync("host_panic", params);
    }

    // -----------------------------------------------------------------------
    // Internal
    // -----------------------------------------------------------------------

    /// Spawns a background task to send an event, satisfying the non-blocking
    /// contract of `TelemetryBackend::send()`. Called by `telemetry::HostBackend`.
    pub(crate) fn track_raw(&self, name: &'static str, params: Value) {
        let Some(inner) = self.inner.clone() else {
            return;
        };
        tokio::spawn(async move {
            inner.send(name, params).await;
        });
    }

    /// Send an event and wait for the HTTP request to finish.
    ///
    /// Short-lived CLI commands use this so completion telemetry is not dropped
    /// when the Tokio runtime shuts down immediately after the command exits.
    pub(crate) async fn track_raw_now(&self, name: &'static str, params: Value) {
        let Some(inner) = self.inner.clone() else {
            return;
        };
        inner.send(name, params).await;
    }
}

impl Inner {
    fn build_payload(&self, event_name: &str, mut params: Value) -> Value {
        if let Some(obj) = params.as_object_mut() {
            obj.insert("host_version".into(), self.host_version.into());
            obj.insert("os".into(), self.os.into());
            obj.insert("arch".into(), self.arch.into());
        }
        json!({
            "client_id": self.host_id,
            "events": [{ "name": event_name, "params": params }],
        })
    }

    async fn send(&self, event_name: &str, params: Value) {
        let body = self.build_payload(event_name, params);
        if self.debug {
            eprintln!("[telemetry] >> {event_name} {body}");
        }
        match self.http.post(&self.url).json(&body).send().await {
            Ok(resp) if self.debug => {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                eprintln!("[telemetry] << {event_name} HTTP {status}: {text}");
            }
            Ok(_) => {}
            Err(e) => tracing::debug!("telemetry: send failed ({}): {}", event_name, e),
        }
    }

    /// Blocking send for use in panic hooks where the tokio runtime may be gone.
    fn send_sync(&self, event_name: &str, params: Value) {
        let body = self.build_payload(event_name, params);
        if self.debug {
            eprintln!("[telemetry] >> {event_name} {body}");
        }
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(3))
            .build()
            .unwrap_or_default();
        if self.debug {
            if let Ok(resp) = client.post(&self.url).json(&body).send() {
                let status = resp.status();
                let text = resp.text().unwrap_or_default();
                eprintln!("[telemetry] << {event_name} HTTP {status}: {text}");
            }
        } else {
            let _ = client.post(&self.url).json(&body).send();
        }
    }
}

/// Strip filesystem paths from panic messages to avoid leaking usernames/dirs.
/// Replaces `/Users/foo/...` or `/home/foo/...` style paths with `<path>`.
fn sanitize_panic_message(msg: &str) -> String {
    let mut result = String::with_capacity(msg.len());
    for token in msg.split_whitespace() {
        if !result.is_empty() {
            result.push(' ');
        }
        // Using a broad match to avoid false negatives leaking usernames.
        let t = token.trim_start_matches('"').trim_start_matches('\'');
        if t.starts_with('/')
            || t.starts_with('~')
            || t.contains("/src/")
            || t.contains('\\')
            || (t.len() >= 3 && t.as_bytes()[1] == b':' && t.as_bytes()[2] == b'\\')
        {
            result.push_str("<path>");
        } else {
            result.push_str(token);
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Telemetry ID (stable, opaque, machine-level random UUID)
// ---------------------------------------------------------------------------

fn load_or_generate_id(path: &std::path::Path) -> anyhow::Result<String> {
    if path.exists() {
        let id = std::fs::read_to_string(path)?;
        let id = id.trim().to_string();
        if id.len() >= 32 {
            return Ok(id);
        }
    }
    let id = random_uuid();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, &id)?;
    Ok(id)
}

fn random_uuid() -> String {
    let b: [u8; 16] = rand::random();
    format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:04x}{:08x}",
        u32::from_be_bytes([b[0], b[1], b[2], b[3]]),
        u16::from_be_bytes([b[4], b[5]]),
        u16::from_be_bytes([b[6], b[7]]),
        u16::from_be_bytes([b[8], b[9]]),
        u16::from_be_bytes([b[10], b[11]]),
        u32::from_be_bytes([b[12], b[13], b[14], b[15]]),
    )
}
