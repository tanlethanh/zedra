// zedra-telemetry: typed telemetry events with runtime dependency injection.
//
// Each runtime (app, host, client) calls `init()` once at startup to register
// its concrete backend (Firebase, GA4, no-op). All other crates call `send()`
// with a typed `Event` variant that carries relevant context.
//
// If no backend is registered, all calls are silent no-ops.
//
// Privacy: no personal data (usernames, file paths, IP addresses) is ever
// included in events. Only opaque IDs, durations, counts, and enum labels.

use std::sync::{
    OnceLock,
    atomic::{AtomicBool, Ordering},
};

static BACKEND: OnceLock<Box<dyn TelemetryBackend>> = OnceLock::new();
static ENABLED: AtomicBool = AtomicBool::new(true);

// ---------------------------------------------------------------------------
// Event enum — every telemetry event is a typed variant
// ---------------------------------------------------------------------------

/// All telemetry events. Each variant carries the context relevant to that event.
///
/// Events are grouped by origin:
/// - **App events** — emitted by the mobile app (iOS/Android) and shared crates (zedra-session).
/// - **Host events** — emitted by the desktop daemon (zedra-host).
///
/// When adding a new feature, define a new variant here with a dedicated struct
/// that includes timing, counts, path/transport info, and version fields as
/// appropriate. Never include personal data (usernames, file contents, IPs).
#[derive(Clone, Debug)]
pub enum Event {
    // ═══════════════════════════════════════════════════════════════════════
    // APP EVENTS — mobile app + shared crates (zedra-session)
    // ═══════════════════════════════════════════════════════════════════════

    // ── App lifecycle ──────────────────────────────────────────────────────
    /// App launched (cold start).
    AppOpen(AppOpen),
    /// User navigated to a new screen.
    ScreenView(ScreenView),

    // ── QR / pairing ──────────────────────────────────────────────────────
    /// User tapped the "Scan QR" button.
    QrScanInitiated,

    // ── Connection (client-side) ───────────────────────────────────────────
    /// Connection established successfully.
    ConnectSuccess(ConnectSuccess),
    /// Connection attempt failed.
    ConnectFailed(ConnectFailed),
    /// Session resumed with existing server-side terminals.
    SessionResumed(SessionResumed),
    /// User-initiated disconnect.
    Disconnect,

    // ── Reconnect (client-side) ────────────────────────────────────────────
    /// Auto-reconnect loop started.
    ReconnectStarted(ReconnectStarted),
    /// Reconnect succeeded after N attempts.
    ReconnectSuccess(ReconnectSuccess),
    /// All reconnect attempts exhausted.
    ReconnectExhausted(ReconnectExhausted),

    // ── Transport (client-side) ────────────────────────────────────────────
    /// Connection path upgraded from relay to direct P2P.
    PathUpgraded(PathUpgraded),

    // ── Terminal (client-side) ─────────────────────────────────────────────
    /// A new terminal was created (PTY spawned on server).
    TerminalOpened(TerminalOpened),
    /// A terminal was closed by the user.
    TerminalClosed(TerminalClosed),

    // ═══════════════════════════════════════════════════════════════════════
    // HOST EVENTS — desktop daemon (zedra-host)
    // ═══════════════════════════════════════════════════════════════════════

    // ── Daemon lifecycle ───────────────────────────────────────────────────
    /// Daemon started (`zedra start`).
    DaemonStart(DaemonStart),
    /// STUN/network report completed at startup.
    NetReport(NetReport),

    // ── Auth (server-side) ─────────────────────────────────────────────────
    /// New device paired via QR code (first-time Register flow).
    ClientPaired,
    /// Client authenticated and entered the RPC loop.
    AuthSuccess(AuthSuccess),
    /// Authentication rejected.
    AuthFailed(AuthFailed),

    // ── Session (server-side) ──────────────────────────────────────────────
    /// Client disconnected; session stays alive in registry.
    SessionEnd(SessionEnd),

    // ── Terminal (server-side) ─────────────────────────────────────────────
    /// A new terminal PTY was spawned on the host.
    HostTerminalOpen(HostTerminalOpen),

    // ── Monitoring (server-side) ───────────────────────────────────────────
    /// Periodic bandwidth sample from the active iroh path.
    BandwidthSample(BandwidthSample),
}

// ---------------------------------------------------------------------------
// Event context structs
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct AppOpen {
    pub saved_workspaces: usize,
    pub app_version: &'static str,
    pub platform: &'static str,
    pub arch: &'static str,
}

#[derive(Clone, Debug)]
pub struct ScreenView {
    /// e.g. "home", "workspace"
    pub screen: &'static str,
}

#[derive(Clone, Debug)]
pub struct ConnectSuccess {
    // Phase timings (ms)
    pub total_ms: u64,
    pub binding_ms: u64,
    pub hole_punch_ms: u64,
    pub auth_ms: u64,
    pub fetch_ms: u64,
    // Transport context
    /// "direct" or "relay"
    pub path: &'static str,
    /// Network classification: "LAN", "Tailscale", "Internet", "unknown"
    pub network: &'static str,
    pub rtt_ms: u64,
    /// Preferred relay URL (e.g. "sg1.relay.zedra.dev") or "none"
    pub relay: String,
    pub relay_latency_ms: u64,
    /// ALPN protocol (e.g. "zedra/rpc/3")
    pub alpn: String,
    // Discovery
    pub has_ipv4: bool,
    pub has_ipv6: bool,
    pub symmetric_nat: bool,
    pub is_first_pairing: bool,
}

#[derive(Clone, Debug)]
pub struct ConnectFailed {
    /// Phase label where failure occurred (e.g. "hole_punching", "authenticating")
    pub phase: &'static str,
    /// Error label (e.g. "quic_connect_failed", "host_signature_invalid")
    pub error: &'static str,
    pub relay: String,
    pub alpn: String,
    pub has_ipv4: bool,
    pub has_ipv6: bool,
    pub relay_connected: bool,
}

#[derive(Clone, Debug)]
pub struct SessionResumed {
    pub terminal_count: usize,
    pub resume_ms: u64,
}

#[derive(Clone, Debug)]
pub struct ReconnectStarted {
    /// "connection_lost" or "app_foregrounded"
    pub reason: &'static str,
}

#[derive(Clone, Debug)]
pub struct ReconnectSuccess {
    pub attempt: u32,
    pub elapsed_ms: u64,
    pub reason: &'static str,
}

#[derive(Clone, Debug)]
pub struct ReconnectExhausted {
    pub attempts: u32,
    pub elapsed_ms: u64,
    pub reason: &'static str,
}

#[derive(Clone, Debug)]
pub struct PathUpgraded {
    /// Network classification of the new direct path.
    pub network: &'static str,
    pub rtt_ms: u64,
    /// Relay URL the connection upgraded from.
    pub from_relay: String,
}

#[derive(Clone, Debug)]
pub struct TerminalOpened {
    /// "new_session", "user_action"
    pub source: &'static str,
    /// Total terminal count after opening.
    pub terminal_count: usize,
}

#[derive(Clone, Debug)]
pub struct TerminalClosed {
    /// Remaining terminal count after closing.
    pub remaining: usize,
}

// ── Host event context structs ─────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct DaemonStart {
    /// "custom" or "default"
    pub relay_type: &'static str,
}

#[derive(Clone, Debug)]
pub struct NetReport {
    pub has_ipv4: bool,
    pub has_ipv6: bool,
    pub symmetric_nat: bool,
}

#[derive(Clone, Debug)]
pub struct AuthSuccess {
    /// true for first-ever pairing (Register), false for reconnect.
    pub is_new_client: bool,
    /// Wall time from inbound accept to RPC loop entry.
    pub duration_ms: u64,
    /// "direct", "relay", or "unknown"
    pub path_type: &'static str,
}

#[derive(Clone, Debug)]
pub struct AuthFailed {
    /// Short category string (e.g. "auth_error", "bad_hmac").
    pub reason: &'static str,
}

#[derive(Clone, Debug)]
pub struct SessionEnd {
    /// How long the authenticated RPC session lasted.
    pub duration_ms: u64,
    /// Number of terminals that existed during the session.
    pub terminal_count: u64,
    /// "direct", "relay", or "unknown"
    pub path_type: &'static str,
}

#[derive(Clone, Debug)]
pub struct HostTerminalOpen {
    /// Whether a launch command was injected (e.g. "claude --resume").
    pub has_launch_cmd: bool,
}

#[derive(Clone, Debug)]
pub struct BandwidthSample {
    pub bytes_sent: u64,
    pub bytes_recv: u64,
    pub interval_secs: u64,
}

// ---------------------------------------------------------------------------
// Event → (name, params) serialization
// ---------------------------------------------------------------------------

impl Event {
    /// Event name for the analytics backend (Firebase event name / GA4 event name).
    pub fn name(&self) -> &'static str {
        match self {
            Self::AppOpen(_) => "app_open",
            Self::ScreenView(_) => "screen_view",
            Self::QrScanInitiated => "qr_scan_initiated",
            Self::ConnectSuccess(_) => "connect_success",
            Self::ConnectFailed(_) => "connect_failed",
            Self::SessionResumed(_) => "session_resumed",
            Self::Disconnect => "disconnect",
            Self::ReconnectStarted(_) => "reconnect_started",
            Self::ReconnectSuccess(_) => "reconnect_success",
            Self::ReconnectExhausted(_) => "reconnect_exhausted",
            Self::PathUpgraded(_) => "path_upgraded",
            Self::TerminalOpened(_) => "terminal_opened",
            Self::TerminalClosed(_) => "terminal_closed",
            // Host events
            Self::DaemonStart(_) => "daemon_start",
            Self::NetReport(_) => "net_report",
            Self::ClientPaired => "client_paired",
            Self::AuthSuccess(_) => "auth_success",
            Self::AuthFailed(_) => "auth_failed",
            Self::SessionEnd(_) => "session_end",
            Self::HostTerminalOpen(_) => "terminal_open",
            Self::BandwidthSample(_) => "bandwidth_sample",
        }
    }

    /// Serialize event fields as key-value string pairs for platform backends.
    /// Values are borrowed from the event — caller must not outlive the event.
    pub fn to_params(&self) -> Vec<(&str, String)> {
        match self {
            Self::AppOpen(e) => vec![
                ("saved_workspaces", e.saved_workspaces.to_string()),
                ("app_version", e.app_version.to_string()),
                ("platform", e.platform.to_string()),
                ("arch", e.arch.to_string()),
            ],
            Self::ScreenView(e) => vec![("screen", e.screen.to_string())],
            Self::QrScanInitiated | Self::Disconnect | Self::ClientPaired => vec![],
            Self::ConnectSuccess(e) => vec![
                ("total_ms", e.total_ms.to_string()),
                ("binding_ms", e.binding_ms.to_string()),
                ("hole_punch_ms", e.hole_punch_ms.to_string()),
                ("auth_ms", e.auth_ms.to_string()),
                ("fetch_ms", e.fetch_ms.to_string()),
                ("path", e.path.to_string()),
                ("network", e.network.to_string()),
                ("rtt_ms", e.rtt_ms.to_string()),
                ("relay", e.relay.clone()),
                ("relay_latency_ms", e.relay_latency_ms.to_string()),
                ("alpn", e.alpn.clone()),
                ("has_ipv4", bool_str(e.has_ipv4)),
                ("has_ipv6", bool_str(e.has_ipv6)),
                ("symmetric_nat", bool_str(e.symmetric_nat)),
                ("is_first_pairing", bool_str(e.is_first_pairing)),
            ],
            Self::ConnectFailed(e) => vec![
                ("phase", e.phase.to_string()),
                ("error", e.error.to_string()),
                ("relay", e.relay.clone()),
                ("alpn", e.alpn.clone()),
                ("has_ipv4", bool_str(e.has_ipv4)),
                ("has_ipv6", bool_str(e.has_ipv6)),
                ("relay_connected", bool_str(e.relay_connected)),
            ],
            Self::SessionResumed(e) => vec![
                ("terminal_count", e.terminal_count.to_string()),
                ("resume_ms", e.resume_ms.to_string()),
            ],
            Self::ReconnectStarted(e) => vec![("reason", e.reason.to_string())],
            Self::ReconnectSuccess(e) => vec![
                ("attempt", e.attempt.to_string()),
                ("elapsed_ms", e.elapsed_ms.to_string()),
                ("reason", e.reason.to_string()),
            ],
            Self::ReconnectExhausted(e) => vec![
                ("attempts", e.attempts.to_string()),
                ("elapsed_ms", e.elapsed_ms.to_string()),
                ("reason", e.reason.to_string()),
            ],
            Self::PathUpgraded(e) => vec![
                ("network", e.network.to_string()),
                ("rtt_ms", e.rtt_ms.to_string()),
                ("from_relay", e.from_relay.clone()),
            ],
            Self::TerminalOpened(e) => vec![
                ("source", e.source.to_string()),
                ("terminal_count", e.terminal_count.to_string()),
            ],
            Self::TerminalClosed(e) => vec![("remaining", e.remaining.to_string())],
            // Host events
            Self::DaemonStart(e) => vec![("relay_type", e.relay_type.to_string())],
            Self::NetReport(e) => vec![
                ("has_ipv4", bool_str(e.has_ipv4)),
                ("has_ipv6", bool_str(e.has_ipv6)),
                ("symmetric_nat", bool_str(e.symmetric_nat)),
            ],
            Self::AuthSuccess(e) => vec![
                ("is_new_client", bool_str(e.is_new_client)),
                ("duration_ms", e.duration_ms.to_string()),
                ("path_type", e.path_type.to_string()),
            ],
            Self::AuthFailed(e) => vec![("reason", e.reason.to_string())],
            Self::SessionEnd(e) => vec![
                ("duration_ms", e.duration_ms.to_string()),
                ("terminal_count", e.terminal_count.to_string()),
                ("path_type", e.path_type.to_string()),
            ],
            Self::HostTerminalOpen(e) => {
                vec![("has_launch_cmd", bool_str(e.has_launch_cmd))]
            }
            Self::BandwidthSample(e) => vec![
                ("bytes_sent", e.bytes_sent.to_string()),
                ("bytes_recv", e.bytes_recv.to_string()),
                ("interval_secs", e.interval_secs.to_string()),
            ],
        }
    }
}

fn bool_str(v: bool) -> String {
    if v { "1" } else { "0" }.to_string()
}

// ---------------------------------------------------------------------------
// Backend trait
// ---------------------------------------------------------------------------

/// Trait implemented by each runtime's telemetry backend.
///
/// **Non-blocking contract**: `send()` MUST NOT block the calling thread.
/// Implementations must either queue work internally (Firebase SDK does this)
/// or spawn it onto a background executor (e.g. `tokio::spawn`). This
/// ensures telemetry never delays app logic, rendering, or RPC handling.
///
/// Crash-related methods (`record_error`, `record_panic`) are separate
/// because they have different gating (panics always sent) and platform APIs.
/// `record_panic` is the one exception — it MAY block briefly to ensure the
/// event is flushed before the process aborts.
pub trait TelemetryBackend: Send + Sync + 'static {
    /// Send a typed telemetry event to the backend.
    /// **Must be non-blocking** — queue or spawn, never do synchronous I/O.
    fn send(&self, event: &Event);

    /// Record a non-fatal error (Crashlytics / GA4 non-fatal).
    fn record_error(&self, _message: &str, _file: &str, _line: u32) {}

    /// Record a panic. Always sent regardless of enabled/disabled state.
    fn record_panic(&self, _message: &str, _location: &str) {}

    /// Associate events/crashes with a user or session identity.
    fn set_user_id(&self, _id: &str) {}

    /// Set a custom key-value pair for crash reports.
    fn set_custom_key(&self, _key: &str, _value: &str) {}

    /// Enable or disable collection at the platform SDK level.
    fn set_collection_enabled(&self, _enabled: bool) {}
}

// ---------------------------------------------------------------------------
// Public API — free functions
// ---------------------------------------------------------------------------

/// Register the telemetry backend. Call once at startup.
/// Returns `Err` with the backend if already initialized.
pub fn init(backend: Box<dyn TelemetryBackend>) -> Result<(), Box<dyn TelemetryBackend>> {
    BACKEND.set(backend)
}

/// Enable or disable telemetry at runtime.
/// Also calls through to the backend's platform-level toggle.
pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
    if let Some(b) = BACKEND.get() {
        b.set_collection_enabled(enabled);
    }
}

/// Returns true if telemetry is currently enabled.
pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Send a typed telemetry event.
/// No-op if telemetry is disabled or no backend registered.
pub fn send(event: Event) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    if let Some(b) = BACKEND.get() {
        b.send(&event);
    }
}

/// Record a non-fatal error.
/// No-op if telemetry is disabled or no backend registered.
pub fn record_error(message: &str) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    if let Some(b) = BACKEND.get() {
        b.record_error(message, "", 0);
    }
}

/// Record a non-fatal error with file/line context.
/// No-op if telemetry is disabled or no backend registered.
pub fn record_error_at(message: &str, file: &str, line: u32) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    if let Some(b) = BACKEND.get() {
        b.record_error(message, file, line);
    }
}

/// Record a panic. Always sent regardless of enabled/disabled state.
pub fn record_panic(message: &str, location: &str) {
    if let Some(b) = BACKEND.get() {
        b.record_panic(message, location);
    }
}

/// Associate subsequent events and crashes with a session/user identity.
pub fn set_user_id(id: &str) {
    if let Some(b) = BACKEND.get() {
        b.set_user_id(id);
    }
}

/// Set a custom key-value pair for crash reports.
pub fn set_custom_key(key: &str, value: &str) {
    if let Some(b) = BACKEND.get() {
        b.set_custom_key(key, value);
    }
}
