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
        let host_id =
            load_or_generate_id(analytics_id_path).unwrap_or_else(|_| random_uuid());
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

    // -----------------------------------------------------------------------
    // Events
    // -----------------------------------------------------------------------

    /// Daemon started (`zedra start`).
    ///
    /// `relay_type`: "cf_worker" | "custom" | "default".
    /// `os`, `arch`, and `host_version` are injected automatically on every event.
    pub fn daemon_start(&self, relay_type: &str) {
        self.track("daemon_start", json!({
            "relay_type": relay_type,
        }));
    }

    /// STUN completed. Reports network topology of the host machine.
    /// Called once at startup after the iroh endpoint is bound.
    pub fn net_report(&self, has_ipv4: bool, has_ipv6: bool, symmetric_nat: bool) {
        self.track("net_report", json!({
            "has_ipv4": has_ipv4 as i64,
            "has_ipv6": has_ipv6 as i64,
            "symmetric_nat": symmetric_nat as i64,
        }));
    }

    /// A new device paired via QR code (Register flow, first-time only).
    pub fn client_paired(&self) {
        self.track("client_paired", json!({}));
    }

    /// Client authenticated and entered the RPC loop.
    ///
    /// `is_new_client`: true for first-ever pairing (Register), false for reconnect.
    /// `duration_ms`: wall time from inbound accept to RPC loop entry.
    /// `path_type`: "direct" | "relay" | "unknown" (iroh connection path at auth time).
    pub fn auth_success(&self, is_new_client: bool, duration_ms: u64, path_type: &str) {
        self.track("auth_success", json!({
            "is_new_client": is_new_client as i64,
            "duration_ms": duration_ms,
            "path_type": path_type,
        }));
    }

    /// Authentication was rejected before the client entered the RPC loop.
    ///
    /// `reason`: short category string, never contains personal data.
    pub fn auth_failed(&self, reason: &str) {
        let reason = &reason[..reason.len().min(50)];
        self.track("auth_failed", json!({ "reason": reason }));
    }

    /// Client disconnected. Session stays alive in the registry.
    ///
    /// `duration_ms`: how long the authenticated RPC session lasted.
    /// `terminal_count`: number of terminals that existed during the session.
    /// `path_type`: iroh connection path (captured at auth time).
    pub fn session_end(&self, duration_ms: u64, terminal_count: u64, path_type: &str) {
        self.track("session_end", json!({
            "duration_ms": duration_ms,
            "terminal_count": terminal_count,
            "path_type": path_type,
        }));
    }

    /// A new terminal PTY was spawned.
    ///
    /// `has_launch_cmd`: whether a launch command was injected (e.g. "claude --resume").
    pub fn terminal_open(&self, has_launch_cmd: bool) {
        self.track("terminal_open", json!({
            "has_launch_cmd": has_launch_cmd as i64,
        }));
    }

    /// Periodic bandwidth sample from the active iroh path.
    /// Intended to be fired every 60 seconds while a client is connected.
    pub fn bandwidth_sample(&self, bytes_sent: u64, bytes_recv: u64, interval_secs: u64) {
        self.track("bandwidth_sample", json!({
            "bytes_sent": bytes_sent,
            "bytes_recv": bytes_recv,
            "interval_secs": interval_secs,
        }));
    }

    // -----------------------------------------------------------------------
    // Internal
    // -----------------------------------------------------------------------

    fn track(&self, name: &'static str, params: Value) {
        let Some(inner) = self.inner.clone() else {
            return;
        };
        tokio::spawn(async move {
            inner.send(name, params).await;
        });
    }
}

impl Inner {
    async fn send(&self, event_name: &str, mut params: Value) {
        // Inject common fields into every event so they're filterable in GA4.
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
        if let Err(e) = self.http.post(&url).json(&body).send().await {
            tracing::debug!("analytics: send failed ({}): {}", event_name, e);
        }
    }
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
