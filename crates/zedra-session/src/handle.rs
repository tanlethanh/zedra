use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
};

use anyhow::Result;
use zedra_rpc::proto::*;

use crate::{session_runtime, signer::ClientSigner, terminal::RemoteTerminal};

/// Why a reconnect was triggered.
#[derive(Clone, Debug, PartialEq)]
pub enum ReconnectReason {
    /// QUIC connection closed (transport failure, timeout).
    ConnectionLost,
    /// App returned to foreground after iOS suspension.
    AppForegrounded,
}

/// Per-connection state of the session.
#[derive(Clone, Debug)]
pub enum SessionState {
    Disconnected,
    Connecting {
        phase: String,
    },
    Connected {
        hostname: String,
        username: String,
        workdir: String,
        os: String,
        arch: String,
        os_version: String,
        host_version: String,
    },
    Reconnecting {
        attempt: u32,
        reason: ReconnectReason,
        next_retry_secs: u64,
    },
    /// All reconnect attempts exhausted; user must retry or re-scan QR.
    HostUnreachable,
    Error(String),
}

/// Active iroh transport path snapshot, refreshed by the path watcher task.
#[derive(Clone, Debug)]
pub struct ConnectionInfo {
    pub is_direct: bool,
    pub remote_addr: String,
    pub relay_url: Option<String>,
    pub endpoint_id: String,
    pub local_endpoint_id: String,
    pub num_paths: usize,
    pub protocol: String,
    pub path_rtt_ms: u64,
    pub bytes_sent: u64,
    pub bytes_recv: u64,
}

/// Workspace-scoped durable session state; owns credentials, terminals, reconnect
/// control, and the live RPC client. Arc-wrapped — clone freely to share with async tasks.
#[derive(Clone)]
pub struct SessionHandle(Arc<SessionHandleInner>);

struct SessionHandleInner {
    // Durable credentials / identity
    sid: Arc<Mutex<Option<String>>>,
    endpoint_addr: Mutex<Option<iroh::EndpointAddr>>,
    terminals: Arc<Mutex<Vec<Arc<RemoteTerminal>>>>,
    signer: Mutex<Option<Arc<dyn ClientSigner>>>,
    endpoint_id: Mutex<Option<iroh::PublicKey>>,
    pending_ticket: Mutex<Option<zedra_rpc::ZedraPairingTicket>>,

    // Connection state (replaced on each new connect)
    client: Mutex<Option<irpc::Client<ZedraProto>>>,
    session_state: Mutex<SessionState>,
    connection_info: Arc<Mutex<Option<ConnectionInfo>>>,
    latency_ms: Arc<AtomicU64>,

    // Reconnect control
    reconnect_attempt: AtomicU32,
    reconnect_reason: Mutex<ReconnectReason>,
    user_disconnect: AtomicBool,
    skip_next_backoff: AtomicBool,
    next_retry_secs: AtomicU64,
    host_unreachable: AtomicBool,

    // Notifier set by the UI layer; called whenever session state changes.
    // Must only capture Send types (WeakEntity and AsyncApp are !Send).
    state_notifier: Mutex<Option<Box<dyn Fn() + Send + Sync + 'static>>>,
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
            session_state: Mutex::new(SessionState::Disconnected),
            connection_info: Arc::new(Mutex::new(None)),
            latency_ms: Arc::new(AtomicU64::new(0)),
            reconnect_attempt: AtomicU32::new(0),
            reconnect_reason: Mutex::new(ReconnectReason::ConnectionLost),
            user_disconnect: AtomicBool::new(false),
            skip_next_backoff: AtomicBool::new(false),
            next_retry_secs: AtomicU64::new(0),
            host_unreachable: AtomicBool::new(false),
            state_notifier: Mutex::new(None),
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

    pub fn is_host_unreachable(&self) -> bool {
        self.0.host_unreachable.load(Ordering::Relaxed)
    }

    // -----------------------------------------------------------------------
    // Session state
    // -----------------------------------------------------------------------

    /// Effective state for the UI. Reconnect atomics take priority.
    pub fn state(&self) -> SessionState {
        let attempt = self.reconnect_attempt();
        if attempt > 0 {
            return SessionState::Reconnecting {
                attempt,
                reason: self.reconnect_reason(),
                next_retry_secs: self.next_retry_secs(),
            };
        }
        if self.is_host_unreachable() {
            return SessionState::HostUnreachable;
        }
        self.0
            .session_state
            .lock()
            .map(|s| s.clone())
            .unwrap_or(SessionState::Disconnected)
    }

    pub fn connection_info(&self) -> Option<ConnectionInfo> {
        self.0.connection_info.lock().ok().and_then(|g| g.clone())
    }

    pub fn latency_ms(&self) -> u64 {
        self.0.latency_ms.load(Ordering::Relaxed)
    }

    /// User-initiated disconnect; suppresses automatic reconnect.
    pub fn clear_session(&self) {
        self.0.user_disconnect.store(true, Ordering::Release);
        if let Ok(mut slot) = self.0.client.lock() {
            *slot = None;
        }
        if let Ok(mut s) = self.0.session_state.lock() {
            *s = SessionState::Disconnected;
        }
        tracing::info!("SessionHandle: session cleared (user disconnect)");
    }

    pub fn is_connected(&self) -> bool {
        matches!(self.state(), SessionState::Connected { .. })
    }

    // -----------------------------------------------------------------------
    // Reconnect control
    // -----------------------------------------------------------------------

    pub fn is_reconnecting(&self) -> bool {
        self.reconnect_attempt() > 0
    }

    pub fn reconnect_attempt(&self) -> u32 {
        self.0.reconnect_attempt.load(Ordering::Relaxed)
    }

    pub fn set_reconnect_attempt(&self, attempt: u32) {
        self.0.reconnect_attempt.store(attempt, Ordering::Release);
    }

    pub fn reconnect_reason(&self) -> ReconnectReason {
        self.0
            .reconnect_reason
            .lock()
            .map(|r| r.clone())
            .unwrap_or(ReconnectReason::ConnectionLost)
    }

    pub fn set_reconnect_reason(&self, reason: ReconnectReason) {
        if let Ok(mut slot) = self.0.reconnect_reason.lock() {
            *slot = reason;
        }
    }

    pub fn next_retry_secs(&self) -> u64 {
        self.0.next_retry_secs.load(Ordering::Relaxed)
    }

    pub fn skip_next_backoff(&self) -> bool {
        self.0.skip_next_backoff.load(Ordering::Relaxed)
    }

    /// iOS foreground resume: skip the next backoff and trigger an immediate reconnect.
    pub fn notify_foreground_resume(&self) {
        if self.0.user_disconnect.load(Ordering::Acquire) {
            return;
        }

        if self.endpoint_addr().is_none() {
            return;
        }

        self.set_reconnect_reason(ReconnectReason::AppForegrounded);
        self.0.skip_next_backoff.store(true, Ordering::Release);

        if self.is_reconnecting() {
            tracing::info!("foreground_resume: reconnect in progress, flagged to skip backoff");
            return;
        }

        tracing::info!("foreground_resume: triggering immediate reconnect");
        self.spawn_reconnect();
    }

    // -----------------------------------------------------------------------
    // State notifier (set by UI layer)
    // -----------------------------------------------------------------------

    /// Register a callback to be invoked (via push_callback) whenever session
    /// state changes. The closure must be Send + Sync — it cannot capture GPUI
    /// handles (WeakEntity / AsyncApp are !Send). Typically it just calls
    /// `push_callback(Box::new(|| {}))` to force a re-render.
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
    // Connect / reconnect
    // -----------------------------------------------------------------------

    /// Establish a QUIC/iroh connection to `addr` and run PKI auth.
    /// Updates internal state (session_state, client, terminals) in place.
    pub async fn connect(&self, addr: iroh::EndpointAddr) -> Result<()> {
        self.set_endpoint_addr(addr.clone());
        self.set_endpoint_id(addr.id);
        self.0.user_disconnect.store(false, Ordering::Release);

        tracing::info!(
            "SessionHandle: connecting via iroh (endpoint: {})",
            addr.id.fmt_short(),
        );

        if let Ok(mut s) = self.0.session_state.lock() {
            *s = SessionState::Connecting {
                phase: "Creating endpoint".into(),
            };
        }
        self.notify_state_change();

        let relay_url: iroh::RelayUrl =
            zedra_rpc::ZEDRA_RELAY_URL.parse().expect("valid relay url");
        let relay_map = iroh::RelayMap::from_iter([iroh::RelayConfig {
            url: relay_url,
            quic: Some(iroh_relay::RelayQuicConfig::default()),
        }]);
        let endpoint = iroh::Endpoint::builder()
            .relay_mode(iroh::RelayMode::Custom(relay_map))
            .alpns(vec![ZEDRA_ALPN.to_vec()])
            .address_lookup(iroh::address_lookup::PkarrResolver::n0_dns())
            .bind()
            .await?;
        tracing::info!("iroh client endpoint bound: {}", endpoint.id().fmt_short());

        if let Ok(mut s) = self.0.session_state.lock() {
            *s = SessionState::Connecting {
                phase: "QUIC handshake (direct P2P)".into(),
            };
        }
        self.notify_state_change();

        let conn = endpoint.connect(addr, ZEDRA_ALPN).await?;
        tracing::info!("iroh: connected to {}", conn.remote_id().fmt_short());

        if let Ok(mut s) = self.0.session_state.lock() {
            *s = SessionState::Connecting {
                phase: "Establishing RPC session".into(),
            };
        }
        self.notify_state_change();

        let local_eid = endpoint.id().fmt_short().to_string();
        let remote_eid = conn.remote_id().fmt_short().to_string();
        let alpn = String::from_utf8_lossy(conn.alpn()).to_string();
        let conn_for_paths = conn.clone();
        let conn_for_watcher = conn.clone();
        let remote_eid_for_log = remote_eid.clone();

        let remote = irpc_iroh::IrohRemoteConnection::new(conn);
        let client = irpc::Client::<ZedraProto>::boxed(remote);

        // Refresh ConnectionInfo on each path change or every 2s.
        {
            use iroh::Watcher;
            let mut paths = conn_for_paths.paths();
            let info_slot = self.0.connection_info.clone();
            let latency_ms = self.0.latency_ms.clone();
            tokio::spawn(async move {
                let mut paths_watcher_active = true;
                loop {
                    let path_list = paths.get();
                    let selected = path_list.iter().find(|p| p.is_selected());
                    if let Some(path) = selected {
                        let stats = path.stats();
                        let is_direct = path.is_ip();
                        let (remote_addr, relay_url) = match path.remote_addr() {
                            iroh::TransportAddr::Ip(addr) => (addr.to_string(), None),
                            iroh::TransportAddr::Relay(url) => {
                                let host = url.host_str().unwrap_or(url.as_str()).to_string();
                                (host.clone(), Some(host))
                            }
                            _ => (format!("{:?}", path.remote_addr()), None),
                        };
                        let rtt = stats.rtt.as_millis() as u64;
                        latency_ms.store(rtt, Ordering::Relaxed);
                        let info = ConnectionInfo {
                            is_direct,
                            remote_addr,
                            relay_url,
                            endpoint_id: remote_eid.clone(),
                            local_endpoint_id: local_eid.clone(),
                            num_paths: path_list.len(),
                            protocol: alpn.clone(),
                            path_rtt_ms: rtt,
                            bytes_sent: stats.udp_tx.bytes,
                            bytes_recv: stats.udp_rx.bytes,
                        };
                        let was_relay = info_slot
                            .lock()
                            .ok()
                            .and_then(|g| g.as_ref().map(|i| !i.is_direct))
                            .unwrap_or(true);
                        if was_relay && info.is_direct {
                            tracing::info!("iroh path upgraded: relay -> direct P2P");
                        }
                        if let Ok(mut slot) = info_slot.lock() {
                            *slot = Some(info);
                        }
                    }
                    if paths_watcher_active {
                        tokio::select! {
                            res = paths.updated() => {
                                if res.is_err() {
                                    tracing::debug!("iroh path watcher closed — switching to polling");
                                    paths_watcher_active = false;
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

        // Store the new client.
        if let Ok(mut slot) = self.0.client.lock() {
            *slot = Some(client.clone());
        }

        // PKI auth.
        {
            let ticket = self.0.pending_ticket.lock().ok().and_then(|mut g| g.take());
            let session_id = self.session_id();
            let endpoint_id = self.endpoint_id().expect("endpoint_id stored above");
            match self.signer() {
                Some(signer) => {
                    match Self::authenticate(
                        &client,
                        ticket.as_ref(),
                        signer.as_ref(),
                        &endpoint_id,
                        session_id.as_deref(),
                    )
                    .await
                    {
                        Ok(sid) => {
                            if let Ok(mut slot) = self.0.sid.lock() {
                                *slot = Some(sid);
                            }
                        }
                        Err(e) => {
                            tracing::warn!("PKI auth failed: {}", e);
                        }
                    }
                }
                None => {
                    tracing::warn!("No signer — skipping PKI auth (RPC calls will fail)");
                }
            }
        }

        self.fetch_session_info(&client, "unknown").await;
        self.reattach_terminals(&client).await;

        let handle_for_watcher = self.clone();
        tokio::spawn(async move {
            conn_for_watcher.closed().await;
            tracing::info!("iroh connection closed, triggering reconnect");
            if let Ok(mut reason) = handle_for_watcher.0.reconnect_reason.lock() {
                *reason = ReconnectReason::ConnectionLost;
            }
            handle_for_watcher.spawn_reconnect();
        });

        tracing::info!(
            "SessionHandle: connected via iroh to {}",
            remote_eid_for_log
        );
        Ok(())
    }

    /// PKI auth. First connect: Register → Auth → AuthProve.
    /// Reconnect (no ticket): Auth → AuthProve.
    async fn authenticate(
        client: &irpc::Client<ZedraProto>,
        ticket: Option<&zedra_rpc::ZedraPairingTicket>,
        signer: &dyn ClientSigner,
        endpoint_id: &iroh::PublicKey,
        session_id: Option<&str>,
    ) -> Result<String> {
        use ed25519_dalek::{Verifier, VerifyingKey};
        use std::time::{SystemTime, UNIX_EPOCH};

        let client_pubkey = signer.pubkey();

        // Step 1: Register (first connection only).
        if let Some(t) = ticket {
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
                }
                other => {
                    return Err(anyhow::anyhow!("register failed: {:?}", other));
                }
            }
        }

        // Authenticate — get nonce + verify host signature.
        let challenge: AuthChallengeResult = client.rpc(AuthReq { client_pubkey }).await?;
        {
            let vk_bytes = endpoint_id.as_bytes();
            let vk = VerifyingKey::from_bytes(vk_bytes)
                .map_err(|e| anyhow::anyhow!("invalid host pubkey: {e}"))?;
            let sig = ed25519_dalek::Signature::from_bytes(&challenge.host_signature);
            vk.verify(&challenge.nonce, &sig)
                .map_err(|_| anyhow::anyhow!("host challenge signature invalid"))?;
        }

        // AuthProve — sign nonce, attach to target session.
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
                tracing::info!("PKI: authenticated, session {}", attach_session_id);
                Ok(attach_session_id)
            }
            other => Err(anyhow::anyhow!("auth prove failed: {:?}", other)),
        }
    }

    async fn fetch_session_info(&self, client: &irpc::Client<ZedraProto>, fallback_hostname: &str) {
        match client.rpc(SessionInfoReq {}).await {
            Ok(info) => {
                tracing::info!(
                    "Session info: host={}, user={}, workdir={}",
                    info.hostname,
                    info.username,
                    info.workdir,
                );
                if let Some(sid) = info.session_id {
                    if let Ok(mut slot) = self.0.sid.lock() {
                        if slot.is_none() {
                            *slot = Some(sid);
                        }
                    }
                }
                if let Ok(mut s) = self.0.session_state.lock() {
                    *s = SessionState::Connected {
                        hostname: info.hostname,
                        username: info.username,
                        workdir: info.workdir,
                        os: info.os.unwrap_or_default(),
                        arch: info.arch.unwrap_or_default(),
                        os_version: info.os_version.unwrap_or_default(),
                        host_version: info.host_version.unwrap_or_default(),
                    };
                }
            }
            Err(e) => {
                tracing::warn!("session/info failed: {e}");
                if let Ok(mut s) = self.0.session_state.lock() {
                    *s = SessionState::Connected {
                        hostname: fallback_hostname.to_string(),
                        username: String::new(),
                        workdir: String::new(),
                        os: String::new(),
                        arch: String::new(),
                        os_version: String::new(),
                        host_version: String::new(),
                    };
                }
            }
        }
        self.notify_state_change();
    }

    /// Open a TermAttach bidi stream and spawn input/output bridge tasks.
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
            loop {
                match irpc_output_rx.recv().await {
                    Ok(Some(output)) => {
                        terminal_pump.update_seq(output.seq);
                        if let Ok(mut buf) = terminal_pump.output.lock() {
                            buf.push_back(output.data);
                        }
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
        for terminal in &terminals {
            if let Err(e) = self.attach_terminal(client, terminal).await {
                tracing::warn!("failed to reattach terminal {}: {e}", terminal.id);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Reconnect loop
    // -----------------------------------------------------------------------

    /// Spawn a reconnect loop with exponential backoff (1–30s, max 10 attempts).
    /// No-op if `user_disconnect` is set or a loop is already running.
    pub fn spawn_reconnect(&self) {
        if self.0.user_disconnect.load(Ordering::Acquire) {
            tracing::info!("spawn_reconnect: skipping, user disconnect in progress");
            return;
        }

        if self
            .0
            .reconnect_attempt
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Relaxed)
            .is_err()
        {
            tracing::info!("spawn_reconnect: already reconnecting");
            return;
        }

        let handle = self.clone();
        session_runtime().spawn(async move {
            let max_attempts = 10u32;
            let mut attempt = 1u32;

            loop {
                if handle.0.user_disconnect.load(Ordering::Acquire) {
                    tracing::info!("reconnect: user disconnect during reconnect, aborting");
                    break;
                }

                if attempt > max_attempts {
                    tracing::warn!("reconnect: max attempts ({}) reached", max_attempts);
                    break;
                }

                handle.set_reconnect_attempt(attempt);
                handle.notify_state_change();

                let delay_secs = std::cmp::min(1u64 << (attempt - 1), 30);
                let skip_backoff = handle.0.skip_next_backoff.swap(false, Ordering::AcqRel);
                let next_retry_secs = if skip_backoff { 0 } else { delay_secs };
                handle
                    .0
                    .next_retry_secs
                    .store(next_retry_secs, Ordering::Release);

                if skip_backoff {
                    tracing::info!(
                        "reconnect: attempt {} of {} (skipping backoff — foreground resume)",
                        attempt,
                        max_attempts,
                    );
                } else {
                    tracing::info!(
                        "reconnect: attempt {} of {} (backoff {}s)",
                        attempt,
                        max_attempts,
                        delay_secs,
                    );
                    for remaining in (1..=delay_secs).rev() {
                        handle.0.next_retry_secs.store(remaining, Ordering::Release);
                        handle.notify_state_change();
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        if handle.0.user_disconnect.load(Ordering::Acquire) {
                            break;
                        }
                    }
                    handle.0.next_retry_secs.store(0, Ordering::Release);
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
                        match handle.terminal_list().await {
                            Ok(ids) => tracing::info!(
                                "reconnect: server has {} terminals: {:?}",
                                ids.len(),
                                ids,
                            ),
                            Err(e) => tracing::warn!("reconnect: terminal_list failed: {}", e),
                        }
                        handle.set_reconnect_attempt(0);
                        handle.notify_state_change();
                        return;
                    }
                    Err(e) => {
                        tracing::warn!("reconnect: attempt {} failed: {}", attempt, e);
                        attempt += 1;
                    }
                }
            }

            handle.set_reconnect_attempt(0);
            if !handle.0.user_disconnect.load(Ordering::Acquire) && attempt > max_attempts {
                tracing::warn!(
                    "reconnect: {} attempts exhausted, marking host unreachable",
                    max_attempts
                );
                handle.0.host_unreachable.store(true, Ordering::Release);
            }
            handle.notify_state_change();
        });
    }

    // -----------------------------------------------------------------------
    // RPC helpers
    // -----------------------------------------------------------------------

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
        self.0.latency_ms.store(rtt_ms, Ordering::Relaxed);
        Ok(rtt_ms)
    }

    // -----------------------------------------------------------------------
    // RPC: filesystem
    // -----------------------------------------------------------------------

    pub async fn fs_list(&self, path: &str) -> Result<Vec<FsEntry>> {
        let client = self.client()?;
        let result: FsListResult = client
            .rpc(FsListReq {
                path: path.to_string(),
            })
            .await?;
        Ok(result.entries)
    }

    pub async fn fs_read(&self, path: &str) -> Result<String> {
        let client = self.client()?;
        let result: FsReadResult = client
            .rpc(FsReadReq {
                path: path.to_string(),
            })
            .await?;
        Ok(result.content)
    }

    pub async fn fs_write(&self, path: &str, content: &str) -> Result<()> {
        let client = self.client()?;
        let _: FsWriteResult = client
            .rpc(FsWriteReq {
                path: path.to_string(),
                content: content.to_string(),
            })
            .await?;
        Ok(())
    }

    pub async fn fs_stat(&self, path: &str) -> Result<FsStatResult> {
        let client = self.client()?;
        Ok(client
            .rpc(FsStatReq {
                path: path.to_string(),
            })
            .await?)
    }

    // -----------------------------------------------------------------------
    // RPC: git
    // -----------------------------------------------------------------------

    pub async fn git_status(&self) -> Result<GitStatusResult> {
        Ok(self.client()?.rpc(GitStatusReq {}).await?)
    }

    pub async fn git_diff(&self, path: Option<&str>, staged: bool) -> Result<String> {
        let client = self.client()?;
        let result: GitDiffResult = client
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

    // -----------------------------------------------------------------------
    // RPC: terminals
    // -----------------------------------------------------------------------

    pub async fn terminal_create(&self, cols: u16, rows: u16) -> Result<String> {
        let client = self.client()?;
        let result: TermCreateResult = client.rpc(TermCreateReq { cols, rows }).await?;

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

    pub async fn terminal_list(&self) -> Result<Vec<String>> {
        let result: TermListResult = self.client()?.rpc(TermListReq {}).await?;
        Ok(result.terminals.into_iter().map(|e| e.id).collect())
    }

    /// Attach to a pre-existing server terminal (cold-start session resume).
    pub async fn terminal_attach_existing(&self, id: &str) -> Result<()> {
        let client = self.client()?;
        let terminal = {
            let mut terms = self.0.terminals.lock().unwrap();
            if let Some(t) = terms.iter().find(|t| t.id == id) {
                t.clone()
            } else {
                let t = RemoteTerminal::new(id.to_string());
                terms.push(t.clone());
                t
            }
        };
        self.attach_terminal(&client, &terminal).await?;
        tracing::info!("Terminal attached (existing): {}", id);
        Ok(())
    }
}

impl Default for SessionHandle {
    fn default() -> Self {
        Self::new()
    }
}
