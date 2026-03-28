// Connection state machine for zedra-session.
//
// `ConnectState` is the single source of truth for the full connection lifecycle.
// It replaces the old SessionState + ConnectionInfo + reconnect atomics.
//
// State machine (linear phases + two side exits):
//
//   Idle
//     │
//     ▼
//   BindingEndpoint  → local_node_id, relay_url, alpn
//     │
//     ▼
//   HolePunching     → remote_node_id; path watcher begins populating transport
//     │
//     ▼
//   EstablishingRpc
//     │
//     ▼
//   Registering      (first pairing only — skipped on reconnect)
//     │
//     ▼
//   Authenticating
//     │
//     ▼
//   Proving          → session_id, auth_outcome
//     │
//     ▼
//   FetchingInfo     → hostname, username, workdir, os, …
//     │
//     ▼
//   Connected
//
//   Any phase → Failed(ConnectError)
//   Connected → Reconnecting { … } → BindingEndpoint (retry cycle)

use std::fmt;

/// Why a reconnect was triggered.
#[derive(Clone, Debug, PartialEq)]
pub enum ReconnectReason {
    /// QUIC connection closed (transport failure, timeout).
    ConnectionLost,
    /// App returned to foreground after iOS suspension.
    AppForegrounded,
}

/// Ordered connection phases. Advances monotonically during a connect() call.
/// `Reconnecting` and `Failed` are side exits from the linear sequence.
#[derive(Clone, Debug, PartialEq)]
pub enum ConnectPhase {
    /// Not yet connecting — initial or post-disconnect state.
    Idle,
    /// Creating local iroh QUIC endpoint + pkarr DNS resolver.
    BindingEndpoint,
    /// QUIC connect: relay-assisted handshake + ICE hole-punch attempt.
    HolePunching,
    /// Wrapping QUIC conn in irpc typed RPC client.
    EstablishingRpc,
    /// First pairing only: HMAC registration (prove QR possession).
    Registering,
    /// PKI challenge: request nonce from host + verify host Ed25519 signature.
    Authenticating,
    /// PKI prove: client signs nonce + AuthProve to attach to session.
    Proving,
    /// Fetching session info (hostname, workdir, OS, …) via RPC.
    FetchingInfo,
    /// Fully operational.
    Connected,
    /// Re-attaching existing terminals after session resume / reconnect.
    ResumingTerminals,
    /// Waiting to retry after connection loss.
    Reconnecting {
        attempt: u32,
        reason: ReconnectReason,
        /// Seconds remaining until next attempt (counts down, 0 = attempting now).
        next_retry_secs: u64,
    },
    /// Terminal failure — requires user action to recover.
    Failed(ConnectError),
}

/// Step index labels for the horizontal progress stepper (5 visual steps).
pub const STEPPER_STEP_NAMES: [&str; 5] = ["Init", "Connect", "Auth", "Sync", "Done"];

impl ConnectPhase {
    /// Maps to a stepper step index (0–4) for the horizontal progress bar.
    /// Returns `None` for Idle / Reconnecting / Failed (they don't map linearly).
    pub fn step_index(&self) -> Option<usize> {
        match self {
            Self::BindingEndpoint => Some(0),
            Self::HolePunching | Self::EstablishingRpc => Some(1),
            Self::Registering | Self::Authenticating | Self::Proving => Some(2),
            Self::FetchingInfo | Self::ResumingTerminals => Some(3),
            Self::Connected => Some(4),
            _ => None,
        }
    }

    /// Short machine-readable label for telemetry events.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::BindingEndpoint => "binding_endpoint",
            Self::HolePunching => "hole_punching",
            Self::EstablishingRpc => "establishing_rpc",
            Self::Registering => "registering",
            Self::Authenticating => "authenticating",
            Self::Proving => "proving",
            Self::FetchingInfo => "fetching_info",
            Self::Connected => "connected",
            Self::ResumingTerminals => "resuming_terminals",
            Self::Reconnecting { .. } => "reconnecting",
            Self::Failed(_) => "failed",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::BindingEndpoint => "Creating endpoint",
            Self::HolePunching => "Hole punching",
            Self::EstablishingRpc => "Establishing RPC",
            Self::Registering => "Registering",
            Self::Authenticating => "Authenticating",
            Self::Proving => "Proving identity",
            Self::FetchingInfo => "Fetching info",
            Self::Connected => "Connected",
            Self::ResumingTerminals => "Resuming terminals",
            Self::Reconnecting { .. } => "Reconnecting",
            Self::Failed(_) => "Failed",
        }
    }

    pub fn is_idle(&self) -> bool {
        matches!(self, Self::Idle)
    }

    pub fn is_connecting(&self) -> bool {
        matches!(
            self,
            Self::BindingEndpoint
                | Self::HolePunching
                | Self::EstablishingRpc
                | Self::Registering
                | Self::Authenticating
                | Self::Proving
                | Self::FetchingInfo
                | Self::ResumingTerminals
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

/// Network classification for a direct P2P connection.
#[derive(Clone, Debug, PartialEq)]
pub enum NetworkHint {
    /// Tailscale CGNAT range: 100.64.0.0/10 (100.64–100.127).
    Tailscale,
    /// RFC 1918 private LAN: 10/8, 172.16/12, 192.168/16.
    Lan,
    /// Publicly routable address.
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

/// Live transport path data. Updated by the iroh path watcher task.
#[derive(Clone, Debug)]
pub struct TransportSnapshot {
    pub is_direct: bool,
    /// IP:port if direct, relay hostname if relayed.
    pub remote_addr: String,
    pub relay_url: Option<String>,
    pub num_paths: usize,
    pub rtt_ms: u64,
    pub bytes_sent: u64,
    pub bytes_recv: u64,
    /// True once a relay→P2P path upgrade has been observed on this connection.
    pub path_upgraded: bool,
    /// Network classification for direct connections (`None` when relayed).
    pub network_hint: Option<NetworkHint>,
    /// Wall-clock instant when a non-zero RTT was last observed.
    /// `None` = never seen alive on this connection.
    /// Render code computes elapsed from this to get a live ticking counter.
    pub last_alive_at: Option<std::time::Instant>,
}

/// All data accumulated during the connection process.
/// Fields are `Option` and filled in as phases complete.
/// The UI renders whatever is populated at each point in time.
#[derive(Clone, Debug, Default)]
pub struct ConnectSnapshot {
    // ── Phase timing (ms, set when each phase completes) ──────────────────
    pub binding_ms: Option<u64>,
    pub hole_punch_ms: Option<u64>,
    pub rpc_ms: Option<u64>,
    /// `None` on a reconnect (no Registering step needed).
    pub register_ms: Option<u64>,
    pub auth_ms: Option<u64>,
    pub fetch_ms: Option<u64>,
    /// Time spent re-attaching existing terminals (ms). `None` if no resume was needed.
    pub resume_ms: Option<u64>,

    // ── Endpoint (populated at start of BindingEndpoint) ──────────────────
    /// Local iroh NodeId (short form, 8 hex chars).
    pub local_node_id: Option<String>,
    /// Remote iroh NodeId (short form).
    pub remote_node_id: Option<String>,
    /// Relay server URL being used (e.g. "relay.zedra.dev").
    pub relay_url: Option<String>,
    /// ALPN protocol identifier (e.g. "zedra/rpc/3").
    pub alpn: Option<String>,

    // ── Discovery (live, updated during BindingEndpoint / HolePunching) ───
    /// True once the local endpoint has connected to its home relay.
    pub relay_connected: bool,
    /// Discovered direct endpoint addresses shown in the connecting UI.
    pub direct_addrs: Vec<String>,
    /// IPv4 UDP reachability confirmed by net-report.
    pub has_ipv4: bool,
    /// IPv6 UDP reachability confirmed by net-report.
    pub has_ipv6: bool,
    /// Whether NAT mapping varies by destination (symmetric NAT → harder hole-punch).
    pub mapping_varies: Option<bool>,
    /// RTT to the preferred relay server (ms).
    pub relay_latency_ms: Option<u64>,
    /// Captive portal detected.
    pub captive_portal: Option<bool>,

    // ── Transport (live, updated by path watcher after HolePunching) ──────
    pub transport: Option<TransportSnapshot>,

    // ── Auth (populated after Proving) ────────────────────────────────────
    pub is_first_pairing: bool,
    pub session_id: Option<String>,
    pub auth_outcome: Option<AuthOutcome>,

    // ── Host (populated after FetchingInfo completes) ──────────────────────
    pub hostname: Option<String>,
    pub username: Option<String>,
    pub workdir: Option<String>,
    pub os: Option<String>,
    pub arch: Option<String>,
    pub os_version: Option<String>,
    pub host_version: Option<String>,

    // ── Failure info ───────────────────────────────────────────────────────
    /// Step index at which the failure occurred (for stepper rendering).
    pub failed_at_step: Option<usize>,
}

/// The full connection state — single source of truth for all UI.
#[derive(Clone, Debug)]
pub struct ConnectState {
    pub phase: ConnectPhase,
    pub snapshot: ConnectSnapshot,
    /// Wall-clock instant when the current connect attempt began. Set at the
    /// start of `connect()` and reset on each new attempt.  Used by the UI to
    /// detect a stuck connection (elapsed > 30 s → show prominent retry button).
    pub started_at: Option<std::time::Instant>,
    /// Set to `Some(N)` when a reconnect loop is active (attempt number).
    /// `None` for initial connects.  Persists across the `Reconnecting →
    /// BindingEndpoint → …` phase transitions so the transport badge can show
    /// "Retry N" during the actual attempt phases, not just the countdown.
    pub reconnect_attempt: Option<u32>,
}

impl ConnectState {
    pub fn idle() -> Self {
        Self {
            phase: ConnectPhase::Idle,
            snapshot: ConnectSnapshot::default(),
            started_at: None,
            reconnect_attempt: None,
        }
    }

    /// Seconds elapsed since the current connect attempt started, or 0.
    pub fn elapsed_secs(&self) -> u64 {
        self.started_at.map(|t| t.elapsed().as_secs()).unwrap_or(0)
    }

    /// Milliseconds elapsed since the current connect attempt started, or 0.
    pub fn elapsed_ms(&self) -> u64 {
        self.started_at
            .map(|t| t.elapsed().as_millis() as u64)
            .unwrap_or(0)
    }
}

impl Default for ConnectState {
    fn default() -> Self {
        Self::idle()
    }
}

/// Auth outcome recorded after Proving completes.
#[derive(Clone, Debug, PartialEq)]
pub enum AuthOutcome {
    /// First pairing: Registration + auth succeeded.
    Registered,
    /// Reconnect: challenge-response auth succeeded.
    Authenticated,
}

/// All connection failure reasons — typed 1:1 with protocol errors + transport.
#[derive(Clone, Debug, PartialEq)]
pub enum ConnectError {
    // Transport
    EndpointBindFailed(String),
    QuicConnectFailed(String),
    // Registration (first pairing only)
    HandshakeConsumed,
    InvalidHandshake,
    StaleTimestamp,
    SlotNotFound,
    // Auth (every connection)
    Unauthorized,
    NotInSessionAcl,
    SessionOccupied,
    SessionNotFound,
    InvalidSignature,
    HostSignatureInvalid,
    // RPC
    SessionInfoFailed(String),
    // Reconnect exhausted
    HostUnreachable,
    Other(String),
}

impl ConnectError {
    /// Whether this error is terminal — retrying won't help.
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
            Self::HostSignatureInvalid => "Host identity check failed (possible MITM).".into(),
            Self::SessionInfoFailed(e) => format!("Failed to fetch session info: {e}"),
            Self::HostUnreachable => "Host unreachable after repeated attempts.".into(),
            Self::Other(e) => e.clone(),
        }
    }

    /// Short machine-readable label for telemetry events.
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
            Self::HostSignatureInvalid => "host_signature_invalid",
            Self::SessionInfoFailed(_) => "session_info_failed",
            Self::HostUnreachable => "host_unreachable",
            Self::Other(_) => "other",
        }
    }

    /// Returns `Some(label)` if this error is fatal (retrying won't help), else `None`.
    pub fn fatal_label(&self) -> Option<&'static str> {
        if self.is_fatal() {
            Some(self.label())
        } else {
            None
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
