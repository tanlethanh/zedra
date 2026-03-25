use std::collections::HashSet;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::Instant;

use anyhow::Result;
use zedra_rpc::proto::*;
use zedra_telemetry::*;

use crate::connect_state::{
    AuthOutcome, ConnectError, ConnectPhase, ConnectState, NetworkHint, ReconnectReason,
    TransportSnapshot,
};

/// Classify a remote IP address for debugging display.
/// Return the relay hostname for telemetry, redacting custom/private relays.
/// Built-in zedra.dev relays are kept verbatim; anything else becomes "custom"
/// to avoid leaking private IPs or internal hostnames.
fn sanitize_relay(relay_url: Option<&str>) -> String {
    match relay_url {
        None => "none".into(),
        Some(url) if url == "none" => "none".into(),
        Some(url) => {
            // Extract hostname from a full URL or bare hostname string.
            // Strip scheme ("https://"), port (":443"), and path ("/relay").
            let after_scheme = url.find("://").map(|i| &url[i + 3..]).unwrap_or(url);
            let host = after_scheme
                .split('/')
                .next()
                .unwrap_or(after_scheme)
                .split(':')
                .next()
                .unwrap_or(after_scheme);
            if host.ends_with(".zedra.dev") || host == "zedra.dev" {
                host.to_string()
            } else {
                "custom".into()
            }
        }
    }
}

fn classify_ip(ip: std::net::IpAddr) -> NetworkHint {
    match ip {
        std::net::IpAddr::V4(v4) => {
            let o = v4.octets();
            // Tailscale CGNAT: 100.64.0.0/10 → first octet 100, second 64–127
            if o[0] == 100 && o[1] >= 64 && o[1] <= 127 {
                return NetworkHint::Tailscale;
            }
            // RFC 1918
            if o[0] == 10
                || (o[0] == 172 && o[1] >= 16 && o[1] <= 31)
                || (o[0] == 192 && o[1] == 168)
            {
                return NetworkHint::Lan;
            }
            NetworkHint::Internet
        }
        // IPv6 link-local / ULA → treat as LAN; everything else Internet
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
use crate::{push_callback, session_runtime, signer::ClientSigner, terminal::RemoteTerminal};

/// Workspace-scoped durable session state. Arc-wrapped — clone freely to share
/// with async tasks. Survives transport failures and reconnect cycles.
#[derive(Clone)]
pub struct SessionHandle(Arc<SessionHandleInner>);

struct SessionHandleInner {
    // ── Durable credentials / identity ─────────────────────────────────────
    sid: Arc<Mutex<Option<String>>>,
    endpoint_addr: Mutex<Option<iroh::EndpointAddr>>,
    terminals: Arc<Mutex<Vec<Arc<RemoteTerminal>>>>,
    signer: Mutex<Option<Arc<dyn ClientSigner>>>,
    endpoint_id: Mutex<Option<iroh::PublicKey>>,
    pending_ticket: Mutex<Option<zedra_rpc::ZedraPairingTicket>>,

    // ── Live connection ──────────────────────────────────────────────────────
    client: Mutex<Option<irpc::Client<ZedraProto>>>,

    // ── Connection state machine (single source of truth) ───────────────────
    connect_state: Mutex<ConnectState>,

    /// Monotonically increasing counter bumped on every connect() call.
    /// Stale watchers (path watcher, connection closed watcher) capture their
    /// generation and bail out if the counter has advanced past theirs.
    conn_generation: AtomicU64,

    // ── Cached host identity (survives reconnect — used for UI during Reconnecting) ─
    workdir: Mutex<String>,
    hostname: Mutex<String>,
    homedir: Mutex<String>,
    project_name: Mutex<String>,
    strip_path: Mutex<String>,

    // ── Reconnect control signals ───────────────────────────────────────────
    /// Set to true by user-initiated disconnect; suppresses automatic reconnect.
    user_disconnect: AtomicBool,
    /// Set to true by foreground resume; causes the next backoff to be skipped.
    skip_next_backoff: AtomicBool,
    /// CAS guard — prevents two concurrent reconnect loops.
    reconnect_running: AtomicBool,

    // ── UI notifier ─────────────────────────────────────────────────────────
    /// Called (via push_callback) whenever session state changes.
    /// Must only capture Send types (WeakEntity / AsyncApp are !Send).
    state_notifier: Mutex<Option<Box<dyn Fn() + Send + Sync + 'static>>>,
    /// Set by HostEvent::GitChanged, consumed by UI render loop.
    git_needs_refresh: AtomicBool,
    /// Set of changed watched paths from HostEvent::FsChanged.
    fs_changed_paths: Mutex<HashSet<String>>,
    /// Optional observer RPC capability gate for cross-version fallback.
    observer_rpc_supported: AtomicBool,
}

impl SessionHandle {
    pub fn new() -> Self {
        Self(Arc::new(SessionHandleInner {
            sid: Arc::new(Mutex::new(None)),
            endpoint_addr: Mutex::new(None),
            terminals: Arc::new(Mutex::new(Vec::new())),
            signer: Mutex::new(None),
            endpoint_id: Mutex::new(None),
            pending_ticket: Mutex::new(None),
            client: Mutex::new(None),
            connect_state: Mutex::new(ConnectState::idle()),
            conn_generation: AtomicU64::new(0),
            workdir: Mutex::new(String::new()),
            homedir: Mutex::new(String::new()),
            hostname: Mutex::new(String::new()),
            project_name: Mutex::new(String::new()),
            strip_path: Mutex::new(String::new()),
            user_disconnect: AtomicBool::new(false),
            skip_next_backoff: AtomicBool::new(false),
            reconnect_running: AtomicBool::new(false),
            state_notifier: Mutex::new(None),
            git_needs_refresh: AtomicBool::new(false),
            fs_changed_paths: Mutex::new(HashSet::new()),
            observer_rpc_supported: AtomicBool::new(true),
        }))
    }

    // -----------------------------------------------------------------------
    // Credentials / identity
    // -----------------------------------------------------------------------

    pub fn set_signer(&self, signer: Arc<dyn ClientSigner>) {
        if let Ok(mut slot) = self.0.signer.lock() {
            *slot = Some(signer);
        }
    }

    pub fn signer(&self) -> Option<Arc<dyn ClientSigner>> {
        self.0.signer.lock().ok()?.clone()
    }

    pub fn set_endpoint_id(&self, id: iroh::PublicKey) {
        if let Ok(mut slot) = self.0.endpoint_id.lock() {
            *slot = Some(id);
        }
    }

    pub fn endpoint_id(&self) -> Option<iroh::PublicKey> {
        self.0.endpoint_id.lock().ok()?.clone()
    }

    pub fn set_pending_ticket(&self, ticket: zedra_rpc::ZedraPairingTicket) {
        if let Ok(mut slot) = self.0.pending_ticket.lock() {
            *slot = Some(ticket);
        }
    }

    pub fn set_endpoint_addr(&self, addr: iroh::EndpointAddr) {
        if let Ok(mut slot) = self.0.endpoint_addr.lock() {
            *slot = Some(addr);
        }
    }

    pub fn endpoint_addr(&self) -> Option<iroh::EndpointAddr> {
        self.0.endpoint_addr.lock().ok()?.clone()
    }

    pub fn session_id(&self) -> Option<String> {
        self.0.sid.lock().ok()?.clone()
    }

    pub fn set_session_id(&self, session_id: Option<String>) {
        if let Ok(mut slot) = self.0.sid.lock() {
            *slot = session_id;
        }
    }

    // -----------------------------------------------------------------------
    // Connection state (single source of truth)
    // -----------------------------------------------------------------------

    pub fn connect_state(&self) -> ConnectState {
        match self.0.connect_state.lock() {
            Ok(cs) => cs.clone(),
            Err(_) => ConnectState::idle(),
        }
    }

    /// Returns only the current phase without cloning the full snapshot.
    /// Prefer this over `connect_state().phase` when only the phase is needed.
    pub fn connect_phase(&self) -> ConnectPhase {
        match self.0.connect_state.lock() {
            Ok(cs) => cs.phase.clone(),
            Err(_) => ConnectPhase::Idle,
        }
    }

    pub fn is_connected(&self) -> bool {
        self.connect_phase().is_connected()
    }

    pub fn is_reconnecting(&self) -> bool {
        self.connect_phase().is_reconnecting()
    }

    /// User-initiated disconnect — clears session and suppresses auto-reconnect.
    pub fn clear_session(&self) {
        self.0.user_disconnect.store(true, Ordering::Release);
        if let Ok(mut slot) = self.0.client.lock() {
            *slot = None;
        }
        if let Ok(mut cs) = self.0.connect_state.lock() {
            *cs = ConnectState::idle();
        }
        self.0.reconnect_running.store(false, Ordering::Release);
        tracing::info!("SessionHandle: session cleared (user disconnect)");
    }

    // -----------------------------------------------------------------------
    // Reconnect control
    // -----------------------------------------------------------------------

    pub fn skip_next_backoff(&self) -> bool {
        self.0.skip_next_backoff.load(Ordering::Relaxed)
    }

    /// Manual retry from the UI: skip any pending backoff and re-connect immediately.
    /// Safe to call at any time; no-ops when already connected or user-disconnected.
    pub fn retry_connect(&self) {
        if self.0.user_disconnect.load(Ordering::Acquire) {
            return;
        }
        if self.endpoint_addr().is_none() {
            return;
        }
        let phase = self.connect_state().phase;
        if phase.is_connected() {
            return;
        }
        self.0.skip_next_backoff.store(true, Ordering::Release);
        if !phase.is_reconnecting() && !phase.is_connecting() {
            self.spawn_reconnect_with_reason(ReconnectReason::ConnectionLost);
        }
        // If already reconnecting, skip_next_backoff causes the loop to retry immediately.
    }

    /// Foreground resume: skip the next backoff and trigger an immediate reconnect
    /// only if the session is not already connected or in-progress.
    ///
    /// When the phase is `Connected` we leave it alone — the QUIC closed-watcher
    /// running in the background will detect a real dropout and fire
    /// `spawn_reconnect_with_reason(ConnectionLost)` on its own.  Calling
    /// reconnect unconditionally here caused a redundant reconnect cycle every
    /// time the app foregrounded even when the connection was still alive.
    pub fn notify_foreground_resume(&self) {
        if self.0.user_disconnect.load(Ordering::Acquire) {
            return;
        }
        if self.endpoint_addr().is_none() {
            return;
        }

        self.0.skip_next_backoff.store(true, Ordering::Release);

        let phase = self.connect_state().phase;

        if phase.is_connected() {
            tracing::info!(
                "foreground_resume: session Connected, skipping reconnect (closed-watcher handles dropout)"
            );
            return;
        }

        if phase.is_reconnecting() || phase.is_connecting() {
            tracing::info!(
                "foreground_resume: reconnect already in progress, flagged to skip backoff"
            );
            return;
        }

        tracing::info!("foreground_resume: session not connected, triggering immediate reconnect");
        self.spawn_reconnect_with_reason(ReconnectReason::AppForegrounded);
    }

    // -----------------------------------------------------------------------
    // State notifier (set by UI layer)
    // -----------------------------------------------------------------------

    pub fn set_state_notifier(&self, f: impl Fn() + Send + Sync + 'static) {
        if let Ok(mut slot) = self.0.state_notifier.lock() {
            *slot = Some(Box::new(f));
        }
    }

    fn notify_state_change(&self) {
        if let Ok(slot) = self.0.state_notifier.lock() {
            if let Some(f) = slot.as_ref() {
                f();
            }
        }
    }

    // -----------------------------------------------------------------------
    // Terminals
    // -----------------------------------------------------------------------

    pub fn terminal_ids(&self) -> Vec<String> {
        self.0
            .terminals
            .lock()
            .map(|terms| terms.iter().map(|t| t.id.clone()).collect())
            .unwrap_or_default()
    }

    pub fn terminal(&self, id: &str) -> Option<Arc<RemoteTerminal>> {
        self.0
            .terminals
            .lock()
            .ok()
            .and_then(|terms| terms.iter().find(|t| t.id == id).cloned())
    }

    // -----------------------------------------------------------------------
    // Cached host display fields (survive reconnect cycles)
    // -----------------------------------------------------------------------

    pub fn workdir(&self) -> String {
        self.0.workdir.lock().map(|g| g.clone()).unwrap_or_default()
    }

    pub fn homedir(&self) -> String {
        self.0.homedir.lock().map(|g| g.clone()).unwrap_or_default()
    }

    /// Working directory with home prefix stripped and replaced by `~` for display.
    pub fn strip_path(&self) -> String {
        self.0
            .strip_path
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    pub fn hostname(&self) -> String {
        self.0
            .hostname
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    /// Last path component of the remote working directory (e.g. `"zedra"`).
    pub fn project_name(&self) -> String {
        self.0
            .project_name
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    // -----------------------------------------------------------------------
    // Connect
    // -----------------------------------------------------------------------

    /// Establish a QUIC/iroh connection to `addr` and run PKI auth.
    /// Advances `connect_state.phase` through the full sequence.
    pub async fn connect(&self, addr: iroh::EndpointAddr) -> Result<()> {
        self.set_endpoint_addr(addr.clone());
        self.set_endpoint_id(addr.id);
        self.0.user_disconnect.store(false, Ordering::Release);

        let my_gen = self.0.conn_generation.fetch_add(1, Ordering::AcqRel) + 1;

        tracing::info!(
            "SessionHandle: connecting (endpoint: {}, gen: {})",
            addr.id.fmt_short(),
            my_gen,
        );

        // ── Phase: BindingEndpoint ────────────────────────────────────────
        {
            let mut cs = self.0.connect_state.lock().unwrap();
            cs.phase = ConnectPhase::BindingEndpoint;
            cs.started_at = Some(Instant::now());
            // Reset per-attempt timing and discovery fields so stale values from
            // the previous attempt don't linger in the connecting view.
            // Host identity fields (hostname, workdir, transport, session_id,
            // auth_outcome, etc.) are intentionally preserved so the workspace
            // header and session panel stay populated during auto-reconnect.
            let s = &mut cs.snapshot;
            s.binding_ms = None;
            s.hole_punch_ms = None;
            s.rpc_ms = None;
            s.register_ms = None;
            s.auth_ms = None;
            s.fetch_ms = None;
            s.resume_ms = None;
            s.local_node_id = None;
            s.remote_node_id = Some(addr.id.fmt_short().to_string());
            s.relay_url = Some(zedra_rpc::ZEDRA_RELAY_URLS[0].to_string());
            s.alpn = Some(String::from_utf8_lossy(ZEDRA_ALPN).to_string());
            s.relay_connected = false;
            s.direct_addrs.clear();
            s.has_ipv4 = false;
            s.has_ipv6 = false;
            s.mapping_varies = None;
            s.relay_latency_ms = None;
            s.captive_portal = None;
            s.failed_at_step = None;
            s.is_first_pairing = false;
            // transport, hostname, username, workdir, os, arch, os_version,
            // host_version, session_id, auth_outcome — preserved from previous session.
        }
        self.notify_state_change();

        let relay_configs: Vec<iroh::RelayConfig> = zedra_rpc::ZEDRA_RELAY_URLS
            .iter()
            .map(|u| iroh::RelayConfig {
                url: u.parse().expect("valid relay url"),
                quic: Some(iroh_relay::RelayQuicConfig::default()),
            })
            .collect();
        let relay_map = iroh::RelayMap::from_iter(relay_configs);

        // Tighten QUIC timeouts for fast disconnect detection.
        // PING frames are tiny UDP packets — cheap even on mobile.
        // Default iroh heartbeat is 5 s; we lower to 2 s so bytes_recv ticks
        // every ~2 s and the stale counter in the session tab reacts quickly.
        let transport_config = iroh::endpoint::QuicTransportConfig::builder()
            .keep_alive_interval(std::time::Duration::from_secs(2))
            .default_path_keep_alive_interval(std::time::Duration::from_secs(2))
            .default_path_max_idle_timeout(std::time::Duration::from_millis(3500))
            .max_idle_timeout(Some(
                std::time::Duration::from_secs(6)
                    .try_into()
                    .expect("6s fits in QUIC VarInt"),
            ))
            .build();

        let t0 = Instant::now();
        let endpoint = iroh::Endpoint::builder()
            .transport_config(transport_config)
            .relay_mode(iroh::RelayMode::Custom(relay_map))
            .alpns(vec![ZEDRA_ALPN.to_vec()])
            .address_lookup(iroh::address_lookup::PkarrResolver::n0_dns())
            .bind()
            .await
            .map_err(|e| {
                self.set_failed(ConnectError::EndpointBindFailed(e.to_string()));
                anyhow::anyhow!("endpoint bind failed: {e}")
            })?;

        let local_node_id = endpoint.id().fmt_short().to_string();
        tracing::info!("iroh client endpoint bound: {}", local_node_id);

        // ── Phase: HolePunching ───────────────────────────────────────────
        {
            let mut cs = self.0.connect_state.lock().unwrap();
            cs.snapshot.binding_ms = Some(t0.elapsed().as_millis() as u64);
            cs.snapshot.local_node_id = Some(local_node_id);
            cs.phase = ConnectPhase::HolePunching;
        }
        self.notify_state_change();

        // Spawn discovery watchers — update snapshot.discovery fields while
        // endpoint.connect() is in flight, so the UI shows live progress.
        let discovery_handle = self.clone();
        let discovery_endpoint = endpoint.clone();
        let discovery_task = tokio::spawn(async move {
            use iroh::Watcher;

            let mut addr_watcher = discovery_endpoint.watch_addr();
            let mut report_watcher = discovery_endpoint.net_report();

            loop {
                tokio::select! {
                    res = addr_watcher.updated() => {
                        if res.is_err() { break; }
                        let ep_addr = addr_watcher.get();
                        let has_relay = ep_addr.relay_urls().next().is_some();
                        let direct_addrs: Vec<String> =
                            ep_addr.ip_addrs().map(|addr| addr.to_string()).collect();

                        if let Ok(mut cs) = discovery_handle.0.connect_state.lock() {
                            cs.snapshot.relay_connected = has_relay;
                            cs.snapshot.direct_addrs = direct_addrs;
                        }
                        discovery_handle.notify_state_change();
                    }
                    res = report_watcher.updated() => {
                        if res.is_err() { break; }
                        if let Some(report) = report_watcher.get() {
                            let relay_lat = report.relay_latency.iter().next().map(|(_, _, lat)| {
                                lat.as_millis() as u64
                            });
                            if let Ok(mut cs) = discovery_handle.0.connect_state.lock() {
                                cs.snapshot.has_ipv4 = report.udp_v4;
                                cs.snapshot.has_ipv6 = report.udp_v6;
                                cs.snapshot.mapping_varies = report.mapping_varies_by_dest();
                                cs.snapshot.relay_latency_ms = relay_lat;
                                cs.snapshot.captive_portal = report.captive_portal;
                            }
                            discovery_handle.notify_state_change();
                        }
                    }
                }
            }
        });

        let t1 = Instant::now();
        let conn = endpoint.connect(addr, ZEDRA_ALPN).await.map_err(|e| {
            self.set_failed(ConnectError::QuicConnectFailed(e.to_string()));
            anyhow::anyhow!("quic connect failed: {e}")
        })?;

        // Stop discovery watchers — connection is established.
        discovery_task.abort();

        let remote_node_id = conn.remote_id().fmt_short().to_string();
        let alpn_str = String::from_utf8_lossy(conn.alpn()).to_string();
        tracing::info!("iroh: connected to {}", remote_node_id);

        // ── Phase: EstablishingRpc ────────────────────────────────────────
        {
            let mut cs = self.0.connect_state.lock().unwrap();
            cs.snapshot.hole_punch_ms = Some(t1.elapsed().as_millis() as u64);
            cs.snapshot.remote_node_id = Some(remote_node_id);
            cs.snapshot.alpn = Some(alpn_str);
            cs.phase = ConnectPhase::EstablishingRpc;
        }
        self.notify_state_change();

        let conn_for_paths = conn.clone();
        let conn_for_watcher = conn.clone();

        let t2 = Instant::now();
        let remote = irpc_iroh::IrohRemoteConnection::new(conn);
        let client = irpc::Client::<ZedraProto>::boxed(remote);

        {
            let mut cs = self.0.connect_state.lock().unwrap();
            cs.snapshot.rpc_ms = Some(t2.elapsed().as_millis() as u64);
        }

        if let Ok(mut slot) = self.0.client.lock() {
            *slot = Some(client.clone());
        }

        // Spawn path watcher: updates snapshot.transport live.
        //
        // Wakes on iroh path events (fires within ~3.5 s of a path going silent, when
        // iroh prunes it via PATH_MAX_IDLE_TIMEOUT) and falls back to polling at 1 s so
        // `last_alive_at` increments promptly in the session tab.
        // Reconnect is handled by max_idle_timeout (6 s) + conn.closed() below — the
        // path watcher is display-only.
        {
            use iroh::Watcher;
            let mut paths = conn_for_paths.paths();
            let handle_for_paths = self.clone();
            tokio::spawn(async move {
                let mut watcher_active = true;
                // `last_alive_at` tracks when bytes_recv last increased.
                // iroh sends PONG probes every 2 s (keep_alive_interval); if bytes_recv
                // stops growing the path is no longer receiving responses.
                // We do NOT use rtt > 0 — iroh's RTT estimator retains its last measured
                // value even after a path goes silent, which would give false "alive" reads.
                let mut last_alive_at: Option<std::time::Instant> = None;
                let mut prev_bytes_recv: u64 = 0;
                loop {
                    // Stop if the connection has been superseded.
                    if handle_for_paths.0.conn_generation.load(Ordering::Acquire) != my_gen {
                        break;
                    }

                    let mut bytes_increased = false;
                    let path_list = paths.get();
                    if let Some(path) = path_list.iter().find(|p| p.is_selected()) {
                        let stats = path.stats();
                        let is_direct = path.is_ip();
                        let (remote_addr, relay_url, network_hint) = match path.remote_addr() {
                            iroh::TransportAddr::Ip(addr) => {
                                let hint = classify_ip(addr.ip());
                                (addr.to_string(), None, Some(hint))
                            }
                            iroh::TransportAddr::Relay(url) => {
                                let host = url.host_str().unwrap_or(url.as_str()).to_string();
                                (host.clone(), Some(host), None)
                            }
                            _ => (format!("{:?}", path.remote_addr()), None, None),
                        };
                        let rtt = stats.rtt.as_millis() as u64;
                        let bytes_recv = stats.udp_rx.bytes;

                        // Update alive timestamp only when new bytes arrive.
                        // PONG responses from iroh's 2-s heartbeat probes keep this fresh.
                        if bytes_recv > prev_bytes_recv {
                            last_alive_at = Some(std::time::Instant::now());
                            prev_bytes_recv = bytes_recv;
                            bytes_increased = true;
                        }

                        let mut path_upgrade_event = None;
                        if let Ok(mut cs) = handle_for_paths.0.connect_state.lock() {
                            let prev_direct = cs
                                .snapshot
                                .transport
                                .as_ref()
                                .map(|t| t.is_direct)
                                .unwrap_or(false);
                            let path_upgraded = (!prev_direct && is_direct)
                                || cs
                                    .snapshot
                                    .transport
                                    .as_ref()
                                    .map(|t| t.path_upgraded)
                                    .unwrap_or(false);

                            path_upgrade_event = if !prev_direct && is_direct {
                                tracing::info!("iroh path upgraded: relay → direct P2P");
                                Some(PathUpgraded {
                                    network: network_hint
                                        .as_ref()
                                        .map(|h| h.label())
                                        .unwrap_or("unknown"),
                                    rtt_ms: rtt,
                                    from_relay: sanitize_relay(cs.snapshot.relay_url.as_deref()),
                                })
                            } else {
                                None
                            };

                            cs.snapshot.transport = Some(TransportSnapshot {
                                is_direct,
                                remote_addr,
                                relay_url,
                                num_paths: path_list.len(),
                                rtt_ms: rtt,
                                bytes_sent: stats.udp_tx.bytes,
                                bytes_recv,
                                path_upgraded,
                                network_hint,
                                last_alive_at,
                            });
                        }
                        if let Some(upgrade) = path_upgrade_event {
                            zedra_telemetry::send(Event::PathUpgraded(upgrade));
                        }
                    }

                    // Notify when bytes arrived (path alive) or while the stale counter
                    // is actively ticking.  Skip when idle/long-dead to avoid a continuous
                    // 1 Hz re-render drain on the workspace header badge.
                    // The session drawer has its own 2 s polling refresh for the session tab.
                    let stale_window = last_alive_at.map_or(false, |t| t.elapsed().as_secs() < 10);
                    if bytes_increased || stale_window {
                        handle_for_paths.notify_state_change();
                    }

                    if watcher_active {
                        tokio::select! {
                            res = paths.updated() => {
                                if res.is_err() {
                                    tracing::debug!("path watcher closed — polling");
                                    watcher_active = false;
                                }
                            }
                            _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {}
                        }
                    } else {
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }
                }
            });
        }

        // ── PKI auth phases (Registering → Authenticating → Proving) ─────
        let ticket = self.0.pending_ticket.lock().ok().and_then(|mut g| g.take());
        let session_id = self.session_id();
        let endpoint_id = self.endpoint_id().expect("endpoint_id stored above");

        match self.signer() {
            Some(signer) => {
                let t_auth = Instant::now();
                match self
                    .authenticate(
                        &client,
                        ticket.as_ref(),
                        signer.as_ref(),
                        &endpoint_id,
                        session_id.as_deref(),
                    )
                    .await
                {
                    Ok((sid, outcome)) => {
                        if let Ok(mut slot) = self.0.sid.lock() {
                            *slot = Some(sid.clone());
                        }
                        if let Ok(mut cs) = self.0.connect_state.lock() {
                            cs.snapshot.session_id = Some(sid);
                            cs.snapshot.auth_outcome = Some(outcome);
                            cs.snapshot.auth_ms = Some(t_auth.elapsed().as_millis() as u64);
                        }
                    }
                    Err(e) => {
                        tracing::warn!("PKI auth failed: {e}");
                        return Err(e);
                    }
                }
            }
            None => {
                tracing::warn!("No signer — skipping PKI auth (RPC calls will fail)");
            }
        }

        // ── Phase: FetchingInfo ────────────────────────────────────────────
        {
            let mut cs = self.0.connect_state.lock().unwrap();
            cs.phase = ConnectPhase::FetchingInfo;
        }
        self.notify_state_change();

        let t_fetch = Instant::now();
        self.fetch_session_info(&client).await;

        {
            let mut cs = self.0.connect_state.lock().unwrap();
            cs.snapshot.fetch_ms = Some(t_fetch.elapsed().as_millis() as u64);
            cs.phase = ConnectPhase::Connected;
            cs.reconnect_attempt = None;
        }
        self.notify_state_change();

        self.reattach_terminals(&client).await;

        // Subscribe to host-initiated events (spawned separately — must NOT be awaited
        // inline: open_bi() can stall under QUIC stream pressure).
        {
            let handle_for_sub = self.clone();
            let client_for_sub = client.clone();
            tokio::spawn(async move {
                handle_for_sub.subscribe_to_events(&client_for_sub).await;
            });
        }

        // Watch for connection close and trigger reconnect.
        let handle_for_watcher = self.clone();
        tokio::spawn(async move {
            conn_for_watcher.closed().await;
            let current_gen = handle_for_watcher.0.conn_generation.load(Ordering::Acquire);
            if current_gen != my_gen {
                tracing::debug!(
                    "stale connection closed (gen={my_gen} current={current_gen}), skipping"
                );
                return;
            }
            tracing::info!("iroh connection closed, triggering reconnect");
            handle_for_watcher.spawn_reconnect_with_reason(ReconnectReason::ConnectionLost);
        });

        tracing::info!("SessionHandle: connected (gen={my_gen})");

        // Telemetry: connect success with phase timings + transport context.
        {
            let cs = self.0.connect_state.lock().unwrap();
            let s = &cs.snapshot;
            let total_ms = cs
                .started_at
                .map(|t| t.elapsed().as_millis() as u64)
                .unwrap_or(0);
            let transport = s.transport.as_ref();
            let is_direct = transport.map(|t| t.is_direct).unwrap_or(false);
            zedra_telemetry::send(Event::ConnectSuccess(ConnectSuccess {
                total_ms,
                binding_ms: s.binding_ms.unwrap_or(0),
                hole_punch_ms: s.hole_punch_ms.unwrap_or(0),
                auth_ms: s.auth_ms.unwrap_or(0),
                fetch_ms: s.fetch_ms.unwrap_or(0),
                path: if is_direct { "direct" } else { "relay" },
                network: transport
                    .and_then(|t| t.network_hint.as_ref())
                    .map(|h| h.label())
                    .unwrap_or("unknown"),
                rtt_ms: transport.map(|t| t.rtt_ms).unwrap_or(0),
                relay: sanitize_relay(s.relay_url.as_deref()),
                relay_latency_ms: s.relay_latency_ms.unwrap_or(0),
                alpn: s.alpn.clone().unwrap_or_else(|| "unknown".into()),
                has_ipv4: s.has_ipv4,
                has_ipv6: s.has_ipv6,
                symmetric_nat: s.mapping_varies.unwrap_or(false),
                is_first_pairing: s.is_first_pairing,
            }));
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // PKI auth (Registering → Authenticating → Proving)
    // -----------------------------------------------------------------------

    async fn authenticate(
        &self,
        client: &irpc::Client<ZedraProto>,
        ticket: Option<&zedra_rpc::ZedraPairingTicket>,
        signer: &dyn ClientSigner,
        endpoint_id: &iroh::PublicKey,
        session_id: Option<&str>,
    ) -> Result<(String, AuthOutcome)> {
        use ed25519_dalek::{Verifier, VerifyingKey};
        use std::time::{SystemTime, UNIX_EPOCH};

        let client_pubkey = signer.pubkey();
        let mut outcome = AuthOutcome::Authenticated;

        // Step 1: Register (first pairing only).
        if let Some(t) = ticket {
            outcome = AuthOutcome::Registered;
            {
                let mut cs = self.0.connect_state.lock().unwrap();
                cs.phase = ConnectPhase::Registering;
                cs.snapshot.is_first_pairing = true;
            }
            self.notify_state_change();

            let t_register = Instant::now();
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let hmac = zedra_rpc::compute_registration_hmac(
                &t.handshake_secret,
                &client_pubkey,
                timestamp,
            );

            match client
                .rpc(RegisterReq {
                    client_pubkey,
                    timestamp,
                    hmac,
                    slot_session_id: t.session_id.clone(),
                })
                .await?
            {
                RegisterResult::Ok => {
                    tracing::info!("PKI: registered, session={}", t.session_id);
                    if let Ok(mut cs) = self.0.connect_state.lock() {
                        cs.snapshot.register_ms = Some(t_register.elapsed().as_millis() as u64);
                    }
                }
                RegisterResult::HandshakeConsumed => {
                    self.set_failed(ConnectError::HandshakeConsumed);
                    return Err(anyhow::anyhow!("register: HandshakeConsumed"));
                }
                RegisterResult::InvalidHandshake => {
                    self.set_failed(ConnectError::InvalidHandshake);
                    return Err(anyhow::anyhow!("register: InvalidHandshake"));
                }
                RegisterResult::StaleTimestamp => {
                    self.set_failed(ConnectError::StaleTimestamp);
                    return Err(anyhow::anyhow!("register: StaleTimestamp"));
                }
                RegisterResult::SlotNotFound => {
                    self.set_failed(ConnectError::SlotNotFound);
                    return Err(anyhow::anyhow!("register: SlotNotFound"));
                }
            }
        }

        // Step 2: Authenticate — request nonce + verify host signature.
        {
            let mut cs = self.0.connect_state.lock().unwrap();
            cs.phase = ConnectPhase::Authenticating;
        }
        self.notify_state_change();

        let challenge: AuthChallengeResult = client.rpc(AuthReq { client_pubkey }).await?;
        {
            let vk_bytes = endpoint_id.as_bytes();
            let vk = VerifyingKey::from_bytes(vk_bytes)
                .map_err(|e| anyhow::anyhow!("invalid host pubkey: {e}"))?;
            let sig = ed25519_dalek::Signature::from_bytes(&challenge.host_signature);
            vk.verify(&challenge.nonce, &sig).map_err(|_| {
                self.set_failed(ConnectError::HostSignatureInvalid);
                anyhow::anyhow!("host challenge signature invalid")
            })?;
        }

        // Step 3: Prove identity.
        {
            let mut cs = self.0.connect_state.lock().unwrap();
            cs.phase = ConnectPhase::Proving;
        }
        self.notify_state_change();

        let client_signature = signer.sign(&challenge.nonce);
        let attach_session_id = ticket
            .map(|t| t.session_id.clone())
            .or_else(|| session_id.map(|s| s.to_string()))
            .unwrap_or_default();

        match client
            .rpc(AuthProveReq {
                nonce: challenge.nonce,
                client_signature,
                session_id: attach_session_id.clone(),
            })
            .await?
        {
            AuthProveResult::Ok => {
                tracing::info!("PKI: authenticated, session={}", attach_session_id);
                Ok((attach_session_id, outcome))
            }
            AuthProveResult::Unauthorized => {
                self.set_failed(ConnectError::Unauthorized);
                Err(anyhow::anyhow!("auth prove: Unauthorized"))
            }
            AuthProveResult::NotInSessionAcl => {
                self.set_failed(ConnectError::NotInSessionAcl);
                Err(anyhow::anyhow!("auth prove: NotInSessionAcl"))
            }
            AuthProveResult::SessionOccupied => {
                self.set_failed(ConnectError::SessionOccupied);
                Err(anyhow::anyhow!("auth prove: SessionOccupied"))
            }
            AuthProveResult::SessionNotFound => {
                self.set_failed(ConnectError::SessionNotFound);
                Err(anyhow::anyhow!("auth prove: SessionNotFound"))
            }
            AuthProveResult::InvalidSignature => {
                self.set_failed(ConnectError::InvalidSignature);
                Err(anyhow::anyhow!("auth prove: InvalidSignature"))
            }
        }
    }

    async fn fetch_session_info(&self, client: &irpc::Client<ZedraProto>) {
        match client.rpc(SessionInfoReq {}).await {
            Ok(info) => {
                tracing::info!(
                    "Session info: host={}, user={}, workdir={}",
                    info.hostname,
                    info.username,
                    info.workdir,
                );

                // Update snapshot.
                if let Ok(mut cs) = self.0.connect_state.lock() {
                    cs.snapshot.hostname = Some(info.hostname.clone());
                    cs.snapshot.username = Some(info.username.clone());
                    cs.snapshot.workdir = Some(info.workdir.clone());
                    cs.snapshot.os = info.os.clone();
                    cs.snapshot.arch = info.arch.clone();
                    cs.snapshot.os_version = info.os_version.clone();
                    cs.snapshot.host_version = info.host_version.clone();
                }

                // Update cached fields (survive reconnect cycles for header display).
                if !info.hostname.is_empty() {
                    if let Ok(mut h) = self.0.hostname.lock() {
                        *h = info.hostname.clone();
                    }
                }
                if !info.workdir.is_empty() {
                    let workdir = info.workdir.clone();
                    let project = workdir.rsplit('/').next().unwrap_or(&workdir).to_string();

                    // Cache raw workdir and derived project name.
                    if let Ok(mut w) = self.0.workdir.lock() {
                        *w = workdir.clone();
                    }
                    if let Ok(mut p) = self.0.project_name.lock() {
                        *p = project;
                    }

                    // Cache home directory and a `~`-stripped display path.
                    let home_dir = info.home_dir.clone().unwrap_or_default();
                    if !home_dir.is_empty() {
                        if let Ok(mut h) = self.0.homedir.lock() {
                            *h = home_dir.clone();
                        }
                    }
                    let display_path = if !home_dir.is_empty() {
                        if let Some(rest) = workdir.strip_prefix(&home_dir) {
                            format!("~{rest}")
                        } else {
                            workdir
                        }
                    } else {
                        workdir
                    };
                    if let Ok(mut s) = self.0.strip_path.lock() {
                        *s = display_path;
                    }
                }

                // Store session ID if the server provided one and we don't have one yet.
                if let Some(sid) = info.session_id {
                    if let Ok(mut slot) = self.0.sid.lock() {
                        if slot.is_none() {
                            *slot = Some(sid.clone());
                        }
                    }
                    if let Ok(mut cs) = self.0.connect_state.lock() {
                        if cs.snapshot.session_id.is_none() {
                            cs.snapshot.session_id = Some(sid);
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!("session/info failed: {e}");
            }
        }
    }

    // -----------------------------------------------------------------------
    // Terminal attachment
    // -----------------------------------------------------------------------

    async fn attach_terminal(
        &self,
        client: &irpc::Client<ZedraProto>,
        terminal: &Arc<RemoteTerminal>,
    ) -> Result<()> {
        let (irpc_input_tx, mut irpc_output_rx) = client
            .bidi_streaming::<TermAttachReq, TermInput, TermOutput>(
                TermAttachReq {
                    id: terminal.id.clone(),
                    last_seq: terminal.last_seq(),
                },
                256,
                256,
            )
            .await?;

        let (bridge_tx, mut bridge_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(256);
        terminal.set_input_tx(bridge_tx);

        tokio::spawn(async move {
            while let Some(data) = bridge_rx.recv().await {
                if let Err(e) = irpc_input_tx.send(TermInput { data }).await {
                    tracing::debug!("terminal input bridge closed: {e}");
                    break;
                }
            }
        });

        let terminal_id = terminal.id.clone();
        let last_seq = terminal.last_seq();
        let terminal_pump = terminal.clone();
        tokio::spawn(async move {
            let mut first_msg = true;
            loop {
                match irpc_output_rx.recv().await {
                    Ok(Some(output)) => {
                        // seq=0 is a synthetic metadata preamble injected by the
                        // host on TermAttach to seed title/CWD from its OSC cache.
                        // Pass the bytes through for OSC parsing but skip seq
                        // tracking and the backlog-gap check so this doesn't
                        // interfere with the real output stream.
                        if output.seq == 0 {
                            terminal_pump.push_output(output.data);
                            terminal_pump.signal_needs_render();
                            continue;
                        }

                        if first_msg {
                            first_msg = false;
                            if last_seq > 0 && output.seq > last_seq + 1 {
                                tracing::warn!(
                                    "terminal {}: backlog gap (last_seq={} first_recv_seq={}), injecting reset",
                                    terminal_pump.id,
                                    last_seq,
                                    output.seq,
                                );
                                // RIS resets the visual state; also reset the OSC scanner
                                // so a partial sequence from before the gap doesn't corrupt
                                // the next sequence.
                                terminal_pump.reset_osc_scanner();
                                terminal_pump.push_output(b"\x1bc".to_vec());
                            }
                        }
                        terminal_pump.update_seq(output.seq);
                        terminal_pump.push_output(output.data);
                        terminal_pump.signal_needs_render();
                    }
                    Ok(None) => {
                        tracing::debug!("terminal {} output stream ended", terminal_pump.id);
                        break;
                    }
                    Err(e) => {
                        tracing::debug!("terminal {} output recv error: {e}", terminal_pump.id);
                        break;
                    }
                }
            }
        });

        tracing::info!("Terminal {} attached (last_seq={})", terminal_id, last_seq);
        Ok(())
    }

    async fn reattach_terminals(&self, client: &irpc::Client<ZedraProto>) {
        let terminals = self
            .0
            .terminals
            .lock()
            .map(|v| v.clone())
            .unwrap_or_default();
        if terminals.is_empty() {
            return;
        }
        tracing::info!("reattaching {} terminals", terminals.len());
        if let Ok(mut cs) = self.0.connect_state.lock() {
            cs.phase = ConnectPhase::ResumingTerminals;
        }
        self.notify_state_change();
        let t = Instant::now();
        for terminal in &terminals {
            if let Err(e) = self.attach_terminal(client, terminal).await {
                tracing::warn!("failed to reattach terminal {}: {e}", terminal.id);
            }
        }
        if let Ok(mut cs) = self.0.connect_state.lock() {
            cs.phase = ConnectPhase::Connected;
            cs.snapshot.resume_ms = Some(t.elapsed().as_millis() as u64);
        }
        self.notify_state_change();
    }

    /// Set phase to ResumingTerminals — call from app before manually attaching
    /// existing server-side terminals on initial session resume.
    pub fn set_resuming_terminals(&self) {
        if let Ok(mut cs) = self.0.connect_state.lock() {
            cs.phase = ConnectPhase::ResumingTerminals;
        }
        self.notify_state_change();
    }

    /// Set phase back to Connected with resume timing — call from app after
    /// all terminal attach_existing calls complete.
    pub fn mark_connected_after_resume(&self, elapsed_ms: u64) {
        if let Ok(mut cs) = self.0.connect_state.lock() {
            cs.phase = ConnectPhase::Connected;
            cs.snapshot.resume_ms = Some(elapsed_ms);
        }
        self.notify_state_change();
    }

    async fn subscribe_to_events(&self, client: &irpc::Client<ZedraProto>) {
        let stream_fut = client.server_streaming::<SubscribeReq, HostEvent>(SubscribeReq {}, 32);

        let mut rx =
            match tokio::time::timeout(std::time::Duration::from_secs(10), stream_fut).await {
                Ok(Ok(rx)) => rx,
                Ok(Err(e)) => {
                    tracing::warn!("Subscribe failed: {e}");
                    return;
                }
                Err(_) => {
                    tracing::warn!("Subscribe timed out");
                    return;
                }
            };

        let handle = self.clone();
        let client = client.clone();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(Some(event)) => handle.handle_host_event(event, &client).await,
                    Ok(None) => {
                        tracing::debug!("Subscribe stream ended");
                        break;
                    }
                    Err(e) => {
                        tracing::debug!("Subscribe recv error: {e}");
                        break;
                    }
                }
            }
        });

        tracing::info!("Subscribed to host events");
    }

    async fn handle_host_event(&self, event: HostEvent, client: &irpc::Client<ZedraProto>) {
        match event {
            HostEvent::TerminalCreated { id, launch_cmd } => {
                tracing::info!(
                    "HostEvent: terminal created id={} launch_cmd={:?}",
                    id,
                    launch_cmd,
                );
                let terminal = RemoteTerminal::new(id);
                self.0.terminals.lock().unwrap().push(terminal.clone());
                if let Err(e) = self.attach_terminal(client, &terminal).await {
                    tracing::warn!(
                        "Failed to attach host-created terminal {}: {e}",
                        terminal.id
                    );
                }
                self.notify_state_change();
            }
            HostEvent::GitChanged => {
                self.0.git_needs_refresh.store(true, Ordering::Release);
                push_callback(Box::new(|| {}));
            }
            HostEvent::FsChanged { path } => {
                if let Ok(mut changed) = self.0.fs_changed_paths.lock() {
                    changed.insert(path);
                }
                push_callback(Box::new(|| {}));
            }
        }
    }

    // -----------------------------------------------------------------------
    // Reconnect loop
    // -----------------------------------------------------------------------

    /// Spawn a reconnect loop assuming `ConnectionLost`.
    pub fn spawn_reconnect(&self) {
        self.spawn_reconnect_with_reason(ReconnectReason::ConnectionLost);
    }

    fn spawn_reconnect_with_reason(&self, initial_reason: ReconnectReason) {
        if self.0.user_disconnect.load(Ordering::Acquire) {
            tracing::info!("spawn_reconnect: skipping, user disconnect");
            return;
        }
        if self
            .0
            .reconnect_running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
            .is_err()
        {
            tracing::info!("spawn_reconnect: already running");
            return;
        }

        let handle = self.clone();
        session_runtime().spawn(async move {
            let max_attempts = 10u32;
            let mut attempt = 1u32;
            let mut reason = initial_reason;
            let reconnect_start = std::time::Instant::now();
            let reason_label = match &reason {
                ReconnectReason::ConnectionLost => "connection_lost",
                ReconnectReason::AppForegrounded => "app_foregrounded",
            };
            zedra_telemetry::send(Event::ReconnectStarted(ReconnectStarted {
                reason: reason_label,
            }));

            'reconnect: loop {
                if handle.0.user_disconnect.load(Ordering::Acquire) {
                    break;
                }
                if attempt > max_attempts {
                    break;
                }

                let delay_secs = std::cmp::min(1u64 << (attempt - 1), 30);
                let skip_backoff = handle.0.skip_next_backoff.swap(false, Ordering::AcqRel);
                let next_retry_secs = if skip_backoff { 0 } else { delay_secs };

                // Set Reconnecting phase.
                {
                    let mut cs = handle.0.connect_state.lock().unwrap();
                    cs.phase = ConnectPhase::Reconnecting {
                        attempt,
                        reason: reason.clone(),
                        next_retry_secs,
                    };
                    // Persist the attempt number across the Reconnecting →
                    // BindingEndpoint → … phase transitions so the transport
                    // badge can show "Retry N" during the actual attempt phases.
                    cs.reconnect_attempt = Some(attempt);
                }
                handle.notify_state_change();

                // Countdown.
                if !skip_backoff && delay_secs > 0 {
                    tracing::info!(
                        "reconnect: attempt {}/{} backoff {}s",
                        attempt,
                        max_attempts,
                        delay_secs,
                    );
                    for remaining in (1..=delay_secs).rev() {
                        if let Ok(mut cs) = handle.0.connect_state.lock() {
                            cs.phase = ConnectPhase::Reconnecting {
                                attempt,
                                reason: reason.clone(),
                                next_retry_secs: remaining,
                            };
                        }
                        handle.notify_state_change();
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        if handle.0.user_disconnect.load(Ordering::Acquire) {
                            break 'reconnect;
                        }
                    }
                } else {
                    tracing::info!(
                        "reconnect: attempt {}/{} (immediate)",
                        attempt,
                        max_attempts,
                    );
                }

                if handle.0.user_disconnect.load(Ordering::Acquire) {
                    break;
                }

                let addr = match handle.endpoint_addr() {
                    Some(a) => a,
                    None => {
                        tracing::error!("reconnect: no stored endpoint address, aborting");
                        break;
                    }
                };

                match handle.connect(addr).await {
                    Ok(()) => {
                        tracing::info!("reconnect: success on attempt {}", attempt);
                        zedra_telemetry::send(Event::ReconnectSuccess(ReconnectSuccess {
                            attempt,
                            elapsed_ms: reconnect_start.elapsed().as_millis() as u64,
                            reason: reason_label,
                        }));
                        match handle.terminal_list().await {
                            Ok(ids) => tracing::info!(
                                "reconnect: server has {} terminals: {:?}",
                                ids.len(),
                                ids,
                            ),
                            Err(e) => tracing::warn!("reconnect: terminal_list failed: {e}"),
                        }
                        // Phase is now Connected (set inside connect()).
                        break;
                    }
                    Err(e) => {
                        tracing::warn!("reconnect: attempt {} failed: {e}", attempt);
                        // Fatal auth errors — stop retrying.
                        if let ConnectPhase::Failed(ref err) = handle.connect_state().phase {
                            if err.is_fatal() {
                                break;
                            }
                        }
                        attempt += 1;
                        reason = ReconnectReason::ConnectionLost;
                    }
                }
            }

            // Post-loop: if we exhausted attempts without connecting or hitting a
            // fatal error, mark host as unreachable.
            if !handle.0.user_disconnect.load(Ordering::Acquire) {
                let phase = handle.connect_state().phase;
                let is_fatal = matches!(&phase, ConnectPhase::Failed(err) if err.is_fatal());
                if !phase.is_connected() && !is_fatal {
                    zedra_telemetry::send(Event::ReconnectExhausted(ReconnectExhausted {
                        attempts: max_attempts,
                        elapsed_ms: reconnect_start.elapsed().as_millis() as u64,
                        reason: reason_label,
                    }));
                    if let Ok(mut cs) = handle.0.connect_state.lock() {
                        cs.phase = ConnectPhase::Failed(ConnectError::HostUnreachable);
                    }
                    handle.notify_state_change();
                }
            }

            handle.0.reconnect_running.store(false, Ordering::Release);
        });
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn set_failed(&self, error: ConnectError) {
        let phase_label;
        let relay;
        let alpn;
        let has_ipv4;
        let has_ipv6;
        let relay_connected;
        if let Ok(mut cs) = self.0.connect_state.lock() {
            phase_label = cs.phase.label();
            relay = sanitize_relay(cs.snapshot.relay_url.as_deref());
            alpn = cs.snapshot.alpn.clone().unwrap_or_else(|| "unknown".into());
            has_ipv4 = cs.snapshot.has_ipv4;
            has_ipv6 = cs.snapshot.has_ipv6;
            relay_connected = cs.snapshot.relay_connected;
            cs.snapshot.failed_at_step = cs.phase.step_index();
            cs.phase = ConnectPhase::Failed(error.clone());
        } else {
            phase_label = "unknown";
            relay = "none".into();
            alpn = "unknown".into();
            has_ipv4 = false;
            has_ipv6 = false;
            relay_connected = false;
        }
        zedra_telemetry::send(Event::ConnectFailed(ConnectFailed {
            phase: phase_label,
            error: error.label(),
            relay,
            alpn,
            has_ipv4,
            has_ipv6,
            relay_connected,
        }));
        self.notify_state_change();
    }

    fn client(&self) -> Result<irpc::Client<ZedraProto>> {
        self.0
            .client
            .lock()
            .ok()
            .and_then(|g| g.clone())
            .ok_or_else(|| anyhow::anyhow!("not connected"))
    }

    // -----------------------------------------------------------------------
    // RPC: ping
    // -----------------------------------------------------------------------

    pub async fn ping(&self) -> Result<u64> {
        use std::time::{SystemTime, UNIX_EPOCH};
        let client = self.client()?;
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let result: PongResult = client
            .rpc(PingReq {
                timestamp_ms: now_ms,
            })
            .await?;
        let after_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let rtt_ms = after_ms.saturating_sub(result.timestamp_ms);
        if let Ok(mut cs) = self.0.connect_state.lock() {
            if let Some(ref mut t) = cs.snapshot.transport {
                t.rtt_ms = rtt_ms;
            }
        }
        Ok(rtt_ms)
    }

    // -----------------------------------------------------------------------
    // RPC: filesystem
    // -----------------------------------------------------------------------

    /// Fetch the first page of a directory listing.
    /// Returns `(entries, total, has_more)`.
    pub async fn fs_list(&self, path: &str) -> Result<(Vec<FsEntry>, u32, bool)> {
        self.fs_list_page(path, 0, FS_LIST_DEFAULT_LIMIT).await
    }

    /// Fetch a specific page of a directory listing.
    /// Returns `(entries, total, has_more)`.
    pub async fn fs_list_page(
        &self,
        path: &str,
        offset: u32,
        limit: u32,
    ) -> Result<(Vec<FsEntry>, u32, bool)> {
        let result: FsListResult = self
            .client()?
            .rpc(FsListReq {
                path: path.to_string(),
                offset,
                limit,
            })
            .await?;
        Ok((result.entries, result.total, result.has_more))
    }

    pub async fn fs_read(&self, path: &str) -> Result<FsReadResult> {
        let result: FsReadResult = self
            .client()?
            .rpc(FsReadReq {
                path: path.to_string(),
            })
            .await?;
        Ok(result)
    }

    pub async fn fs_write(&self, path: &str, content: &str) -> Result<()> {
        let _: FsWriteResult = self
            .client()?
            .rpc(FsWriteReq {
                path: path.to_string(),
                content: content.to_string(),
            })
            .await?;
        Ok(())
    }

    pub async fn fs_stat(&self, path: &str) -> Result<FsStatResult> {
        Ok(self
            .client()?
            .rpc(FsStatReq {
                path: path.to_string(),
            })
            .await?)
    }

    pub async fn fs_watch(&self, path: &str) -> Result<FsWatchResult> {
        if !self.0.observer_rpc_supported.load(Ordering::Acquire) {
            return Ok(FsWatchResult::Unsupported);
        }
        let result: std::result::Result<FsWatchResult, _> = self
            .client()?
            .rpc(FsWatchReq {
                path: path.to_string(),
            })
            .await;
        match result {
            Ok(result) => Ok(result),
            Err(e) => {
                if self.downgrade_observer_rpc_if_incompatible(&e.to_string()) {
                    return Ok(FsWatchResult::Unsupported);
                }
                Err(anyhow::anyhow!(e.to_string()))
            }
        }
    }

    pub async fn fs_unwatch(&self, path: &str) -> Result<FsUnwatchResult> {
        if !self.0.observer_rpc_supported.load(Ordering::Acquire) {
            return Ok(FsUnwatchResult::Unsupported);
        }
        let result: std::result::Result<FsUnwatchResult, _> = self
            .client()?
            .rpc(FsUnwatchReq {
                path: path.to_string(),
            })
            .await;
        match result {
            Ok(result) => Ok(result),
            Err(e) => {
                if self.downgrade_observer_rpc_if_incompatible(&e.to_string()) {
                    return Ok(FsUnwatchResult::Unsupported);
                }
                Err(anyhow::anyhow!(e.to_string()))
            }
        }
    }

    fn downgrade_observer_rpc_if_incompatible(&self, err_msg: &str) -> bool {
        let msg = err_msg.to_lowercase();
        let incompatible = msg.contains("unknown variant")
            || msg.contains("deserialize")
            || msg.contains("decode")
            || msg.contains("invalid type");
        if incompatible {
            self.0
                .observer_rpc_supported
                .store(false, Ordering::Release);
            tracing::warn!(
                "observer RPC unsupported by host, disabling fs watch/unwatch for this session: {}",
                err_msg
            );
        }
        incompatible
    }

    pub fn take_git_refresh(&self) -> bool {
        self.0.git_needs_refresh.swap(false, Ordering::AcqRel)
    }

    pub fn take_fs_changed(&self) -> Vec<String> {
        match self.0.fs_changed_paths.lock() {
            Ok(mut changed) => changed.drain().collect(),
            Err(_) => Vec::new(),
        }
    }

    // -----------------------------------------------------------------------
    // RPC: git
    // -----------------------------------------------------------------------

    pub async fn git_status(&self) -> Result<GitStatusResult> {
        Ok(self.client()?.rpc(GitStatusReq {}).await?)
    }

    pub async fn git_diff(&self, path: Option<&str>, staged: bool) -> Result<String> {
        let result: GitDiffResult = self
            .client()?
            .rpc(GitDiffReq {
                path: path.map(|s| s.to_string()),
                staged,
            })
            .await?;
        Ok(result.diff)
    }

    pub async fn git_log(&self, limit: Option<usize>) -> Result<Vec<GitLogEntry>> {
        let result: GitLogResult = self.client()?.rpc(GitLogReq { limit }).await?;
        Ok(result.entries)
    }

    pub async fn git_branches(&self) -> Result<Vec<GitBranchEntry>> {
        let result: GitBranchesResult = self.client()?.rpc(GitBranchesReq {}).await?;
        Ok(result.branches)
    }

    pub async fn git_checkout(&self, branch: &str) -> Result<()> {
        let _: GitCheckoutResult = self
            .client()?
            .rpc(GitCheckoutReq {
                branch: branch.to_string(),
            })
            .await?;
        Ok(())
    }

    pub async fn git_commit(&self, message: &str, paths: &[String]) -> Result<String> {
        let result: GitCommitResult = self
            .client()?
            .rpc(GitCommitReq {
                message: message.to_string(),
                paths: paths.to_vec(),
            })
            .await?;
        Ok(result.hash)
    }

    pub async fn git_stage(&self, paths: &[String]) -> Result<()> {
        let _: GitStageResult = self
            .client()?
            .rpc(GitStageReq {
                paths: paths.to_vec(),
            })
            .await?;
        Ok(())
    }

    pub async fn git_unstage(&self, paths: &[String]) -> Result<()> {
        let _: GitUnstageResult = self
            .client()?
            .rpc(GitUnstageReq {
                paths: paths.to_vec(),
            })
            .await?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // RPC: terminals
    // -----------------------------------------------------------------------

    pub async fn terminal_create(&self, cols: u16, rows: u16) -> Result<String> {
        self.terminal_create_with_cmd(cols, rows, None).await
    }

    pub async fn terminal_create_with_cmd(
        &self,
        cols: u16,
        rows: u16,
        launch_cmd: Option<String>,
    ) -> Result<String> {
        let client = self.client()?;
        let result: TermCreateResult = client
            .rpc(TermCreateReq {
                cols,
                rows,
                launch_cmd,
            })
            .await?;

        let terminal = RemoteTerminal::new(result.id.clone());
        self.0.terminals.lock().unwrap().push(terminal.clone());
        self.attach_terminal(&client, &terminal).await?;

        tracing::info!("Terminal created: {}", result.id);
        Ok(result.id)
    }

    pub async fn terminal_resize(&self, id: &str, cols: u16, rows: u16) -> Result<()> {
        let _: TermResizeResult = self
            .client()?
            .rpc(TermResizeReq {
                id: id.to_string(),
                cols,
                rows,
            })
            .await?;
        Ok(())
    }

    pub async fn terminal_close(&self, id: &str) -> Result<()> {
        let _: TermCloseResult = self
            .client()?
            .rpc(TermCloseReq { id: id.to_string() })
            .await?;
        self.0.terminals.lock().unwrap().retain(|t| t.id != id);
        Ok(())
    }

    /// Attach to an existing server-side terminal (session resume). Creates a
    /// `RemoteTerminal` with the given ID and starts the output pump.
    pub async fn terminal_attach_existing(&self, id: &str) -> Result<()> {
        let client = self.client()?;
        let terminal = RemoteTerminal::new(id.to_string());
        self.0.terminals.lock().unwrap().push(terminal.clone());
        self.attach_terminal(&client, &terminal).await?;
        tracing::info!("Terminal attached (resume): {}", id);
        Ok(())
    }

    pub async fn terminal_list(&self) -> Result<Vec<String>> {
        let result: TermListResult = self.client()?.rpc(TermListReq {}).await?;
        Ok(result.terminals.into_iter().map(|e| e.id).collect())
    }
}
