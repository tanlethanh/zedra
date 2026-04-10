use std::fmt;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

use crate::ConnectEvent;

#[derive(Clone, Debug, PartialEq)]
pub enum ReconnectReason {
    ConnectionLost,
    AppForegrounded,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub enum ConnectPhase {
    #[default]
    Init,
    Idle {
        idle_since: std::time::Instant,
    },
    BindingEndpoint,
    HolePunching,
    Registering,
    Authenticating,
    Proving,
    Sync,
    Connected,
    Disconnected,
    Reconnecting {
        attempt: u32,
        reason: ReconnectReason,
        next_retry_secs: u64,
    },
    Failed(ConnectError),
}

pub const STEPPER_STEP_NAMES: [&str; 3] = ["Connect", "Auth", "Sync"];

impl ConnectPhase {
    pub fn step_index(&self) -> Option<usize> {
        match self {
            Self::BindingEndpoint | Self::HolePunching => Some(0),
            Self::Registering | Self::Authenticating | Self::Proving => Some(1),
            Self::Sync | Self::Connected => Some(2),
            _ => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Init => "init",
            Self::Idle { .. } => "idle",
            Self::BindingEndpoint => "binding_endpoint",
            Self::HolePunching => "hole_punching",
            Self::Registering => "registering",
            Self::Authenticating => "authenticating",
            Self::Proving => "proving",
            Self::Sync => "sync",
            Self::Connected => "connected",
            Self::Disconnected => "disconnected",
            Self::Reconnecting { .. } => "reconnecting",
            Self::Failed(_) => "failed",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Init => "Init",
            Self::Idle { .. } => "Idle",
            Self::BindingEndpoint => "Creating endpoint",
            Self::HolePunching => "Hole punching",
            Self::Registering => "Registering",
            Self::Authenticating => "Authenticating",
            Self::Proving => "Proving identity",
            Self::Sync => "Syncing",
            Self::Connected => "Connected",
            Self::Disconnected => "Disconnected",
            Self::Reconnecting { .. } => "Reconnecting",
            Self::Failed(_) => "Failed",
        }
    }

    pub fn is_init(&self) -> bool {
        matches!(self, Self::Init)
    }

    pub fn is_idle(&self) -> bool {
        matches!(self, Self::Idle { .. })
    }

    pub fn is_connecting(&self) -> bool {
        matches!(
            self,
            Self::BindingEndpoint
                | Self::HolePunching
                | Self::Registering
                | Self::Authenticating
                | Self::Proving
                | Self::Sync
        )
    }

    pub fn is_connected(&self) -> bool {
        matches!(self, Self::Connected)
    }

    pub fn is_reconnecting(&self) -> bool {
        matches!(self, Self::Reconnecting { .. })
    }

    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed(_))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ConnectError {
    EndpointBindFailed(String),
    QuicConnectFailed(String),
    HandshakeConsumed,
    InvalidHandshake,
    StaleTimestamp,
    SlotNotFound,
    Unauthorized,
    NotInSessionAcl,
    SessionOccupied,
    SessionNotFound,
    InvalidSignature,
    HostInvalidPubkey,
    HostSignatureInvalid,
    SessionInfoFailed(String),
    HostUnreachable,
    RequestError(String),
    Other(String),
}

impl ConnectError {
    pub fn is_fatal(&self) -> bool {
        matches!(
            self,
            Self::HandshakeConsumed
                | Self::InvalidHandshake
                | Self::StaleTimestamp
                | Self::SlotNotFound
                | Self::Unauthorized
                | Self::NotInSessionAcl
                | Self::SessionOccupied
                | Self::InvalidSignature
                | Self::HostSignatureInvalid
        )
    }

    pub fn user_message(&self) -> String {
        match self {
            Self::EndpointBindFailed(e) => format!("Failed to create network endpoint: {e}"),
            Self::QuicConnectFailed(e) => format!("Connection failed: {e}"),
            Self::HandshakeConsumed => "This QR has already been used by another device.".into(),
            Self::InvalidHandshake => "QR verification failed (HMAC mismatch).".into(),
            Self::StaleTimestamp => "Clock skew detected. Check device clock.".into(),
            Self::SlotNotFound => "QR code expired. Generate a new one on the host.".into(),
            Self::Unauthorized => "Device not authorized. Re-scan the QR code.".into(),
            Self::NotInSessionAcl => "Not authorized for this session. Re-scan QR.".into(),
            Self::SessionOccupied => "Another client is attached to this session.".into(),
            Self::SessionNotFound => "Session not found. Host may have restarted.".into(),
            Self::InvalidSignature => "Signature verification failed.".into(),
            Self::HostInvalidPubkey => "Host public key invalid.".into(),
            Self::HostSignatureInvalid => "Host identity check failed (possible MITM).".into(),
            Self::SessionInfoFailed(e) => format!("Failed to fetch session info: {e}"),
            Self::HostUnreachable => "Host unreachable after repeated attempts.".into(),
            Self::RequestError(e) => format!("RPC request failed: {e}"),
            Self::Other(e) => e.clone(),
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::EndpointBindFailed(_) => "endpoint_bind_failed",
            Self::QuicConnectFailed(_) => "quic_connect_failed",
            Self::HandshakeConsumed => "handshake_consumed",
            Self::InvalidHandshake => "invalid_handshake",
            Self::StaleTimestamp => "stale_timestamp",
            Self::SlotNotFound => "slot_not_found",
            Self::Unauthorized => "unauthorized",
            Self::NotInSessionAcl => "not_in_session_acl",
            Self::SessionOccupied => "session_occupied",
            Self::SessionNotFound => "session_not_found",
            Self::InvalidSignature => "invalid_signature",
            Self::HostInvalidPubkey => "host_invalid_pubkey",
            Self::HostSignatureInvalid => "host_signature_invalid",
            Self::SessionInfoFailed(_) => "session_info_failed",
            Self::HostUnreachable => "host_unreachable",
            Self::RequestError(_) => "request_error",
            Self::Other(_) => "other",
        }
    }

    pub fn action_hint(&self) -> Option<&'static str> {
        match self {
            Self::HandshakeConsumed
            | Self::SlotNotFound
            | Self::Unauthorized
            | Self::NotInSessionAcl => Some("Run `zedra qr` on the host to generate a new QR."),
            Self::SessionOccupied => Some("Run `zedra detach` on the host to release."),
            Self::SessionNotFound | Self::HostUnreachable => {
                Some("Run `zedra-host listen` to restart the daemon.")
            }
            _ => None,
        }
    }
}

impl fmt::Display for ConnectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.user_message())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum NetworkHint {
    Tailscale,
    Lan,
    Internet,
}

impl NetworkHint {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Tailscale => "Tailscale",
            Self::Lan => "LAN",
            Self::Internet => "Internet",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum AuthOutcome {
    Registered,
    Authenticated,
}

#[derive(Clone, Debug)]
pub struct TransportSnapshot {
    pub is_direct: bool,
    pub remote_addr: String,
    pub relay_url: Option<String>,
    pub num_paths: usize,
    pub rtt_ms: u64,
    pub bytes_sent: u64,
    pub bytes_recv: u64,
    pub path_upgraded: bool,
    pub network_hint: Option<NetworkHint>,
    pub last_alive_at: Option<std::time::Instant>,
}

#[derive(Clone, Debug, Default)]
pub struct ConnectSnapshot {
    pub binding_ms: Option<u64>,
    pub hole_punch_ms: Option<u64>,
    pub rpc_ms: Option<u64>,
    pub register_ms: Option<u64>,
    pub auth_ms: Option<u64>,
    pub fetch_ms: Option<u64>,
    pub resume_ms: Option<u64>,
    pub local_node_id: Option<String>,
    pub remote_node_id: Option<String>,
    pub relay_url: Option<String>,
    pub alpn: Option<String>,
    pub relay_connected: bool,
    pub direct_addrs: Vec<String>,
    pub has_ipv4: bool,
    pub has_ipv6: bool,
    pub mapping_varies: Option<bool>,
    pub relay_latency_ms: Option<u64>,
    pub captive_portal: Option<bool>,
    pub transport: Option<TransportSnapshot>,
    pub is_first_pairing: bool,
    pub session_id: Option<String>,
    pub auth_outcome: Option<AuthOutcome>,
    pub hostname: String,
    pub username: String,
    pub workdir: String,
    pub homedir: String,
    pub project_name: String,
    pub strip_path: String,
    pub os: Option<String>,
    pub arch: Option<String>,
    pub os_version: Option<String>,
    pub host_version: Option<String>,
    pub failed_at_step: Option<usize>,
}

// ─── SessionState ────────────────────────────────────────────────────────────

pub type StateNotifyReceiver = futures::channel::mpsc::UnboundedReceiver<()>;

#[derive(Clone)]
pub struct SessionState {
    inner: Arc<Mutex<SessionStateInner>>,
    notify_tx: Arc<Mutex<Option<futures::channel::mpsc::UnboundedSender<()>>>>,
}

#[derive(Clone, Debug, Default)]
pub struct SessionStateInner {
    pub phase: ConnectPhase,
    pub snapshot: ConnectSnapshot,
    pub started_at: Option<std::time::Instant>,
    pub reconnect_attempt: Option<u32>,
    pub phase_before_idle: Option<ConnectPhase>,
}

impl SessionStateInner {
    pub fn elapsed_secs(&self) -> u64 {
        self.started_at.map(|t| t.elapsed().as_secs()).unwrap_or(0)
    }

    pub fn elapsed_ms(&self) -> u64 {
        self.started_at
            .map(|t| t.elapsed().as_millis() as u64)
            .unwrap_or(0)
    }
}

impl Default for SessionState {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionState {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(SessionStateInner::default())),
            notify_tx: Arc::new(Mutex::new(None)),
        }
    }

    pub fn with_channel() -> (Self, mpsc::Sender<ConnectEvent>) {
        let (tx, rx) = mpsc::channel(64);
        let state = Self::new();
        state.start_listener(rx);
        (state, tx)
    }

    pub fn subscribe(&self) -> StateNotifyReceiver {
        let (tx, rx) = futures::channel::mpsc::unbounded();
        if let Ok(mut g) = self.notify_tx.lock() {
            *g = Some(tx);
        }
        rx
    }

    pub fn phase(&self) -> ConnectPhase {
        self.inner
            .lock()
            .map(|g| g.phase.clone())
            .unwrap_or_default()
    }

    pub fn get(&self) -> SessionStateInner {
        self.inner.lock().map(|g| g.clone()).unwrap_or_default()
    }

    pub fn is_connected(&self) -> bool {
        self.phase().is_connected()
    }

    pub fn start_listener(&self, mut rx: mpsc::Receiver<ConnectEvent>) {
        let state = self.clone();
        crate::session_runtime().spawn(async move {
            while let Some(event) = rx.recv().await {
                state.handle_event(event);
            }
        });
    }

    pub fn handle_event(&self, event: ConnectEvent) {
        self.apply_event(event);
        self.notify_change();
    }

    pub fn notify(&self) {
        self.notify_change();
    }

    fn notify_change(&self) {
        if let Some(tx) = self.notify_tx.lock().ok().and_then(|g| g.clone()) {
            let _ = tx.unbounded_send(());
        }
    }

    fn apply_event(&self, event: ConnectEvent) {
        let Ok(mut guard) = self.inner.lock() else {
            return;
        };
        let inner = &mut *guard;
        let snap = &mut inner.snapshot;

        match event {
            ConnectEvent::BindingEndpoint => {
                inner.phase = ConnectPhase::BindingEndpoint;
                inner.started_at = Some(std::time::Instant::now());
                reset_timing(snap);
            }
            ConnectEvent::EndpointBound {
                local_node_id,
                binding_ms,
            } => {
                snap.local_node_id = Some(local_node_id);
                snap.binding_ms = Some(binding_ms);
            }
            ConnectEvent::HolePunchStarted => {
                inner.phase = ConnectPhase::HolePunching;
            }
            ConnectEvent::HolePunchComplete {
                remote_node_id,
                alpn,
                hole_punch_ms,
            } => {
                snap.remote_node_id = Some(remote_node_id);
                snap.alpn = Some(alpn);
                snap.hole_punch_ms = Some(hole_punch_ms);
            }
            ConnectEvent::EndpointAddrChanged { endpoint_addr } => {
                snap.relay_connected = endpoint_addr.relay_urls().next().is_some();
                snap.direct_addrs = endpoint_addr.ip_addrs().map(|a| a.to_string()).collect();
            }
            ConnectEvent::NetReport { net_report } => {
                snap.has_ipv4 = net_report.udp_v4;
                snap.has_ipv6 = net_report.udp_v6;
                snap.mapping_varies = net_report.mapping_varies_by_dest();
                snap.relay_latency_ms = net_report
                    .relay_latency
                    .iter()
                    .next()
                    .map(|(_, _, lat)| lat.as_millis() as u64);
                snap.captive_portal = net_report.captive_portal;
            }
            ConnectEvent::Registering { session_id } => {
                inner.phase = ConnectPhase::Registering;
                snap.session_id = Some(session_id);
                snap.is_first_pairing = true;
            }
            ConnectEvent::RegisterComplete { register_ms } => {
                snap.register_ms = Some(register_ms);
            }
            ConnectEvent::Authenticating => {
                inner.phase = ConnectPhase::Authenticating;
            }
            ConnectEvent::Proving => {
                inner.phase = ConnectPhase::Proving;
            }
            ConnectEvent::AuthComplete {
                auth_ms,
                outcome,
                is_first_pairing,
            } => {
                snap.auth_ms = Some(auth_ms);
                snap.auth_outcome = Some(outcome);
                snap.is_first_pairing = is_first_pairing;
            }
            ConnectEvent::Syncing => {
                inner.phase = ConnectPhase::Sync;
            }
            ConnectEvent::SyncComplete { sync, fetch_ms } => {
                snap.session_id = Some(sync.session_id);
                snap.hostname = sync.hostname;
                snap.username = sync.username;
                snap.workdir = sync.workdir.clone();
                snap.homedir = sync.home_dir.clone().unwrap_or_default();
                snap.project_name = sync
                    .workdir
                    .rsplit('/')
                    .next()
                    .unwrap_or(&sync.workdir)
                    .to_string();
                let home = sync.home_dir.as_deref().unwrap_or("");
                snap.strip_path = if !home.is_empty() && sync.workdir.starts_with(home) {
                    format!("~{}", &sync.workdir[home.len()..])
                } else {
                    sync.workdir
                };
                snap.os = sync.os;
                snap.arch = sync.arch;
                snap.os_version = sync.os_version;
                snap.host_version = sync.host_version;
                snap.fetch_ms = Some(fetch_ms);
            }
            ConnectEvent::TerminalsReattached { resume_ms, .. } => {
                snap.resume_ms = Some(resume_ms);
            }
            ConnectEvent::Connected { .. } => {
                inner.phase = ConnectPhase::Connected;
                inner.reconnect_attempt = None;
            }
            ConnectEvent::PathReport {
                path,
                bytes_sent_total,
                bytes_recv_total,
            } => {
                let is_direct = path.is_ip();
                let stats = path.stats();
                let (remote_addr, relay_url, network_hint) = extract_path_info(&path);
                let prev_direct = snap
                    .transport
                    .as_ref()
                    .map(|t| t.is_direct)
                    .unwrap_or(false);
                let path_upgraded = (!prev_direct && is_direct)
                    || snap
                        .transport
                        .as_ref()
                        .map(|t| t.path_upgraded)
                        .unwrap_or(false);
                snap.transport = Some(TransportSnapshot {
                    is_direct,
                    remote_addr,
                    relay_url,
                    num_paths: 1,
                    rtt_ms: stats.rtt.as_millis() as u64,
                    bytes_sent: bytes_sent_total,
                    bytes_recv: bytes_recv_total,
                    path_upgraded,
                    network_hint,
                    last_alive_at: Some(std::time::Instant::now()),
                });
            }
            ConnectEvent::PathUpgraded { .. } => {
                if let Some(ref mut t) = snap.transport {
                    t.path_upgraded = true;
                }
            }
            ConnectEvent::NoActivePath => {
                if let Some(ref mut t) = snap.transport {
                    t.last_alive_at = None;
                }
            }
            ConnectEvent::ReconnectStarted { reason } => {
                inner.phase = ConnectPhase::Reconnecting {
                    attempt: 1,
                    reason,
                    next_retry_secs: 0,
                };
            }
            ConnectEvent::ReconnectAttempt {
                attempt,
                reason,
                next_retry_secs,
            } => {
                inner.phase = ConnectPhase::Reconnecting {
                    attempt,
                    reason,
                    next_retry_secs,
                };
                inner.reconnect_attempt = Some(attempt);
            }
            ConnectEvent::ReconnectSuccess { .. } => {
                inner.phase = ConnectPhase::Connected;
                inner.reconnect_attempt = None;
            }
            ConnectEvent::ReconnectExhausted { .. } => {
                inner.phase = ConnectPhase::Failed(ConnectError::HostUnreachable);
            }
            ConnectEvent::Failed { error } => {
                snap.failed_at_step = inner.phase.step_index();
                inner.phase = ConnectPhase::Failed(error);
            }
            ConnectEvent::ConnectionClosed => {
                inner.phase = ConnectPhase::Disconnected;
            }
            ConnectEvent::ConnectionIdle => {
                let is_idle = matches!(inner.phase, ConnectPhase::Idle { .. });
                let is_connected = matches!(inner.phase, ConnectPhase::Connected);
                if !is_idle && is_connected {
                    inner.phase_before_idle = Some(inner.phase.clone());
                    inner.phase = ConnectPhase::Idle {
                        idle_since: std::time::Instant::now(),
                    };
                }
            }
            ConnectEvent::ConnectionActive => {
                let is_idle = matches!(inner.phase, ConnectPhase::Idle { .. });
                if is_idle {
                    let prev_phase = inner
                        .phase_before_idle
                        .clone()
                        .unwrap_or(ConnectPhase::Connected);
                    inner.phase = prev_phase;
                    inner.phase_before_idle = None;
                }
            }
        }
    }
}

fn reset_timing(snap: &mut ConnectSnapshot) {
    snap.binding_ms = None;
    snap.hole_punch_ms = None;
    snap.rpc_ms = None;
    snap.register_ms = None;
    snap.auth_ms = None;
    snap.fetch_ms = None;
    snap.resume_ms = None;
}

fn extract_path_info(
    path: &iroh::endpoint::PathInfo,
) -> (String, Option<String>, Option<NetworkHint>) {
    match path.remote_addr() {
        iroh::TransportAddr::Ip(addr) => {
            let hint = classify_ip(addr.ip());
            (addr.to_string(), None, Some(hint))
        }
        iroh::TransportAddr::Relay(url) => {
            let host = url.host_str().unwrap_or(url.as_str()).to_string();
            (host.clone(), Some(host), None)
        }
        _ => (format!("{:?}", path.remote_addr()), None, None),
    }
}

fn classify_ip(ip: std::net::IpAddr) -> NetworkHint {
    match ip {
        std::net::IpAddr::V4(v4) => {
            let o = v4.octets();
            if o[0] == 100 && o[1] >= 64 && o[1] <= 127 {
                return NetworkHint::Tailscale;
            }
            if o[0] == 10
                || (o[0] == 172 && o[1] >= 16 && o[1] <= 31)
                || (o[0] == 192 && o[1] == 168)
            {
                return NetworkHint::Lan;
            }
            NetworkHint::Internet
        }
        std::net::IpAddr::V6(v6) => {
            let s = v6.segments();
            if s[0] == 0xfe80 || s[0] & 0xfe00 == 0xfc00 {
                NetworkHint::Lan
            } else {
                NetworkHint::Internet
            }
        }
    }
}
