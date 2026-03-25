// Analytics: send product events to GA4 via the Measurement Protocol.
//
// Credentials are compiled in at build time via environment variables:
//   ZEDRA_GA_MEASUREMENT_ID   e.g. "G-XXXXXXXXXX"
//   ZEDRA_GA_API_SECRET       from Firebase console → Data Streams → Measurement Protocol
//
// If either variable is absent or empty the module is silently disabled —
// binaries built from source without credentials send no data.
//
// Privacy: no personal data is collected. The analytics_id is a random UUID
// stored at ~/.config/zedra/analytics_id, separate from the cryptographic
// host identity and never transmitted alongside it.

use serde_json::{json, Value};
use std::sync::Arc;

const GA_ENDPOINT: &str = "https://www.google-analytics.com/mp/collect";

const GA_MEASUREMENT_ID: Option<&str> = option_env!("ZEDRA_GA_MEASUREMENT_ID");
const GA_API_SECRET: Option<&str> = option_env!("ZEDRA_GA_API_SECRET");

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

pub struct Analytics {
    inner: Option<Arc<Inner>>,
}

struct Inner {
    measurement_id: String,
    api_secret: String,
    /// Stable, opaque, machine-level ID (random UUID, persisted to disk).
    host_id: String,
    http: reqwest::Client,
    /// Common fields appended to every event.
    host_version: &'static str,
    os: &'static str,
    arch: &'static str,
}

impl Analytics {
    /// Build from compile-time credentials.
    ///
    /// `analytics_id_path` points to `~/.config/zedra/analytics_id`.
    /// A random UUID is generated on first run and reused on subsequent runs.
    /// Returns a no-op instance if credentials were not compiled in.
    pub fn new(analytics_id_path: &std::path::Path) -> Self {
        let (Some(mid), Some(secret)) = (GA_MEASUREMENT_ID, GA_API_SECRET) else {
            return Self { inner: None };
        };
        if mid.is_empty() || secret.is_empty() {
            return Self { inner: None };
        }
        let host_id = load_or_generate_id(analytics_id_path).unwrap_or_else(|_| random_uuid());
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap_or_default();
        Self {
            inner: Some(Arc::new(Inner {
                measurement_id: mid.to_string(),
                api_secret: secret.to_string(),
                host_id,
                http,
                host_version: env!("CARGO_PKG_VERSION"),
                os: std::env::consts::OS,
                arch: std::env::consts::ARCH,
            })),
        }
    }

    /// No-op instance used when credentials are absent.
    pub fn disabled() -> Self {
        Self { inner: None }
    }

    /// Returns true if analytics is active (credentials were compiled in).
    pub fn is_enabled(&self) -> bool {
        self.inner.is_some()
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

    /// Send an event with pre-built params. Used by the telemetry bridge.
    pub fn track_raw(&self, name: &str, params: Value) {
        let Some(inner) = self.inner.clone() else {
            return;
        };
        let name = name.to_string();
        tokio::spawn(async move {
            inner.send(&name, params).await;
        });
    }
}

impl Inner {
    fn build_payload(&self, event_name: &str, mut params: Value) -> (String, Value) {
        if let Some(obj) = params.as_object_mut() {
            obj.insert("host_version".into(), self.host_version.into());
            obj.insert("os".into(), self.os.into());
            obj.insert("arch".into(), self.arch.into());
        }
        let body = json!({
            "client_id": self.host_id,
            "events": [{ "name": event_name, "params": params }],
        });
        let url = format!(
            "{}?measurement_id={}&api_secret={}",
            GA_ENDPOINT, self.measurement_id, self.api_secret,
        );
        (url, body)
    }

    async fn send(&self, event_name: &str, params: Value) {
        let (url, body) = self.build_payload(event_name, params);
        if let Err(e) = self.http.post(&url).json(&body).send().await {
            tracing::debug!("analytics: send failed ({}): {}", event_name, e);
        }
    }

    /// Blocking send for use in panic hooks where the tokio runtime may be gone.
    fn send_sync(&self, event_name: &str, params: Value) {
        let (url, body) = self.build_payload(event_name, params);
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(3))
            .build()
            .unwrap_or_default();
        let _ = client.post(&url).json(&body).send();
    }
}

/// Strip filesystem paths from panic messages to avoid leaking usernames/dirs.
/// Replaces `/Users/foo/...` or `/home/foo/...` style paths with `<path>`.
fn sanitize_panic_message(msg: &str) -> String {
    // Simple heuristic: replace any token that looks like an absolute path.
    let mut result = String::with_capacity(msg.len());
    for token in msg.split_whitespace() {
        if !result.is_empty() {
            result.push(' ');
        }
        // Strip tokens that look like absolute/relative filesystem paths.
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
// Analytics ID (stable, opaque, machine-level random UUID)
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
