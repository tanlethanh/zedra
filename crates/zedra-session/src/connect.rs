use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ed25519_dalek::VerifyingKey;
use iroh::{
    Endpoint, EndpointAddr, NetReport, PublicKey, RelayConfig, RelayMap, RelayMode,
    address_lookup::PkarrResolver,
    endpoint::{Connection, PathInfo, QuicTransportConfig},
};
use iroh_relay::RelayQuicConfig;
use tokio::{sync::mpsc, task::JoinHandle, time::Instant};
use tokio_util::sync::CancellationToken;
use zedra_rpc::{
    ZEDRA_RELAY_URLS, ZedraPairingTicket, compute_registration_hmac,
    proto::{
        AuthProveReq, AuthProveResult, ConnectReq, ConnectResult, RegisterReq, RegisterResult,
        SyncSessionResult, ZEDRA_ALPN, ZedraProto,
    },
};

use crate::signer::ClientSigner;
use crate::state::{AuthOutcome, ConnectError, ReconnectReason};

pub struct ConnectConfig {
    alpn: Vec<u8>,
    alpns: Vec<Vec<u8>>,
    relay_urls: Vec<String>,
    keep_alive_interval: Duration,
    max_idle_timeout: Duration,
    path_keep_alive_interval: Duration,
    path_max_idle_timeout: Duration,
}

impl Default for ConnectConfig {
    fn default() -> Self {
        // Tighten QUIC timeouts for fast disconnect detection.
        // PING frames are tiny UDP packets — cheap even on mobile.
        // Default iroh heartbeat is 5s; we lower to 2s so bytes_recv ticks
        // every ~2 s and the stale counter in the session tab reacts quickly.
        Self {
            alpn: ZEDRA_ALPN.to_vec(),
            alpns: vec![ZEDRA_ALPN.to_vec()],
            relay_urls: ZEDRA_RELAY_URLS.iter().map(|u| u.to_string()).collect(),
            keep_alive_interval: Duration::from_secs(2),
            max_idle_timeout: Duration::from_secs(30),
            path_keep_alive_interval: Duration::from_secs(2),
            path_max_idle_timeout: Duration::from_secs(30),
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
        fetch_ms: u64,
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
        bytes_sent_total: u64,
        bytes_recv_total: u64,
    },
    PathUpgraded {
        prev_path: PathInfo,
        new_path: PathInfo,
    },
    NoActivePath,

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
        fatal_error: Option<&'static str>,
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
    ) -> Result<(irpc::Client<ZedraProto>, SyncSessionResult), ConnectError> {
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
        let (sync, outcome, is_first_pairing) = self
            .proceed_bootstrap_session(
                &client,
                ticket,
                signer.as_ref(),
                &endpoint_id,
                session_id,
                session_token,
            )
            .await?;
        let auth_ms = t_auth.elapsed().as_millis() as u64;
        self.emit(ConnectEvent::AuthComplete {
            auth_ms,
            outcome,
            is_first_pairing,
        });

        // Phase: Sync complete
        // TODO: sync should handle workspace data fetching and terminal resuming
        self.emit(ConnectEvent::SyncComplete {
            sync: sync.clone(),
            fetch_ms: 0,
        });

        // Connected
        let total_ms = self
            .started_at
            .map(|t| t.elapsed().as_millis() as u64)
            .unwrap_or(0);
        self.emit(ConnectEvent::Connected { total_ms });

        Ok((client, sync))
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
        tracing::info!(
            "iroh client endpoint bound: {} in {}ms",
            local_node_id,
            binding_ms
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
                let err = ConnectError::QuicConnectFailed(e.to_string());
                self.emit(ConnectEvent::Failed { error: err.clone() });
                err
            })?;

        let hole_punch_ms = t.elapsed().as_millis() as u64;
        let remote_node_id = conn.remote_id().fmt_short().to_string();
        let alpn = String::from_utf8_lossy(conn.alpn()).to_string();
        tracing::info!(
            "iroh: connected to {}, alpn: {} in {}ms",
            remote_node_id,
            alpn,
            hole_punch_ms
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
                tracing::info!(
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
                tracing::info!("authenticated, session={}", session_id);
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
                    _ = abort_signal.cancelled() => break,
                    res = addr_watcher.updated() => {
                        if res.is_err() { break; }
                        let endpoint_addr = addr_watcher.get();
                        let _ = tx.send(ConnectEvent::EndpointAddrChanged { endpoint_addr }).await;
                    }
                    res = report_watcher.updated() => {
                        if res.is_err() { break; }
                        if let Some(net_report) = report_watcher.get() {
                            let _ = tx.send(ConnectEvent::NetReport { net_report }).await;
                        }
                    }
                }
            }
            tracing::debug!("local endpoint watcher stopped");
        })
    }

    /// Watch remote path changes (RTT, bytes, path upgrades).
    pub fn spawn_remote_paths_watcher(&self, conn: Connection) -> JoinHandle<()> {
        use iroh::Watcher;
        let tx = self.tx.clone();
        let abort_signal = self.abort_signal.clone();

        tokio::spawn(async move {
            let mut paths = conn.paths();
            let mut prev_is_direct = false;

            loop {
                let path_list = paths.get();
                if let Some(path) = path_list.iter().find(|p| p.is_selected()) {
                    let is_direct = path.is_ip();
                    let stats = path.stats();

                    // Detect relay → direct upgrade
                    if !prev_is_direct && is_direct {
                        if let Some(prev) = path_list.iter().find(|p| !p.is_ip()) {
                            let _ = tx
                                .send(ConnectEvent::PathUpgraded {
                                    prev_path: prev.clone(),
                                    new_path: path.clone(),
                                })
                                .await;
                        }
                    }
                    prev_is_direct = is_direct;

                    let _ = tx
                        .send(ConnectEvent::PathReport {
                            path: path.clone(),
                            // TODO: need to aggregate across all paths
                            bytes_sent_total: stats.udp_tx.bytes,
                            bytes_recv_total: stats.udp_rx.bytes,
                        })
                        .await;
                } else {
                    let _ = tx.send(ConnectEvent::NoActivePath).await;
                }

                tokio::select! {
                    _ = abort_signal.cancelled() => break,
                    _ = paths.updated() => {}
                    _ = tokio::time::sleep(Duration::from_secs(2)) => {}
                }
            }
            tracing::debug!("remote paths watcher stopped");
        })
    }

    /// Watch for connection close and emit ConnectionClosed event.
    pub fn spawn_connection_closed_watcher(&self, conn: Connection) -> JoinHandle<()> {
        let tx = self.tx.clone();
        let abort_signal = self.abort_signal.clone();

        tokio::spawn(async move {
            tokio::select! {
                _ = abort_signal.cancelled() => {}
                _ = conn.closed() => {
                    tracing::info!("iroh connection closed");
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
    ) -> Result<(irpc::Client<ZedraProto>, SyncSessionResult), ConnectError> {
        let reconnect_start = Instant::now();
        self.emit(ConnectEvent::ReconnectStarted {
            reason: reason.clone(),
        });

        for attempt in 1..=max_attempts {
            let delay_secs = std::cmp::min(1u64 << (attempt - 1), 30);

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
                        fatal_error: Some(e.label()),
                    });
                    return Err(e);
                }
                Err(e) => {
                    tracing::warn!("reconnect attempt {} failed: {}", attempt, e);
                }
            }
        }

        let err = ConnectError::HostUnreachable;
        self.emit(ConnectEvent::ReconnectExhausted {
            attempts: max_attempts,
            elapsed_ms: reconnect_start.elapsed().as_millis() as u64,
            fatal_error: None,
        });
        Err(err)
    }
}
