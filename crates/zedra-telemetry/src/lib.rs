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
#[derive(Clone, Debug)]
pub enum Event {
    // ═══════════════════════════════════════════════════════════════════════
    // APP EVENTS — mobile app + shared crates (zedra-session)
    // ═══════════════════════════════════════════════════════════════════════
    AppOpen(AppOpen),
    ScreenView(ScreenView),
    QrScanInitiated,
    ConnectSuccess(ConnectSuccess),
    ConnectFailed(ConnectFailed),
    SessionResumed(SessionResumed),
    Disconnect,
    ReconnectStarted(ReconnectStarted),
    ReconnectSuccess(ReconnectSuccess),
    ReconnectExhausted(ReconnectExhausted),
    PathUpgraded(PathUpgraded),
    TerminalOpened(TerminalOpened),
    TerminalClosed(TerminalClosed),

    // ── TCP Proxy (client-side) ────────────────────────────────────────────
    /// User opened a local-port tunnel to preview a dev server.
    TcpTunnelOpened(TcpTunnelOpened),
    /// A TCP tunnel session ended.
    TcpTunnelClosed(TcpTunnelClosed),

    // ═══════════════════════════════════════════════════════════════════════
    // HOST EVENTS — desktop daemon (zedra-host)
    // ═══════════════════════════════════════════════════════════════════════
    DaemonStart(DaemonStart),
    NetReport(NetReport),
    ClientPaired,
    AuthSuccess(AuthSuccess),
    AuthFailed(AuthFailed),
    SessionEnd(SessionEnd),
    HostTerminalOpen(HostTerminalOpen),
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
    pub screen: &'static str,
}

#[derive(Clone, Debug)]
pub struct ConnectSuccess {
    pub total_ms: u64,
    pub binding_ms: u64,
    pub hole_punch_ms: u64,
    pub auth_ms: u64,
    pub fetch_ms: u64,
    pub path: &'static str,
    pub network: &'static str,
    pub rtt_ms: u64,
    pub relay: String,
    pub relay_latency_ms: u64,
    pub alpn: String,
    pub has_ipv4: bool,
    pub has_ipv6: bool,
    pub symmetric_nat: bool,
    pub is_first_pairing: bool,
}

#[derive(Clone, Debug)]
pub struct ConnectFailed {
    pub phase: &'static str,
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
    pub network: &'static str,
    pub rtt_ms: u64,
    pub from_relay: String,
}

#[derive(Clone, Debug)]
pub struct TerminalOpened {
    pub source: &'static str,
    pub terminal_count: usize,
}

#[derive(Clone, Debug)]
pub struct TerminalClosed {
    pub remaining: usize,
}

#[derive(Clone, Debug)]
pub struct TcpTunnelOpened {
    /// Target port on the host's loopback (e.g. 3000).
    pub port: u16,
}

#[derive(Clone, Debug)]
pub struct TcpTunnelClosed {
    /// Bytes received from the host dev server and forwarded to the WebView.
    pub bytes_proxied_in: u64,
    /// Bytes received from the WebView and forwarded to the host dev server.
    pub bytes_proxied_out: u64,
    /// Wall time the tunnel was open.
    pub duration_ms: u64,
}

// ── Host event context structs ─────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct DaemonStart {
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
    pub is_new_client: bool,
    pub duration_ms: u64,
    pub path_type: &'static str,
}

#[derive(Clone, Debug)]
pub struct AuthFailed {
    pub reason: &'static str,
}

#[derive(Clone, Debug)]
pub struct SessionEnd {
    pub duration_ms: u64,
    pub terminal_count: u64,
    pub path_type: &'static str,
}

#[derive(Clone, Debug)]
pub struct HostTerminalOpen {
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
            Self::TcpTunnelOpened(_) => "tcp_tunnel_opened",
            Self::TcpTunnelClosed(_) => "tcp_tunnel_closed",
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
            Self::TcpTunnelOpened(e) => vec![("port", e.port.to_string())],
            Self::TcpTunnelClosed(e) => vec![
                ("bytes_proxied_in", e.bytes_proxied_in.to_string()),
                ("bytes_proxied_out", e.bytes_proxied_out.to_string()),
                ("duration_ms", e.duration_ms.to_string()),
            ],
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

pub trait TelemetryBackend: Send + Sync + 'static {
    fn send(&self, event: &Event);
    fn record_error(&self, _message: &str, _file: &str, _line: u32) {}
    fn record_panic(&self, _message: &str, _location: &str) {}
    fn set_user_id(&self, _id: &str) {}
    fn set_custom_key(&self, _key: &str, _value: &str) {}
    fn set_collection_enabled(&self, _enabled: bool) {}
}

// ---------------------------------------------------------------------------
// Public API — free functions
// ---------------------------------------------------------------------------

pub fn init(backend: Box<dyn TelemetryBackend>) -> Result<(), Box<dyn TelemetryBackend>> {
    BACKEND.set(backend)
}

pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
    if let Some(b) = BACKEND.get() {
        b.set_collection_enabled(enabled);
    }
}

pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn send(event: Event) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    if let Some(b) = BACKEND.get() {
        b.send(&event);
    }
}

pub fn record_error(message: &str) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    if let Some(b) = BACKEND.get() {
        b.record_error(message, "", 0);
    }
}

pub fn record_error_at(message: &str, file: &str, line: u32) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    if let Some(b) = BACKEND.get() {
        b.record_error(message, file, line);
    }
}

pub fn record_panic(message: &str, location: &str) {
    if let Some(b) = BACKEND.get() {
        b.record_panic(message, location);
    }
}

pub fn set_user_id(id: &str) {
    if let Some(b) = BACKEND.get() {
        b.set_user_id(id);
    }
}

pub fn set_custom_key(key: &str, value: &str) {
    if let Some(b) = BACKEND.get() {
        b.set_custom_key(key, value);
    }
}
