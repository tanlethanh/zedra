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
/// When adding a new feature, define a new variant here with inline fields
/// covering timing, counts, path/transport info, and version as appropriate.
/// Never include personal data (usernames, file contents, IPs).
#[derive(Clone, Debug)]
pub enum Event {
    // ═══════════════════════════════════════════════════════════════════════
    // APP EVENTS — mobile app + shared crates (zedra-session)
    // ═══════════════════════════════════════════════════════════════════════

    // ── App lifecycle ──────────────────────────────────────────────────────
    /// App launched (cold start).
    AppOpen {
        saved_workspaces: usize,
        app_version: String,
        platform: &'static str,
        arch: &'static str,
    },
    /// User navigated to a new screen.
    ScreenView {
        /// e.g. "home", "workspace"
        screen: &'static str,
    },

    // ── QR / pairing ──────────────────────────────────────────────────────
    /// User tapped the "Scan QR" button.
    QrScanInitiated,
    /// User tapped a saved workspace on the home screen to connect/switch to it.
    WorkspaceSelected {
        /// "active" = workspace already open (just switching to it),
        /// "saved" = reconnecting to a saved workspace from history.
        source: &'static str,
    },

    // ── Connection (client-side) ───────────────────────────────────────────
    /// Connection established successfully.
    ConnectSuccess {
        // Phase timings (ms)
        total_ms: u64,
        binding_ms: u64,
        hole_punch_ms: u64,
        auth_ms: u64,
        fetch_ms: u64,
        // Transport context
        /// "direct" or "relay"
        path: &'static str,
        /// Network classification: "LAN", "Tailscale", "Internet", "unknown"
        network: &'static str,
        rtt_ms: u64,
        /// Relay hostname (e.g. "sg1.relay.zedra.dev") or "none" / "custom"
        relay: String,
        relay_latency_ms: u64,
        /// ALPN protocol (e.g. "zedra/rpc/3")
        alpn: String,
        // Discovery
        has_ipv4: bool,
        has_ipv6: bool,
        symmetric_nat: bool,
        is_first_pairing: bool,
    },
    /// Connection attempt failed.
    ConnectFailed {
        /// Phase label where failure occurred (e.g. "hole_punching", "authenticating")
        phase: &'static str,
        /// Error label (e.g. "quic_connect_failed", "host_signature_invalid")
        error: &'static str,
        /// Time from connect() call to failure.
        elapsed_ms: u64,
        relay: String,
        alpn: String,
        has_ipv4: bool,
        has_ipv6: bool,
        relay_connected: bool,
    },
    /// Session resumed with existing server-side terminals.
    SessionResumed {
        terminal_count: usize,
        resume_ms: u64,
    },
    /// User-initiated disconnect.
    Disconnect,

    // ── Reconnect (client-side) ────────────────────────────────────────────
    /// Auto-reconnect loop started.
    ReconnectStarted {
        /// "connection_lost" or "app_foregrounded"
        reason: &'static str,
    },
    /// Reconnect succeeded after N attempts.
    ReconnectSuccess {
        attempt: u32,
        /// Wall time from reconnect loop start to success.
        elapsed_ms: u64,
        reason: &'static str,
        // Phase timings (ms) for the successful attempt
        binding_ms: u64,
        hole_punch_ms: u64,
        auth_ms: u64,
        fetch_ms: u64,
        // Transport context
        /// "direct" or "relay"
        path: &'static str,
        /// Network classification: "LAN", "Tailscale", "Internet", "unknown"
        network: &'static str,
        rtt_ms: u64,
        relay: String,
        alpn: String,
        has_ipv4: bool,
        has_ipv6: bool,
    },
    /// Reconnect loop ended without success.
    /// Covers both attempt exhaustion and fatal auth errors that stop retrying early.
    ReconnectExhausted {
        attempts: u32,
        elapsed_ms: u64,
        reason: &'static str,
        /// Set when a fatal auth error (e.g. "unauthorized") stopped retrying early.
        /// None when all max attempts were used.
        fatal_error: Option<&'static str>,
    },

    // ── Transport (client-side) ────────────────────────────────────────────
    /// Connection path upgraded from relay to direct P2P.
    PathUpgraded {
        /// Network classification of the new direct path.
        network: &'static str,
        rtt_ms: u64,
        /// Relay hostname the connection upgraded from.
        from_relay: String,
    },

    // ── Terminal (client-side) ─────────────────────────────────────────────
    /// A new terminal was created (PTY spawned on server).
    TerminalOpened {
        /// "new_session", "user_action"
        source: &'static str,
        /// Total terminal count after opening.
        terminal_count: usize,
    },
    /// A terminal was closed by the user.
    TerminalClosed {
        /// Remaining terminal count after closing.
        remaining: usize,
    },

    // ═══════════════════════════════════════════════════════════════════════
    // HOST EVENTS — desktop daemon (zedra-host)
    // ═══════════════════════════════════════════════════════════════════════

    // ── Daemon lifecycle ───────────────────────────────────────────────────
    /// Daemon started (`zedra start`). Fires early, before endpoint bind.
    DaemonStart {
        /// "custom" or "default"
        relay_type: &'static str,
        /// True on the very first `zedra start` on this machine (telemetry_id did not exist).
        is_first_run: bool,
    },
    /// Startup complete: endpoint bound and QR printed, daemon is accepting connections.
    StartupComplete {
        /// Time from process start to `create_endpoint()` call (identity/registry load).
        init_ms: u64,
        /// Time for iroh endpoint to bind (STUN probing, relay negotiation).
        endpoint_bind_ms: u64,
        /// Total time from process start to QR printed.
        total_ms: u64,
    },
    /// STUN/network report completed at startup.
    NetReport {
        has_ipv4: bool,
        has_ipv6: bool,
        symmetric_nat: bool,
    },

    // ── Auth (server-side) ─────────────────────────────────────────────────
    /// New device paired via QR code (first-time Register flow).
    ClientPaired,
    /// Client authenticated and entered the RPC loop.
    AuthSuccess {
        /// true for first-ever pairing (Register), false for reconnect.
        is_new_client: bool,
        /// Time for the Register phase (HMAC verify + slot consume). 0 on reconnect.
        register_ms: u64,
        /// Time for the Authenticate → AuthChallenge round-trip.
        challenge_ms: u64,
        /// Time for the AuthProve round-trip (signature verify + session attach).
        prove_ms: u64,
        /// Total wall time from inbound accept to RPC loop entry.
        total_ms: u64,
        /// "direct", "relay", or "unknown"
        path_type: &'static str,
    },
    /// Authentication rejected.
    AuthFailed {
        /// Specific failure label:
        /// "stale_timestamp", "bad_hmac", "slot_consumed", "slot_not_found",
        /// "not_authorized", "unexpected_message", "nonce_mismatch",
        /// "invalid_signature", "session_occupied", "session_not_found",
        /// "not_in_session_acl", "io_error"
        reason: &'static str,
        /// Time elapsed in the auth handshake before failure.
        elapsed_ms: u64,
        /// Whether the failure was on a new pairing (Register) vs reconnect (Authenticate).
        is_new_client: bool,
        /// Connection path at time of failure: "direct", "relay", or "unknown"
        path_type: &'static str,
    },

    // ── Session (server-side) ──────────────────────────────────────────────
    /// Client disconnected; session stays alive in registry.
    SessionEnd {
        /// How long the authenticated RPC session lasted.
        duration_ms: u64,
        /// Number of terminals that existed during the session.
        terminal_count: u64,
        /// "direct", "relay", or "unknown"
        path_type: &'static str,
        // ── Lifetime RPC usage counters for this session ──
        fs_reads: u64,
        fs_writes: u64,
        /// Read-only git ops: status, diff, log, branches, stage, unstage.
        git_ops: u64,
        git_commits: u64,
        ai_prompts: u64,
    },

    // ── Feature usage (server-side, high-value individual events) ──────────
    /// An AI prompt was sent to the Claude CLI and completed.
    AiPromptSent {
        success: bool,
        duration_ms: u64,
        /// Length of the prompt in bytes (not content).
        prompt_bytes: usize,
        /// Length of the response in bytes.
        response_bytes: usize,
    },
    /// A git commit was made via the app.
    GitCommitMade {
        /// Number of paths included in the commit.
        files_staged: usize,
        success: bool,
    },

    // ── Terminal (server-side) ─────────────────────────────────────────────
    /// A new terminal PTY was spawned on the host.
    HostTerminalOpen {
        /// Whether a launch command was injected (e.g. "claude --resume").
        has_launch_cmd: bool,
    },

    // ── Monitoring (server-side) ───────────────────────────────────────────
    /// Periodic daemon heartbeat (every 10 minutes) for uptime tracking.
    DaemonHeartbeat {
        uptime_secs: u64,
        session_count: usize,
        terminal_count: usize,
    },

    /// Periodic bandwidth sample from the active iroh path (per-interval deltas).
    BandwidthSample {
        bytes_sent: u64,
        bytes_recv: u64,
        interval_secs: u64,
    },

    // ── Self-update (server-side) ──────────────────────────────────────────
    /// Background version check completed at daemon start.
    UpdateChecked {
        /// Whether a newer version is available.
        update_available: bool,
        /// The latest version tag (e.g. "v0.3.0"), or empty string if already up-to-date or check failed.
        latest_version: String,
        /// Current running version (e.g. "0.2.1").
        current_version: &'static str,
    },
    /// `zedra update` ran to completion (success or failure).
    SelfUpdate {
        success: bool,
        /// Target version tag (e.g. "v0.3.0").
        target_version: String,
        /// Current version before update (e.g. "0.2.1").
        from_version: &'static str,
        /// Error label on failure (e.g. "download_failed", "checksum_mismatch",
        /// "extract_failed", "install_failed"); empty on success.
        error: &'static str,
        /// Wall time from update start to finish (ms).
        elapsed_ms: u64,
    },
}

// ---------------------------------------------------------------------------
// Event → (name, params) serialization
// ---------------------------------------------------------------------------

impl Event {
    /// Event name for the telemetry backend (Firebase event name / GA4 event name).
    pub fn name(&self) -> &'static str {
        match self {
            Self::AppOpen { .. } => "app_open",
            Self::ScreenView { .. } => "screen_view",
            Self::QrScanInitiated => "qr_scan_initiated",
            Self::WorkspaceSelected { .. } => "workspace_selected",
            Self::ConnectSuccess { .. } => "connect_success",
            Self::ConnectFailed { .. } => "connect_failed",
            Self::SessionResumed { .. } => "session_resumed",
            Self::Disconnect => "disconnect",
            Self::ReconnectStarted { .. } => "reconnect_started",
            Self::ReconnectSuccess { .. } => "reconnect_success",
            Self::ReconnectExhausted { .. } => "reconnect_exhausted",
            Self::PathUpgraded { .. } => "path_upgraded",
            Self::TerminalOpened { .. } => "terminal_opened",
            Self::TerminalClosed { .. } => "terminal_closed",
            Self::DaemonStart { .. } => "daemon_start",
            Self::StartupComplete { .. } => "startup_complete",
            Self::NetReport { .. } => "net_report",
            Self::ClientPaired => "client_paired",
            Self::AuthSuccess { .. } => "auth_success",
            Self::AuthFailed { .. } => "auth_failed",
            Self::SessionEnd { .. } => "session_end",
            Self::AiPromptSent { .. } => "ai_prompt_sent",
            Self::GitCommitMade { .. } => "git_commit_made",
            Self::HostTerminalOpen { .. } => "terminal_open",
            Self::DaemonHeartbeat { .. } => "daemon_heartbeat",
            Self::BandwidthSample { .. } => "bandwidth_sample",
            Self::UpdateChecked { .. } => "update_checked",
            Self::SelfUpdate { .. } => "self_update",
        }
    }

    /// Serialize event fields as key-value string pairs for platform backends.
    pub fn to_params(&self) -> Vec<(&str, String)> {
        match self {
            Self::AppOpen {
                saved_workspaces,
                app_version,
                platform,
                arch,
            } => vec![
                ("saved_workspaces", saved_workspaces.to_string()),
                ("app_version", app_version.to_string()),
                ("platform", platform.to_string()),
                ("arch", arch.to_string()),
            ],
            Self::ScreenView { screen } => vec![("screen", screen.to_string())],
            Self::QrScanInitiated | Self::Disconnect | Self::ClientPaired => vec![],
            Self::WorkspaceSelected { source } => vec![("source", source.to_string())],
            Self::ConnectSuccess {
                total_ms,
                binding_ms,
                hole_punch_ms,
                auth_ms,
                fetch_ms,
                path,
                network,
                rtt_ms,
                relay,
                relay_latency_ms,
                alpn,
                has_ipv4,
                has_ipv6,
                symmetric_nat,
                is_first_pairing,
            } => vec![
                ("total_ms", total_ms.to_string()),
                ("binding_ms", binding_ms.to_string()),
                ("hole_punch_ms", hole_punch_ms.to_string()),
                ("auth_ms", auth_ms.to_string()),
                ("fetch_ms", fetch_ms.to_string()),
                ("path", path.to_string()),
                ("network", network.to_string()),
                ("rtt_ms", rtt_ms.to_string()),
                ("relay", relay.clone()),
                ("relay_latency_ms", relay_latency_ms.to_string()),
                ("alpn", alpn.clone()),
                ("has_ipv4", bool_str(*has_ipv4)),
                ("has_ipv6", bool_str(*has_ipv6)),
                ("symmetric_nat", bool_str(*symmetric_nat)),
                ("is_first_pairing", bool_str(*is_first_pairing)),
            ],
            Self::ConnectFailed {
                phase,
                error,
                elapsed_ms,
                relay,
                alpn,
                has_ipv4,
                has_ipv6,
                relay_connected,
            } => vec![
                ("phase", phase.to_string()),
                ("error", error.to_string()),
                ("elapsed_ms", elapsed_ms.to_string()),
                ("relay", relay.clone()),
                ("alpn", alpn.clone()),
                ("has_ipv4", bool_str(*has_ipv4)),
                ("has_ipv6", bool_str(*has_ipv6)),
                ("relay_connected", bool_str(*relay_connected)),
            ],
            Self::SessionResumed {
                terminal_count,
                resume_ms,
            } => vec![
                ("terminal_count", terminal_count.to_string()),
                ("resume_ms", resume_ms.to_string()),
            ],
            Self::ReconnectStarted { reason } => vec![("reason", reason.to_string())],
            Self::ReconnectSuccess {
                attempt,
                elapsed_ms,
                reason,
                binding_ms,
                hole_punch_ms,
                auth_ms,
                fetch_ms,
                path,
                network,
                rtt_ms,
                relay,
                alpn,
                has_ipv4,
                has_ipv6,
            } => vec![
                ("attempt", attempt.to_string()),
                ("elapsed_ms", elapsed_ms.to_string()),
                ("reason", reason.to_string()),
                ("binding_ms", binding_ms.to_string()),
                ("hole_punch_ms", hole_punch_ms.to_string()),
                ("auth_ms", auth_ms.to_string()),
                ("fetch_ms", fetch_ms.to_string()),
                ("path", path.to_string()),
                ("network", network.to_string()),
                ("rtt_ms", rtt_ms.to_string()),
                ("relay", relay.clone()),
                ("alpn", alpn.clone()),
                ("has_ipv4", bool_str(*has_ipv4)),
                ("has_ipv6", bool_str(*has_ipv6)),
            ],
            Self::ReconnectExhausted {
                attempts,
                elapsed_ms,
                reason,
                fatal_error,
            } => {
                let mut v = vec![
                    ("attempts", attempts.to_string()),
                    ("elapsed_ms", elapsed_ms.to_string()),
                    ("reason", reason.to_string()),
                ];
                if let Some(e) = fatal_error {
                    v.push(("fatal_error", e.to_string()));
                }
                v
            }
            Self::PathUpgraded {
                network,
                rtt_ms,
                from_relay,
            } => vec![
                ("network", network.to_string()),
                ("rtt_ms", rtt_ms.to_string()),
                ("from_relay", from_relay.clone()),
            ],
            Self::TerminalOpened {
                source,
                terminal_count,
            } => vec![
                ("source", source.to_string()),
                ("terminal_count", terminal_count.to_string()),
            ],
            Self::TerminalClosed { remaining } => vec![("remaining", remaining.to_string())],
            Self::DaemonStart {
                relay_type,
                is_first_run,
            } => vec![
                ("relay_type", relay_type.to_string()),
                ("is_first_run", bool_str(*is_first_run)),
            ],
            Self::StartupComplete {
                init_ms,
                endpoint_bind_ms,
                total_ms,
            } => vec![
                ("init_ms", init_ms.to_string()),
                ("endpoint_bind_ms", endpoint_bind_ms.to_string()),
                ("total_ms", total_ms.to_string()),
            ],
            Self::NetReport {
                has_ipv4,
                has_ipv6,
                symmetric_nat,
            } => vec![
                ("has_ipv4", bool_str(*has_ipv4)),
                ("has_ipv6", bool_str(*has_ipv6)),
                ("symmetric_nat", bool_str(*symmetric_nat)),
            ],
            Self::AuthSuccess {
                is_new_client,
                register_ms,
                challenge_ms,
                prove_ms,
                total_ms,
                path_type,
            } => vec![
                ("is_new_client", bool_str(*is_new_client)),
                ("register_ms", register_ms.to_string()),
                ("challenge_ms", challenge_ms.to_string()),
                ("prove_ms", prove_ms.to_string()),
                ("total_ms", total_ms.to_string()),
                ("path_type", path_type.to_string()),
            ],
            Self::AuthFailed {
                reason,
                elapsed_ms,
                is_new_client,
                path_type,
            } => vec![
                ("reason", reason.to_string()),
                ("elapsed_ms", elapsed_ms.to_string()),
                ("is_new_client", bool_str(*is_new_client)),
                ("path_type", path_type.to_string()),
            ],
            Self::SessionEnd {
                duration_ms,
                terminal_count,
                path_type,
                fs_reads,
                fs_writes,
                git_ops,
                git_commits,
                ai_prompts,
            } => vec![
                ("duration_ms", duration_ms.to_string()),
                ("terminal_count", terminal_count.to_string()),
                ("path_type", path_type.to_string()),
                ("fs_reads", fs_reads.to_string()),
                ("fs_writes", fs_writes.to_string()),
                ("git_ops", git_ops.to_string()),
                ("git_commits", git_commits.to_string()),
                ("ai_prompts", ai_prompts.to_string()),
            ],
            Self::AiPromptSent {
                success,
                duration_ms,
                prompt_bytes,
                response_bytes,
            } => vec![
                ("success", bool_str(*success)),
                ("duration_ms", duration_ms.to_string()),
                ("prompt_bytes", prompt_bytes.to_string()),
                ("response_bytes", response_bytes.to_string()),
            ],
            Self::GitCommitMade {
                files_staged,
                success,
            } => vec![
                ("files_staged", files_staged.to_string()),
                ("success", bool_str(*success)),
            ],
            Self::HostTerminalOpen { has_launch_cmd } => {
                vec![("has_launch_cmd", bool_str(*has_launch_cmd))]
            }
            Self::DaemonHeartbeat {
                uptime_secs,
                session_count,
                terminal_count,
            } => vec![
                ("uptime_secs", uptime_secs.to_string()),
                ("session_count", session_count.to_string()),
                ("terminal_count", terminal_count.to_string()),
            ],
            Self::BandwidthSample {
                bytes_sent,
                bytes_recv,
                interval_secs,
            } => vec![
                ("bytes_sent", bytes_sent.to_string()),
                ("bytes_recv", bytes_recv.to_string()),
                ("interval_secs", interval_secs.to_string()),
            ],
            Self::UpdateChecked {
                update_available,
                latest_version,
                current_version,
            } => vec![
                ("update_available", bool_str(*update_available)),
                ("latest_version", latest_version.clone()),
                ("current_version", current_version.to_string()),
            ],
            Self::SelfUpdate {
                success,
                target_version,
                from_version,
                error,
                elapsed_ms,
            } => vec![
                ("success", bool_str(*success)),
                ("target_version", target_version.clone()),
                ("from_version", from_version.to_string()),
                ("error", error.to_string()),
                ("elapsed_ms", elapsed_ms.to_string()),
            ],
        }
    }
}

// GA4 Measurement Protocol stores all event params as strings. "1"/"0" lets
// BigQuery promote them to integer_value automatically, enabling numeric filters.
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
