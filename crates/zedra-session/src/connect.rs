use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ed25519_dalek::VerifyingKey;
use futures::future::join_all;
use iroh::{
    Endpoint, EndpointAddr, NetReport, PublicKey, RelayConfig, RelayMap, RelayMode,
    address_lookup::PkarrResolver,
    endpoint::{
        AuthenticationError as IrohAuthenticationError, ConnectError as IrohConnectError,
        ConnectingError as IrohConnectingError, Connection, ConnectionError as IrohConnectionError,
        PathInfo, QuicTransportConfig, TransportErrorCode,
    },
};
use iroh_relay::RelayQuicConfig;
use tokio::{sync::mpsc, task::JoinHandle, time::Instant};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};
use zedra_rpc::{
    ZEDRA_RELAY_URLS, ZedraPairingTicket, compute_registration_hmac,
    proto::{
        AuthProveReq, AuthProveResult, ConnectReq, ConnectResult, RegisterReq, RegisterResult,
        SyncSessionResult, TerminalSyncEntry, ZEDRA_ALPN, ZedraProto,
    },
};

use crate::RemoteTerminal;
use crate::signer::ClientSigner;
use crate::state::{AuthOutcome, ConnectError, ReconnectReason};

pub struct ConnectConfig {
    alpn: Vec<u8>,
    alpns: Vec<Vec<u8>>,
    relay_urls: Vec<String>,
    /// Interval at which to check for idle connections.
    idle_check_interval: Duration,
    keep_alive_interval: Duration,
    max_idle_timeout: Duration,
    path_keep_alive_interval: Duration,
    path_max_idle_timeout: Duration,
}

pub type ConnectedResult = (
    irpc::Client<ZedraProto>,
    SyncSessionResult,
    Vec<RemoteTerminal>,
);

const IDLE_STALE_INTERVAL_MULTIPLIER: u32 = 2;
const TLS_ALERT_NO_APPLICATION_PROTOCOL: u8 = 0x78;

/// Tracks the last confirmed transport receive progress for UI idle state.
///
/// Reconnect is still driven by iroh timeouts. The UI should flip to idle
/// earlier when the connection stops hearing back from the remote side.
///
/// Liveness is connection-wide, not selected-path-only:
/// - any inbound bytes across any path keep the session active
/// - selected path is only used for the transport label / RTT metadata
///
/// That avoids false idle when iroh is switching paths or when non-selected
/// paths still carry valid traffic for the connection.
#[derive(Debug)]
struct IdleDetector {
    last_recv_bytes: u64,
    last_alive_at: Instant,
}

impl IdleDetector {
    fn with_recv_baseline(now: Instant, recv_bytes_total: u64) -> Self {
        Self {
            last_recv_bytes: recv_bytes_total,
            last_alive_at: now,
        }
    }

    fn observe(&mut self, recv_bytes_total: u64, now: Instant) -> bool {
        let recv_progressed = recv_bytes_total > self.last_recv_bytes;
        if recv_progressed {
            self.last_alive_at = now;
        }
        self.last_recv_bytes = recv_bytes_total;
        recv_progressed
    }

    fn is_idle(&self, now: Instant, threshold: Duration) -> bool {
        now.duration_since(self.last_alive_at) >= threshold
    }
}

impl Default for ConnectConfig {
    fn default() -> Self {
        // Tighten QUIC timeouts for fast disconnect detection.
        // PING frames are tiny UDP packets — cheap even on mobile.
        // Default iroh heartbeat is 5s; we lower to 2s so transport freshness
        // is sampled quickly for the session badge.
        Self {
            alpn: ZEDRA_ALPN.to_vec(),
            alpns: vec![ZEDRA_ALPN.to_vec()],
            relay_urls: ZEDRA_RELAY_URLS.iter().map(|u| u.to_string()).collect(),
            // Must be larger than the keep-alive interval
            idle_check_interval: Duration::from_secs(2),
            keep_alive_interval: Duration::from_secs(1),
            max_idle_timeout: Duration::from_secs(20),
            path_keep_alive_interval: Duration::from_secs(1),
            path_max_idle_timeout: Duration::from_secs(20),
        }
    }
}

impl ConnectConfig {
    pub fn to_relay_map(&self) -> RelayMap {
        let relay_configs: Vec<RelayConfig> = self
            .relay_urls
            .iter()
            .map(|u| RelayConfig {
                url: u.parse().expect("valid relay url"),
                quic: Some(RelayQuicConfig::default()),
            })
            .collect();
        RelayMap::from_iter(relay_configs)
    }

    pub fn to_transport_config(&self) -> QuicTransportConfig {
        QuicTransportConfig::builder()
            .keep_alive_interval(self.keep_alive_interval)
            .max_idle_timeout(Some(
                self.max_idle_timeout
                    .try_into()
                    .expect("6s fits in QUIC VarInt"),
            ))
            .default_path_keep_alive_interval(self.path_keep_alive_interval)
            .default_path_max_idle_timeout(self.path_max_idle_timeout)
            .build()
    }
}

pub struct Connector {
    tx: mpsc::Sender<ConnectEvent>,
    config: ConnectConfig,
    abort_signal: CancellationToken,
    started_at: Option<Instant>,
}

fn reconcile_synced_terminals(
    sync_entries: &[TerminalSyncEntry],
    existing_terminals: Option<&[RemoteTerminal]>,
) -> Vec<RemoteTerminal> {
    let mut sync_by_id = sync_entries
        .iter()
        .map(|entry| (entry.id.as_str(), entry))
        .collect::<HashMap<_, _>>();
    let mut terminals = Vec::with_capacity(sync_entries.len());

    if let Some(existing_terminals) = existing_terminals {
        for terminal in existing_terminals {
            let id = terminal.id();
            if let Some(entry) = sync_by_id.remove(id.as_str()) {
                terminals.push(reconcile_synced_terminal(entry, Some(terminal.clone())));
            }
        }
    }

    for entry in sync_entries {
        if sync_by_id.remove(entry.id.as_str()).is_some() {
            terminals.push(reconcile_synced_terminal(entry, None));
        }
    }

    terminals
}

fn reconcile_synced_terminal(
    entry: &TerminalSyncEntry,
    existing_terminal: Option<RemoteTerminal>,
) -> RemoteTerminal {
    match existing_terminal {
        Some(terminal) if terminal.last_seq() <= entry.last_seq => terminal,
        _ => RemoteTerminal::new(entry.id.clone()),
    }
}

impl Connector {
    pub fn new(tx: mpsc::Sender<ConnectEvent>) -> Self {
        Self {
            tx,
            config: ConnectConfig::default(),
            abort_signal: CancellationToken::new(),
            started_at: None,
        }
    }

    pub fn with_config(tx: mpsc::Sender<ConnectEvent>, config: ConnectConfig) -> Self {
        Self {
            tx,
            config,
            abort_signal: CancellationToken::new(),
            started_at: None,
        }
    }

    /// Cancel all active watcher tasks (call before starting a new connection).
    pub fn abort(&self) {
        self.abort_signal.cancel();
    }

    /// Reset for a new connection attempt.
    pub fn reset(&mut self) {
        self.abort_signal = CancellationToken::new();
        self.started_at = Some(Instant::now());
    }

    fn emit(&self, event: ConnectEvent) {
        let _ = self.tx.try_send(event);
    }
}

/// Events emitted by Connector to signal connection state changes.
/// UI listens on the rx channel and updates local ConnectState accordingly.
/// Events carry raw iroh types - UI extracts what it needs.
#[derive(Debug, Clone)]
pub enum ConnectEvent {
    BindingEndpoint,
    EndpointBound {
        local_node_id: String,
        binding_ms: u64,
    },
    HolePunchStarted,
    HolePunchComplete {
        remote_node_id: String,
        alpn: String,
        hole_punch_ms: u64,
    },
    Registering {
        session_id: String,
    },
    RegisterComplete {
        register_ms: u64,
    },
    Authenticating,
    Proving,
    AuthComplete {
        auth_ms: u64,
        outcome: AuthOutcome,
        is_first_pairing: bool,
    },
    Syncing,
    SyncComplete {
        sync: SyncSessionResult,
        sync_ms: u64,
    },
    TerminalsReattached {
        count: usize,
        resume_ms: u64,
    },
    Connected {
        total_ms: u64,
    },
    Failed {
        error: ConnectError,
    },

    EndpointAddrChanged {
        endpoint_addr: EndpointAddr,
    },
    NetReport {
        net_report: NetReport,
    },

    PathReport {
        path: PathInfo,
        num_paths: usize,
        bytes_sent_total: u64,
        bytes_recv_total: u64,
    },
    PathUpgraded {
        prev_path: Option<PathInfo>,
        new_path: PathInfo,
    },
    NoActivePath,

    ConnectionIdle,
    ConnectionActive,

    ReconnectStarted {
        reason: ReconnectReason,
    },
    ReconnectAttempt {
        attempt: u32,
        reason: ReconnectReason,
        next_retry_secs: u64,
    },
    ReconnectSuccess {
        attempt: u32,
        elapsed_ms: u64,
    },
    ReconnectExhausted {
        attempts: u32,
        elapsed_ms: u64,
        error: ConnectError,
    },

    ConnectionClosed,
}

impl Connector {
    /// Full connection flow: bind endpoint → hole punch → RPC → auth → sync.
    /// Emits ConnectEvent at each phase transition.
    pub async fn connect(
        &mut self,
        remote_addr: EndpointAddr,
        ticket: Option<&ZedraPairingTicket>,
        signer: Arc<dyn ClientSigner>,
        session_id: Option<String>,
        session_token: Option<[u8; 32]>,
        existing_terminals: Option<Vec<RemoteTerminal>>,
    ) -> Result<ConnectedResult, ConnectError> {
        self.reset();
        let endpoint_id = remote_addr.id;

        // Phase: BindingEndpoint
        let endpoint = self.proceed_binding_endpoint().await?;
        self.spawn_local_endpoint_watcher(endpoint.clone());

        // Phase: HolePunching
        let conn = self.proceed_hole_punching(endpoint, remote_addr).await?;
        self.spawn_remote_paths_watcher(conn.clone());
        self.spawn_connection_closed_watcher(conn.clone());

        // Establish RPC client
        let remote = irpc_iroh::IrohRemoteConnection::new(conn);
        let client = irpc::Client::<ZedraProto>::boxed(remote);

        // Phase: Auth (Register if ticket, then Connect/Prove)
        let t_auth = Instant::now();
        let (sync, outcome, is_first_pairing) = match self
            .proceed_bootstrap_session(
                &client,
                ticket,
                signer.as_ref(),
                &endpoint_id,
                session_id,
                session_token,
            )
            .await
        {
            Ok(result) => result,
            Err(error) => {
                self.emit(ConnectEvent::Failed {
                    error: error.clone(),
                });
                return Err(error);
            }
        };
        let auth_ms = t_auth.elapsed().as_millis() as u64;
        self.emit(ConnectEvent::AuthComplete {
            auth_ms,
            outcome,
            is_first_pairing,
        });

        self.emit(ConnectEvent::Syncing);
        let t_sync = Instant::now();
        let terminals = self
            .process_sync(&client, &sync, existing_terminals)
            .await?;
        let sync_ms = t_sync.elapsed().as_millis() as u64;

        self.emit(ConnectEvent::SyncComplete {
            sync: sync.clone(),
            sync_ms,
        });

        // Connected
        let total_ms = self
            .started_at
            .map(|t| t.elapsed().as_millis() as u64)
            .unwrap_or(0);
        self.emit(ConnectEvent::Connected { total_ms });

        Ok((client, sync, terminals))
    }

    pub async fn proceed_binding_endpoint(&mut self) -> Result<Endpoint, ConnectError> {
        self.emit(ConnectEvent::BindingEndpoint);
        let t = Instant::now();

        let relay_map = self.config.to_relay_map();
        let transport_config = self.config.to_transport_config();
        let alpn_protocols = self.config.alpns.clone();

        let endpoint = Endpoint::builder()
            .transport_config(transport_config)
            .relay_mode(RelayMode::Custom(relay_map))
            .alpns(alpn_protocols)
            .address_lookup(PkarrResolver::n0_dns())
            .bind()
            .await
            .map_err(|e| {
                let err = ConnectError::EndpointBindFailed(e.to_string());
                self.emit(ConnectEvent::Failed { error: err.clone() });
                err
            })?;

        let local_node_id = endpoint.id().fmt_short().to_string();
        let binding_ms = t.elapsed().as_millis() as u64;
        info!(
            "iroh client endpoint bound: {} in {}ms",
            local_node_id, binding_ms
        );

        self.emit(ConnectEvent::EndpointBound {
            local_node_id,
            binding_ms,
        });
        Ok(endpoint)
    }

    pub async fn proceed_hole_punching(
        &self,
        endpoint: Endpoint,
        remote_addr: EndpointAddr,
    ) -> Result<Connection, ConnectError> {
        self.emit(ConnectEvent::HolePunchStarted);
        let t = Instant::now();

        let conn = endpoint
            .connect(remote_addr, &self.config.alpn)
            .await
            .map_err(|e| {
                let err = map_iroh_connect_error(&e);
                self.emit(ConnectEvent::Failed { error: err.clone() });
                err
            })?;

        let hole_punch_ms = t.elapsed().as_millis() as u64;
        let remote_node_id = conn.remote_id().fmt_short().to_string();
        let alpn = String::from_utf8_lossy(conn.alpn()).to_string();
        info!(
            "iroh: connected to {}, alpn: {} in {}ms",
            remote_node_id, alpn, hole_punch_ms
        );

        self.emit(ConnectEvent::HolePunchComplete {
            remote_node_id,
            alpn,
            hole_punch_ms,
        });
        Ok(conn)
    }

    /// Full auth flow: Register (if ticket) → Connect → Prove (if challenge).
    /// Returns (SyncSessionResult, AuthOutcome, is_first_pairing).
    pub async fn proceed_bootstrap_session(
        &self,
        client: &irpc::Client<ZedraProto>,
        ticket: Option<&ZedraPairingTicket>,
        signer: &dyn ClientSigner,
        endpoint_id: &PublicKey,
        session_id: Option<String>,
        session_token: Option<[u8; 32]>,
    ) -> Result<(SyncSessionResult, AuthOutcome, bool), ConnectError> {
        let client_pubkey = signer.pubkey();
        let mut is_first_pairing = false;
        let mut outcome = AuthOutcome::Authenticated;

        // Step 1: Register (first pairing only)
        if let Some(ticket) = ticket {
            is_first_pairing = true;
            self.proceed_registering(
                client,
                client_pubkey,
                ticket.handshake_secret,
                ticket.session_id.clone(),
            )
            .await?;
            outcome = AuthOutcome::Registered;
        }

        // Resolve session_id
        let session_id = session_id
            .or_else(|| ticket.map(|t| t.session_id.clone()))
            .ok_or_else(|| ConnectError::Other("no session_id provided".to_string()))?;

        // Step 2: Connect RPC
        self.emit(ConnectEvent::Authenticating);
        let nonce = match self
            .proceed_connect(
                client,
                client_pubkey,
                endpoint_id,
                session_id.clone(),
                session_token,
            )
            .await?
        {
            (None, Some(sync)) => return Ok((sync, outcome, is_first_pairing)),
            (Some(nonce), None) => nonce,
            _ => return Err(ConnectError::Other("unexpected connect result".to_string())),
        };

        // Step 3: Prove identity
        self.emit(ConnectEvent::Proving);
        let sync = self
            .proceed_auth_proving(client, signer, nonce, session_id)
            .await?;
        Ok((sync, outcome, is_first_pairing))
    }

    pub async fn proceed_registering(
        &self,
        client: &irpc::Client<ZedraProto>,
        client_pubkey: [u8; 32],
        handshake_secret: [u8; 16],
        session_id: String,
    ) -> Result<(), ConnectError> {
        self.emit(ConnectEvent::Registering {
            session_id: session_id.clone(),
        });
        let t = Instant::now();
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let hmac = compute_registration_hmac(&handshake_secret, &client_pubkey, timestamp);

        match client
            .rpc(RegisterReq {
                client_pubkey,
                timestamp,
                hmac,
                session_id,
            })
            .await
            .map_err(|e| ConnectError::RequestError(e.to_string()))?
        {
            RegisterResult::Ok => {
                let register_ms = t.elapsed().as_millis() as u64;
                self.emit(ConnectEvent::RegisterComplete { register_ms });
                Ok(())
            }
            RegisterResult::HandshakeConsumed => Err(ConnectError::HandshakeConsumed),
            RegisterResult::InvalidHandshake => Err(ConnectError::InvalidHandshake),
            RegisterResult::StaleTimestamp => Err(ConnectError::StaleTimestamp),
            RegisterResult::SlotNotFound => Err(ConnectError::SlotNotFound),
        }
    }

    /// Returns nonce for PKI challenge or SyncSessionResult if session token accepted.
    pub async fn proceed_connect(
        &self,
        client: &irpc::Client<ZedraProto>,
        client_pubkey: [u8; 32],
        endpoint_id: &PublicKey,
        session_id: String,
        session_token: Option<[u8; 32]>,
    ) -> Result<(Option<[u8; 32]>, Option<SyncSessionResult>), ConnectError> {
        match client
            .rpc(ConnectReq {
                client_pubkey,
                session_id,
                session_token,
            })
            .await
            .map_err(|e| ConnectError::RequestError(e.to_string()))?
        {
            ConnectResult::Ok(sync) => {
                info!(
                    "connect: session token accepted, session={}",
                    sync.session_id
                );
                Ok((None, Some(sync)))
            }
            ConnectResult::Challenge {
                nonce,
                host_signature,
            } => {
                use ed25519_dalek::Verifier;
                let vk = VerifyingKey::from_bytes(endpoint_id.as_bytes())
                    .map_err(|_| ConnectError::HostInvalidPubkey)?;
                let sig = ed25519_dalek::Signature::from_bytes(&host_signature);
                vk.verify(&nonce, &sig)
                    .map_err(|_| ConnectError::HostSignatureInvalid)?;
                Ok((Some(nonce), None))
            }
            ConnectResult::Unauthorized | ConnectResult::NotInSessionAcl => {
                Err(ConnectError::Unauthorized)
            }
            ConnectResult::SessionOccupied => Err(ConnectError::SessionOccupied),
            ConnectResult::SessionNotFound => Err(ConnectError::SessionNotFound),
        }
    }

    /// Sign nonce and return SyncSessionResult.
    pub async fn proceed_auth_proving(
        &self,
        client: &irpc::Client<ZedraProto>,
        signer: &dyn ClientSigner,
        nonce: [u8; 32],
        session_id: String,
    ) -> Result<SyncSessionResult, ConnectError> {
        let client_signature = signer.sign(&nonce);

        match client
            .rpc(AuthProveReq {
                nonce,
                client_signature,
                session_id: session_id.clone(),
            })
            .await
            .map_err(|e| ConnectError::RequestError(e.to_string()))?
        {
            AuthProveResult::Ok(sync) => {
                info!("authenticated, session={}", session_id);
                Ok(sync)
            }
            AuthProveResult::Unauthorized | AuthProveResult::NotInSessionAcl => {
                Err(ConnectError::Unauthorized)
            }
            AuthProveResult::SessionOccupied => Err(ConnectError::SessionOccupied),
            AuthProveResult::SessionNotFound => Err(ConnectError::SessionNotFound),
            AuthProveResult::InvalidSignature => Err(ConnectError::InvalidSignature),
        }
    }

    /// Proceeds sync data, create new remote terminals from sync result.
    /// And attach them to the remote with bidi streams.
    /// Return the list of attached terminals.
    pub async fn process_sync(
        &mut self,
        client: &irpc::Client<ZedraProto>,
        sync: &SyncSessionResult,
        existing_terminals: Option<Vec<RemoteTerminal>>,
    ) -> Result<Vec<RemoteTerminal>, ConnectError> {
        let terminals = reconcile_synced_terminals(&sync.terminals, existing_terminals.as_deref());

        join_all(terminals.iter().map(async |t| {
            if let Err(e) = t.attach_remote(client).await {
                warn!("failed to attach terminal {}: {}", t.id(), e);
            }
        }))
        .await;

        Ok(terminals)
    }
}

impl Connector {
    /// Watch local endpoint address and net-report changes.
    pub fn spawn_local_endpoint_watcher(&self, endpoint: Endpoint) -> JoinHandle<()> {
        let tx = self.tx.clone();
        let abort_signal = self.abort_signal.clone();

        tokio::spawn(async move {
            use iroh::Watcher;
            let mut addr_watcher = endpoint.watch_addr();
            let mut report_watcher = endpoint.net_report();

            loop {
                tokio::select! {
                    _ = abort_signal.cancelled() => {
                        info!("abort signal cancelled in local endpoint watcher");
                        break;
                    },
                    res = addr_watcher.updated() => {
                        if let Err(e) = res {
                            warn!("addr_watcher error: {:?}", e);
                            break;
                        }
                        info!("endpoint addr changed: {:?}", addr_watcher.get());

                        let endpoint_addr = addr_watcher.get();
                        let _ = tx.send(ConnectEvent::EndpointAddrChanged { endpoint_addr }).await;
                    }
                    res = report_watcher.updated() => {
                        if let Err(e) = res {
                            warn!("report_watcher error: {:?}", e);
                            break;
                        }

                        if let Some(net_report) = report_watcher.get() {
                            let preferred_relay = match net_report.preferred_relay.clone() {
                                Some(relay) => relay.to_string(),
                                None => "".to_string(),
                            };
                            info!("net report changed: {})", preferred_relay);
                            let _ = tx.send(ConnectEvent::NetReport { net_report }).await;
                        } else {
                            info!("net report changed without a value");
                        }
                    }
                }
            }
            debug!("local endpoint watcher stopped");
        })
    }

    /// Watch remote path changes (RTT, bytes, path upgrades).
    pub fn spawn_remote_paths_watcher(&self, conn: Connection) -> JoinHandle<()> {
        use iroh::Watcher;
        let tx = self.tx.clone();
        let abort_signal = self.abort_signal.clone();
        let idle_check_interval = self.config.idle_check_interval;
        let idle_stale_timeout = idle_check_interval * IDLE_STALE_INTERVAL_MULTIPLIER;

        tokio::spawn(async move {
            let mut paths = conn.paths();
            let mut prev_path: Option<PathInfo> = None;
            let mut prev_is_direct = false;
            let mut idle_detector =
                IdleDetector::with_recv_baseline(Instant::now(), conn.stats().udp_rx.bytes);

            loop {
                let path_list = paths.get();
                if let Some(path) = path_list.iter().find(|p| p.is_selected()) {
                    let is_direct = path.is_ip();
                    let path_stats = path.stats();
                    let conn_stats = conn.stats();

                    if let Some(prev) = prev_path.clone()
                        && prev != *path
                    {
                        info!(
                            "path selected: {:?} is_direct: {} rtt {:?}",
                            path.remote_addr(),
                            is_direct,
                            path_stats.rtt
                        );
                    }

                    // Detect relay → direct upgrade
                    if !prev_is_direct && is_direct {
                        info!(
                            "direct path upgrade detected: {:?} -> {:?}",
                            prev_path.as_ref().map(|p| p.remote_addr()),
                            path.remote_addr()
                        );
                        let _ = tx
                            .send(ConnectEvent::PathUpgraded {
                                prev_path: prev_path,
                                new_path: path.clone(),
                            })
                            .await;
                    }
                    prev_is_direct = is_direct;
                    prev_path = Some(path.clone());

                    let _ = tx
                        .send(ConnectEvent::PathReport {
                            path: path.clone(),
                            num_paths: path_list.len(),
                            // Use connection stats instead of path stats to accumulate all paths
                            bytes_sent_total: conn_stats.udp_tx.bytes,
                            bytes_recv_total: conn_stats.udp_rx.bytes,
                        })
                        .await;
                } else {
                    info!("no active path");
                    let _ = tx.send(ConnectEvent::NoActivePath).await;
                }

                tokio::select! {
                    _ = abort_signal.cancelled() => {
                        info!("abort signal cancelled in remote paths watcher");
                        break;
                    },
                    res = paths.updated() => {
                        match res {
                            Err(err) => {
                                error!("paths updated error: {:?}", err);
                                break;
                            }
                            Ok(paths) => {
                                info!("paths updated: {}", paths.len());
                            }
                        };
                    }
                    _ = tokio::time::sleep(idle_check_interval) => {
                        let path_list = paths.get();
                        let selected_path = path_list.iter().find(|p| p.is_selected()).cloned();
                        let conn_stats = conn.stats();
                        let now = Instant::now();
                        let recv_progressed = idle_detector.observe(conn_stats.udp_rx.bytes, now);

                        if let Some(path) = selected_path.as_ref() {
                            let _ = tx
                                .send(ConnectEvent::PathReport {
                                    path: path.clone(),
                                    num_paths: path_list.len(),
                                    bytes_sent_total: conn_stats.udp_tx.bytes,
                                    bytes_recv_total: conn_stats.udp_rx.bytes,
                                })
                                .await;
                        } else {
                            let _ = tx.send(ConnectEvent::NoActivePath).await;
                        }

                        if recv_progressed {
                            debug!(
                                "idle check: active - connection recv bytes changed ({})",
                                conn_stats.udp_rx.bytes
                            );
                            let _ = tx.send(ConnectEvent::ConnectionActive).await;
                        } else if idle_detector.is_idle(now, idle_stale_timeout) {
                            debug!(
                                "idle check: idle - no connection recv bytes for {:?}",
                                idle_stale_timeout
                            );
                            let _ = tx.send(ConnectEvent::ConnectionIdle).await;
                        } else if selected_path.is_some() {
                            debug!(
                                "idle check: active - selected path present, waiting for recv progress"
                            );
                            let _ = tx.send(ConnectEvent::ConnectionActive).await;
                        } else {
                            debug!(
                                "idle check: active - no selected path yet, within {:?}",
                                idle_stale_timeout
                            );
                            let _ = tx.send(ConnectEvent::ConnectionActive).await;
                        }
                    }
                }
            }
            debug!("remote paths watcher stopped");
        })
    }

    /// Watch for connection close and emit ConnectionClosed event.
    pub fn spawn_connection_closed_watcher(&self, conn: Connection) -> JoinHandle<()> {
        let tx = self.tx.clone();
        let abort_signal = self.abort_signal.clone();

        tokio::spawn(async move {
            tokio::select! {
                _ = abort_signal.cancelled() => {
                    info!("abort signal cancelled in connection closed watcher");
                    return;
                }
                err = conn.closed() => {
                    warn!("iroh connection closed: {:?}", err);
                    let _ = tx.send(ConnectEvent::ConnectionClosed).await;
                }
            }
        })
    }
}

impl Connector {
    /// Run reconnect loop with exponential backoff.
    pub async fn reconnect_loop(
        &mut self,
        remote_addr: EndpointAddr,
        ticket: Option<&ZedraPairingTicket>,
        signer: Arc<dyn ClientSigner>,
        session_id: Option<String>,
        session_token: Option<[u8; 32]>,
        reason: ReconnectReason,
        max_attempts: u32,
        terminals: Option<Vec<RemoteTerminal>>,
    ) -> Result<ConnectedResult, ConnectError> {
        let reconnect_start = Instant::now();
        self.emit(ConnectEvent::ReconnectStarted {
            reason: reason.clone(),
        });

        for attempt in 1..=max_attempts {
            let delay_secs = std::cmp::min(1u64 << (attempt - 1), 30);
            warn!("Proceed reconnect ({}), delay {}s", attempt, delay_secs);

            self.emit(ConnectEvent::ReconnectAttempt {
                attempt,
                reason: reason.clone(),
                next_retry_secs: delay_secs,
            });

            if delay_secs > 0 {
                tokio::time::sleep(Duration::from_secs(delay_secs)).await;
            }

            match self
                .connect(
                    remote_addr.clone(),
                    ticket,
                    signer.clone(),
                    session_id.clone(),
                    session_token,
                    terminals.clone(),
                )
                .await
            {
                Ok(result) => {
                    let elapsed_ms = reconnect_start.elapsed().as_millis() as u64;
                    self.emit(ConnectEvent::ReconnectSuccess {
                        attempt,
                        elapsed_ms,
                    });
                    return Ok(result);
                }
                Err(e) if e.is_fatal() => {
                    self.emit(ConnectEvent::ReconnectExhausted {
                        attempts: attempt,
                        elapsed_ms: reconnect_start.elapsed().as_millis() as u64,
                        error: e.clone(),
                    });
                    return Err(e);
                }
                Err(e) => {
                    warn!("reconnect attempt {} failed: {}", attempt, e);
                }
            }
        }

        warn!(
            "reconnect exhausted, giving up attempts {}, reason {:?}",
            max_attempts, reason
        );

        let err = ConnectError::HostUnreachable;
        self.emit(ConnectEvent::ReconnectExhausted {
            attempts: max_attempts,
            elapsed_ms: reconnect_start.elapsed().as_millis() as u64,
            error: err.clone(),
        });
        Err(err)
    }
}

fn map_iroh_connect_error(error: &IrohConnectError) -> ConnectError {
    if is_alpn_mismatch(error) {
        ConnectError::AlpnMismatch
    } else {
        ConnectError::QuicConnectFailed(error.to_string())
    }
}

fn is_alpn_mismatch(error: &IrohConnectError) -> bool {
    match error {
        IrohConnectError::Connecting { source, .. } => is_alpn_mismatch_connecting(source),
        IrohConnectError::Connection { source, .. } => is_alpn_mismatch_connection(source),
        _ => false,
    }
}

fn is_alpn_mismatch_connecting(error: &IrohConnectingError) -> bool {
    match error {
        IrohConnectingError::ConnectionError { source, .. } => is_alpn_mismatch_connection(source),
        IrohConnectingError::HandshakeFailure {
            source: IrohAuthenticationError::NoAlpn { .. },
            ..
        } => true,
        _ => false,
    }
}

fn is_alpn_mismatch_connection(error: &IrohConnectionError) -> bool {
    matches!(
        error,
        IrohConnectionError::ConnectionClosed(close)
            if is_no_application_protocol_error_code(close.error_code)
    )
}

fn is_no_application_protocol_error_code(error_code: TransportErrorCode) -> bool {
    error_code == TransportErrorCode::crypto(TLS_ALERT_NO_APPLICATION_PROTOCOL)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zedra_rpc::proto::TerminalSyncEntry;

    #[test]
    fn idle_detector_requires_receive_progress_to_refresh_liveness() {
        let start = Instant::now();
        let mut detector = IdleDetector::with_recv_baseline(start, 10);
        let threshold = Duration::from_secs(4);

        assert!(!detector.observe(10, start + Duration::from_secs(2)));
        assert!(detector.is_idle(start + Duration::from_secs(4), threshold));

        assert!(detector.observe(11, start + Duration::from_secs(5)));
        assert!(!detector.is_idle(start + Duration::from_secs(7), threshold));
    }

    #[test]
    fn detects_no_application_protocol_tls_alert() {
        assert!(is_no_application_protocol_error_code(
            TransportErrorCode::crypto(TLS_ALERT_NO_APPLICATION_PROTOCOL)
        ));
        assert!(!is_no_application_protocol_error_code(
            TransportErrorCode::crypto(0x2f)
        ));
    }

    #[test]
    fn reconcile_synced_terminals_creates_remote_active_terminals() {
        let terminals = reconcile_synced_terminals(
            &[
                TerminalSyncEntry {
                    id: "term-2".to_string(),
                    position: 0,
                    last_seq: 4,
                    title: None,
                    cwd: None,
                    icon_name: None,
                },
                TerminalSyncEntry {
                    id: "term-3".to_string(),
                    position: 0,
                    last_seq: 9,
                    title: None,
                    cwd: None,
                    icon_name: None,
                },
            ],
            None,
        );

        assert_eq!(
            terminals
                .iter()
                .map(|terminal| terminal.id())
                .collect::<Vec<_>>(),
            vec!["term-2", "term-3"]
        );
    }

    #[test]
    fn reconcile_synced_terminals_replaces_stale_reused_id() {
        let old_terminal = RemoteTerminal::new("term-1".to_string());
        old_terminal.update_seq(42);

        let terminals = reconcile_synced_terminals(
            &[TerminalSyncEntry {
                id: "term-1".to_string(),
                position: 0,
                last_seq: 0,
                title: None,
                cwd: None,
                icon_name: None,
            }],
            Some(std::slice::from_ref(&old_terminal)),
        );

        assert_eq!(terminals.len(), 1);
        assert_eq!(terminals[0].id(), "term-1");
        assert_eq!(terminals[0].last_seq(), 0);
    }

    #[test]
    fn reconcile_synced_terminals_reuses_current_terminal() {
        let existing = RemoteTerminal::new("term-1".to_string());
        existing.update_seq(2);

        let terminals = reconcile_synced_terminals(
            &[TerminalSyncEntry {
                id: "term-1".to_string(),
                position: 0,
                last_seq: 5,
                title: None,
                cwd: None,
                icon_name: None,
            }],
            Some(std::slice::from_ref(&existing)),
        );

        assert_eq!(terminals.len(), 1);
        assert_eq!(terminals[0].last_seq(), 2);
    }

    #[test]
    fn reconcile_synced_terminals_preserves_existing_terminal_order() {
        let existing_first = RemoteTerminal::new("term-b".to_string());
        let existing_second = RemoteTerminal::new("term-a".to_string());

        let terminals = reconcile_synced_terminals(
            &[
                TerminalSyncEntry {
                    id: "term-a".to_string(),
                    position: 0,
                    last_seq: 0,
                    title: None,
                    cwd: None,
                    icon_name: None,
                },
                TerminalSyncEntry {
                    id: "term-b".to_string(),
                    position: 0,
                    last_seq: 0,
                    title: None,
                    cwd: None,
                    icon_name: None,
                },
            ],
            Some(&[existing_first, existing_second]),
        );

        assert_eq!(
            terminals
                .iter()
                .map(|terminal| terminal.id())
                .collect::<Vec<_>>(),
            vec!["term-b", "term-a"]
        );
    }

    #[test]
    fn reconcile_synced_terminals_appends_new_remote_terminals_after_existing_order() {
        let existing = RemoteTerminal::new("term-b".to_string());

        let terminals = reconcile_synced_terminals(
            &[
                TerminalSyncEntry {
                    id: "term-a".to_string(),
                    position: 0,
                    last_seq: 0,
                    title: None,
                    cwd: None,
                    icon_name: None,
                },
                TerminalSyncEntry {
                    id: "term-b".to_string(),
                    position: 0,
                    last_seq: 0,
                    title: None,
                    cwd: None,
                    icon_name: None,
                },
                TerminalSyncEntry {
                    id: "term-c".to_string(),
                    position: 0,
                    last_seq: 0,
                    title: None,
                    cwd: None,
                    icon_name: None,
                },
            ],
            Some(&[existing]),
        );

        assert_eq!(
            terminals
                .iter()
                .map(|terminal| terminal.id())
                .collect::<Vec<_>>(),
            vec!["term-b", "term-a", "term-c"]
        );
    }

    #[test]
    fn reconcile_synced_terminals_drops_local_only_terminals() {
        let local_only = RemoteTerminal::new("stale-local".to_string());
        let active_remote = RemoteTerminal::new("active-remote".to_string());

        let terminals = reconcile_synced_terminals(
            &[TerminalSyncEntry {
                id: "active-remote".to_string(),
                position: 0,
                last_seq: 0,
                title: None,
                cwd: None,
                icon_name: None,
            }],
            Some(&[local_only, active_remote]),
        );

        assert_eq!(terminals.len(), 1);
        assert_eq!(terminals[0].id(), "active-remote");
    }

    #[test]
    fn reconcile_synced_terminals_empty_remote_list_clears_local_list() {
        let local = RemoteTerminal::new("stale-local".to_string());

        let terminals = reconcile_synced_terminals(&[], Some(&[local]));

        assert!(terminals.is_empty());
    }
}
