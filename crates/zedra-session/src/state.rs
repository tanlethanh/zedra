use std::fmt;

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
    AlpnMismatch,
    ConnectionClosed,
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
                | Self::AlpnMismatch
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
            Self::AlpnMismatch => "Protocol mismatch, Update App or CLI".into(),
            Self::ConnectionClosed => "Connection closed. Tap refresh to reconnect.".into(),
            Self::HandshakeConsumed => "The QR code was used. Refresh it and scan again.".into(),
            Self::InvalidHandshake => "QR verification failed (HMAC mismatch).".into(),
            Self::StaleTimestamp => "Clock skew detected. Check device clock.".into(),
            Self::SlotNotFound => "QR code expired. Generate a new one on the host.".into(),
            Self::Unauthorized => "Device not authorized. Re-scan the QR code.".into(),
            Self::NotInSessionAcl => "Not authorized for this session. Re-scan QR.".into(),
            Self::SessionOccupied => "Host occupied. Disconnect other device and retry.".into(),
            Self::SessionNotFound => "Session not found. Host may have restarted.".into(),
            Self::InvalidSignature => "Signature verification failed.".into(),
            Self::HostInvalidPubkey => "Host public key invalid.".into(),
            Self::HostSignatureInvalid => "Host identity check failed (possible MITM).".into(),
            Self::SessionInfoFailed(e) => format!("Failed to fetch session info: {e}"),
            Self::HostUnreachable => "Host unreachable. Check network and host.".into(),
            Self::RequestError(e) => format!("RPC request failed: {e}"),
            Self::Other(e) => e.clone(),
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::EndpointBindFailed(_) => "endpoint_bind_failed",
            Self::QuicConnectFailed(_) => "quic_connect_failed",
            Self::AlpnMismatch => "alpn_mismatch",
            Self::ConnectionClosed => "connection_closed",
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
    pub sync_ms: Option<u64>,
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

/// Single-threaded session state owned by UI thread.
#[derive(Clone, Debug, Default)]
pub struct SessionState {
    pub phase: ConnectPhase,
    pub snapshot: ConnectSnapshot,
    pub started_at: Option<std::time::Instant>,
    pub reconnect_attempt: Option<u32>,
    pub phase_before_idle: Option<ConnectPhase>,
}

fn updated_last_alive_at(
    previous: Option<&TransportSnapshot>,
    bytes_recv_total: u64,
    now: std::time::Instant,
) -> Option<std::time::Instant> {
    match previous {
        Some(prev) if bytes_recv_total <= prev.bytes_recv => prev.last_alive_at,
        _ => Some(now),
    }
}

impl SessionState {
    pub fn elapsed_secs(&self) -> u64 {
        self.started_at.map(|t| t.elapsed().as_secs()).unwrap_or(0)
    }

    pub fn elapsed_ms(&self) -> u64 {
        self.started_at
            .map(|t| t.elapsed().as_millis() as u64)
            .unwrap_or(0)
    }
}

/// Should migrate two single-threaded object and own by ui thread of gpui
/// Event listener wired up in gpui task
impl SessionState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn phase(&self) -> ConnectPhase {
        self.phase.clone()
    }

    pub fn is_connected(&self) -> bool {
        self.phase().is_connected()
    }

    pub fn snapshot(&self) -> ConnectSnapshot {
        self.snapshot.clone()
    }

    pub fn apply_event(&mut self, event: ConnectEvent) {
        let snap = &mut self.snapshot;

        match event {
            ConnectEvent::BindingEndpoint => {
                self.phase = ConnectPhase::BindingEndpoint;
                self.started_at = Some(std::time::Instant::now());
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
                self.phase = ConnectPhase::HolePunching;
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
                self.phase = ConnectPhase::Registering;
                snap.session_id = Some(session_id);
                snap.is_first_pairing = true;
            }
            ConnectEvent::RegisterComplete { register_ms } => {
                snap.register_ms = Some(register_ms);
            }
            ConnectEvent::Authenticating => {
                self.phase = ConnectPhase::Authenticating;
            }
            ConnectEvent::Proving => {
                self.phase = ConnectPhase::Proving;
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
                self.phase = ConnectPhase::Sync;
            }
            ConnectEvent::SyncComplete { sync, sync_ms } => {
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
                snap.sync_ms = Some(sync_ms);
            }
            ConnectEvent::TerminalsReattached { resume_ms, .. } => {
                snap.resume_ms = Some(resume_ms);
            }
            ConnectEvent::Connected { .. } => {
                self.phase = ConnectPhase::Connected;
                self.reconnect_attempt = None;
            }
            ConnectEvent::PathReport {
                path,
                num_paths,
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
                let last_alive_at = updated_last_alive_at(
                    snap.transport.as_ref(),
                    bytes_recv_total,
                    std::time::Instant::now(),
                );
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
                    num_paths,
                    rtt_ms: stats.rtt.as_millis() as u64,
                    bytes_sent: bytes_sent_total,
                    bytes_recv: bytes_recv_total,
                    path_upgraded,
                    network_hint,
                    last_alive_at,
                });
            }
            ConnectEvent::PathUpgraded { .. } => {
                if let Some(ref mut t) = snap.transport {
                    t.path_upgraded = true;
                }
            }
            ConnectEvent::NoActivePath => {
                // Keep the last transport metadata and last_alive_at so UI can
                // retain the last known path while idle state is driven by the
                // liveness events above.
            }
            ConnectEvent::ReconnectStarted { reason } => {
                self.phase = ConnectPhase::Reconnecting {
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
                self.phase = ConnectPhase::Reconnecting {
                    attempt,
                    reason,
                    next_retry_secs,
                };
                self.reconnect_attempt = Some(attempt);
            }
            ConnectEvent::ReconnectSuccess { .. } => {
                self.phase = ConnectPhase::Connected;
                self.reconnect_attempt = None;
            }
            ConnectEvent::ReconnectExhausted { error, .. } => {
                self.phase = ConnectPhase::Failed(error);
            }
            ConnectEvent::Failed { error } => {
                snap.failed_at_step = self.phase.step_index();
                self.phase = ConnectPhase::Failed(error);
            }
            ConnectEvent::ConnectionClosed => {
                // Transport closes are failed connection state; manual disconnect
                // is handled by WorkspaceState::mark_disconnected().
                if !matches!(
                    self.phase,
                    ConnectPhase::Failed(_) | ConnectPhase::Reconnecting { .. }
                ) {
                    self.phase = ConnectPhase::Failed(ConnectError::ConnectionClosed);
                }
            }
            ConnectEvent::ConnectionIdle => {
                let is_idle = matches!(self.phase, ConnectPhase::Idle { .. });
                let is_connected = matches!(self.phase, ConnectPhase::Connected);
                if !is_idle && is_connected {
                    self.phase_before_idle = Some(self.phase.clone());
                    self.phase = ConnectPhase::Idle {
                        idle_since: std::time::Instant::now(),
                    };
                }
            }
            ConnectEvent::ConnectionActive => {
                let is_idle = matches!(self.phase, ConnectPhase::Idle { .. });
                if is_idle {
                    let prev_phase = self
                        .phase_before_idle
                        .clone()
                        .unwrap_or(ConnectPhase::Connected);
                    self.phase = prev_phase;
                    self.phase_before_idle = None;
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
    snap.sync_ms = None;
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

#[cfg(test)]
mod tests {
    use super::*;
    use zedra_rpc::proto::SyncSessionResult;

    fn sync_session_result() -> SyncSessionResult {
        SyncSessionResult {
            session_id: "session-1".into(),
            session_token: [1; 32],
            hostname: "host".into(),
            workdir: "/workspace".into(),
            username: "user".into(),
            home_dir: Some("/home/user".into()),
            os: Some("macos".into()),
            arch: Some("aarch64".into()),
            os_version: Some("26.0".into()),
            host_version: Some("0.1.1".into()),
            terminals: Vec::new(),
        }
    }

    #[test]
    fn connection_active_restores_connected_after_idle() {
        let mut state = SessionState::new();

        state.apply_event(ConnectEvent::Connected { total_ms: 0 });
        assert!(matches!(state.phase, ConnectPhase::Connected));

        state.apply_event(ConnectEvent::ConnectionIdle);
        assert!(matches!(state.phase, ConnectPhase::Idle { .. }));

        state.apply_event(ConnectEvent::ConnectionActive);
        assert!(matches!(state.phase, ConnectPhase::Connected));
    }

    #[test]
    fn connection_idle_does_not_override_reconnecting() {
        let mut state = SessionState::new();

        state.apply_event(ConnectEvent::ReconnectStarted {
            reason: ReconnectReason::ConnectionLost,
        });
        state.apply_event(ConnectEvent::ConnectionIdle);

        assert!(matches!(state.phase, ConnectPhase::Reconnecting { .. }));
    }

    #[test]
    fn syncing_phase_stays_visible_until_connected() {
        let mut state = SessionState::new();

        state.apply_event(ConnectEvent::Syncing);
        assert!(matches!(state.phase, ConnectPhase::Sync));

        state.apply_event(ConnectEvent::SyncComplete {
            sync: sync_session_result(),
            sync_ms: 7,
        });
        assert!(matches!(state.phase, ConnectPhase::Sync));
        assert_eq!(state.snapshot.sync_ms, Some(7));

        state.apply_event(ConnectEvent::Connected { total_ms: 10 });
        assert!(matches!(state.phase, ConnectPhase::Connected));
    }

    #[test]
    fn connection_closed_does_not_hide_failed_reason() {
        for error in [
            ConnectError::AlpnMismatch,
            ConnectError::HandshakeConsumed,
            ConnectError::SessionOccupied,
            ConnectError::HostUnreachable,
        ] {
            let mut state = SessionState::new();

            state.apply_event(ConnectEvent::Failed {
                error: error.clone(),
            });
            state.apply_event(ConnectEvent::ConnectionClosed);

            assert_eq!(state.phase, ConnectPhase::Failed(error));
        }
    }

    #[test]
    fn connection_closed_after_connected_becomes_failed() {
        for initial_event in [
            ConnectEvent::Connected { total_ms: 10 },
            ConnectEvent::ReconnectSuccess {
                attempt: 2,
                elapsed_ms: 20,
            },
        ] {
            let mut state = SessionState::new();

            state.apply_event(initial_event);
            state.apply_event(ConnectEvent::ConnectionClosed);

            assert_eq!(
                state.phase,
                ConnectPhase::Failed(ConnectError::ConnectionClosed)
            );
        }
    }

    #[test]
    fn connection_closed_after_idle_becomes_failed() {
        let mut state = SessionState::new();

        state.apply_event(ConnectEvent::Connected { total_ms: 10 });
        state.apply_event(ConnectEvent::ConnectionIdle);
        state.apply_event(ConnectEvent::ConnectionClosed);

        assert_eq!(
            state.phase,
            ConnectPhase::Failed(ConnectError::ConnectionClosed)
        );
    }

    #[test]
    fn connection_closed_before_progress_becomes_failed() {
        let mut state = SessionState::new();

        state.apply_event(ConnectEvent::ConnectionClosed);

        assert_eq!(
            state.phase,
            ConnectPhase::Failed(ConnectError::ConnectionClosed)
        );
    }

    #[test]
    fn connection_closed_while_reconnecting_keeps_reconnecting_status() {
        let mut state = SessionState::new();

        state.apply_event(ConnectEvent::ReconnectAttempt {
            attempt: 3,
            reason: ReconnectReason::ConnectionLost,
            next_retry_secs: 5,
        });
        state.apply_event(ConnectEvent::ConnectionClosed);

        assert_eq!(
            state.phase,
            ConnectPhase::Reconnecting {
                attempt: 3,
                reason: ReconnectReason::ConnectionLost,
                next_retry_secs: 5,
            }
        );
    }

    #[test]
    fn reconnect_attempt_updates_retry_countdown() {
        let mut state = SessionState::new();

        state.apply_event(ConnectEvent::ReconnectAttempt {
            attempt: 2,
            reason: ReconnectReason::ConnectionLost,
            next_retry_secs: 3,
        });
        state.apply_event(ConnectEvent::ReconnectAttempt {
            attempt: 2,
            reason: ReconnectReason::ConnectionLost,
            next_retry_secs: 2,
        });
        state.apply_event(ConnectEvent::ReconnectAttempt {
            attempt: 2,
            reason: ReconnectReason::ConnectionLost,
            next_retry_secs: 0,
        });

        assert_eq!(
            state.phase,
            ConnectPhase::Reconnecting {
                attempt: 2,
                reason: ReconnectReason::ConnectionLost,
                next_retry_secs: 0,
            }
        );
    }

    #[test]
    fn connection_closed_during_bootstrap_becomes_failed() {
        for phase_event in [
            ConnectEvent::BindingEndpoint,
            ConnectEvent::HolePunchStarted,
            ConnectEvent::Registering {
                session_id: "session-1".into(),
            },
            ConnectEvent::Authenticating,
            ConnectEvent::Proving,
            ConnectEvent::Syncing,
        ] {
            let mut state = SessionState::new();

            state.apply_event(phase_event);
            state.apply_event(ConnectEvent::ConnectionClosed);

            assert_eq!(
                state.phase,
                ConnectPhase::Failed(ConnectError::ConnectionClosed)
            );
        }
    }

    #[test]
    fn later_failed_event_overrides_generic_connection_closed_failure() {
        let mut state = SessionState::new();

        state.apply_event(ConnectEvent::Authenticating);
        state.apply_event(ConnectEvent::ConnectionClosed);
        state.apply_event(ConnectEvent::Failed {
            error: ConnectError::SessionOccupied,
        });

        assert_eq!(
            state.phase,
            ConnectPhase::Failed(ConnectError::SessionOccupied)
        );
    }

    #[test]
    fn reconnect_exhausted_uses_final_error() {
        for error in [
            ConnectError::AlpnMismatch,
            ConnectError::SessionOccupied,
            ConnectError::HostUnreachable,
        ] {
            let mut state = SessionState::new();

            state.apply_event(ConnectEvent::ReconnectExhausted {
                attempts: 1,
                elapsed_ms: 0,
                error: error.clone(),
            });

            assert_eq!(state.phase, ConnectPhase::Failed(error));
        }
    }

    #[test]
    fn primary_connection_errors_have_friendly_status_metadata() {
        let cases = [
            (
                ConnectError::AlpnMismatch,
                "alpn_mismatch",
                true,
                "Protocol mismatch, Update App or CLI",
            ),
            (
                ConnectError::ConnectionClosed,
                "connection_closed",
                false,
                "Connection closed. Tap refresh to reconnect.",
            ),
            (
                ConnectError::HandshakeConsumed,
                "handshake_consumed",
                true,
                "The QR code was used. Refresh it and scan again.",
            ),
            (
                ConnectError::SessionOccupied,
                "session_occupied",
                true,
                "Host occupied. Disconnect other device and retry.",
            ),
            (
                ConnectError::HostUnreachable,
                "host_unreachable",
                false,
                "Host unreachable. Check network and host.",
            ),
        ];

        for (error, label, is_fatal, message) in cases {
            assert_eq!(error.label(), label);
            assert_eq!(error.is_fatal(), is_fatal);
            assert_eq!(error.user_message(), message);
            assert_eq!(error.to_string(), message);
        }
    }

    #[test]
    fn failed_event_records_step_for_status_details() {
        let mut state = SessionState::new();

        state.apply_event(ConnectEvent::HolePunchStarted);
        state.apply_event(ConnectEvent::Failed {
            error: ConnectError::AlpnMismatch,
        });

        assert_eq!(state.snapshot.failed_at_step, Some(0));
    }

    #[test]
    fn updated_last_alive_at_only_refreshes_on_receive_progress() {
        let previous_alive_at = std::time::Instant::now();
        let previous = TransportSnapshot {
            is_direct: false,
            remote_addr: "relay".into(),
            relay_url: Some("https://relay.example".into()),
            num_paths: 1,
            rtt_ms: 42,
            bytes_sent: 100,
            bytes_recv: 200,
            path_upgraded: false,
            network_hint: None,
            last_alive_at: Some(previous_alive_at),
        };

        let same_bytes = updated_last_alive_at(
            Some(&previous),
            previous.bytes_recv,
            previous_alive_at + std::time::Duration::from_secs(5),
        );
        assert_eq!(same_bytes, Some(previous_alive_at));

        let advanced = updated_last_alive_at(
            Some(&previous),
            previous.bytes_recv + 1,
            previous_alive_at + std::time::Duration::from_secs(5),
        );
        assert!(advanced.is_some());
        assert_ne!(advanced, Some(previous_alive_at));
    }
}
