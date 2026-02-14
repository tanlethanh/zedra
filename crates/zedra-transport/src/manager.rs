use anyhow::Result;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use crate::discovery;
use crate::providers::lan::LanProvider;
use crate::providers::relay::RelayProvider;
use crate::providers::tailscale::TailscaleProvider;
use crate::providers::TransportProvider;
use crate::PeerInfo;
use zedra_rpc::Transport;

/// How often to check for transport liveness (seconds).
const HEALTH_CHECK_INTERVAL_SECS: u64 = 30;
/// If no data received within this window, consider the transport dead (seconds).
const HEALTH_TIMEOUT_SECS: u64 = 45;
/// How often to probe for a better transport when on relay (seconds).
const UPGRADE_PROBE_INTERVAL_SECS: u64 = 30;

/// Current state of the transport connection.
#[derive(Debug, Clone)]
pub enum TransportState {
    Discovering,
    Connected { transport_name: String },
    Switching { from: String, to: String },
    Disconnected,
}

/// Manages transport discovery, selection, and bridging between an active
/// transport and the session's message channels.
///
/// Includes health monitoring (detects stale connections) and transport upgrade
/// detection (switches from relay to LAN when available).
///
/// Usage:
/// ```ignore
/// let (mut mgr, recv_rx, send_tx) = TransportManager::new(peer_info);
/// // Pass recv_rx and send_tx to RpcClient::spawn_from_channels
/// mgr.connect().await?;
/// mgr.run().await?;
/// ```
pub struct TransportManager {
    peer_info: PeerInfo,
    state: Arc<Mutex<TransportState>>,
    transport: Option<Box<dyn Transport>>,
    // session writes here, manager reads
    session_send_rx: mpsc::Receiver<Vec<u8>>,
    // manager writes here, session reads
    session_recv_tx: mpsc::Sender<Vec<u8>>,
    // Health monitoring
    last_recv_time: Instant,
    consecutive_failures: u32,
    // Message buffering during transport switch
    pending_outgoing: Vec<Vec<u8>>,
}

impl TransportManager {
    /// Create a new TransportManager.
    ///
    /// Returns (manager, recv_rx_for_session, send_tx_for_session):
    /// - `recv_rx`: session reads incoming frames from this channel
    /// - `send_tx`: session sends outgoing frames into this channel
    pub fn new(peer_info: PeerInfo) -> (Self, mpsc::Receiver<Vec<u8>>, mpsc::Sender<Vec<u8>>) {
        let (session_send_tx, session_send_rx) = mpsc::channel(64);
        let (session_recv_tx, session_recv_rx) = mpsc::channel(64);

        let mgr = Self {
            peer_info,
            state: Arc::new(Mutex::new(TransportState::Disconnected)),
            transport: None,
            session_send_rx,
            session_recv_tx,
            last_recv_time: Instant::now(),
            consecutive_failures: 0,
            pending_outgoing: Vec::new(),
        };

        (mgr, session_recv_rx, session_send_tx)
    }

    /// Get a handle to the current transport state.
    pub fn state(&self) -> Arc<Mutex<TransportState>> {
        self.state.clone()
    }

    /// Build the list of transport providers from PeerInfo.
    fn build_providers(peer_info: &PeerInfo) -> Vec<Box<dyn TransportProvider>> {
        let mut providers: Vec<Box<dyn TransportProvider>> = Vec::new();

        // LAN TCP provider (priority 0) - only if we have addresses
        if !peer_info.host_addrs.is_empty() {
            providers.push(Box::new(LanProvider::new(
                peer_info.host_addrs.clone(),
                peer_info.port,
            )));
        }

        // Tailscale provider (priority 1) - only if tailscale addr is available
        if let Some(ref ts_addr) = peer_info.tailscale_addr {
            providers.push(Box::new(TailscaleProvider::new(
                ts_addr.clone(),
                peer_info.port,
            )));
        }

        // Relay provider (priority 2) - always available as fallback
        providers.push(Box::new(RelayProvider::new(
            peer_info.relay_url.clone(),
            peer_info.relay_room.clone(),
            peer_info.relay_secret.clone(),
        )));

        providers
    }

    /// Run discovery and connect the first transport.
    pub async fn connect(&mut self) -> Result<()> {
        {
            let mut state = self.state.lock().unwrap();
            *state = TransportState::Discovering;
        }

        let providers = Self::build_providers(&self.peer_info);
        match discovery::discover(providers).await {
            Ok((transport, name)) => {
                log::info!("TransportManager: connected via {}", name);
                self.transport = Some(transport);
                self.last_recv_time = Instant::now();
                self.consecutive_failures = 0;
                let mut state = self.state.lock().unwrap();
                *state = TransportState::Connected {
                    transport_name: name,
                };
                Ok(())
            }
            Err(e) => {
                let mut state = self.state.lock().unwrap();
                *state = TransportState::Disconnected;
                Err(e)
            }
        }
    }

    /// Get the current transport name from the state.
    fn current_transport_name(&self) -> String {
        let state = self.state.lock().unwrap();
        match &*state {
            TransportState::Connected { transport_name } => transport_name.clone(),
            _ => "unknown".to_string(),
        }
    }

    /// Main loop: bridges the active transport to the session channels.
    ///
    /// Concurrently:
    /// - Reads from transport.recv() and forwards to session_recv_tx
    /// - Reads from session_send_rx and forwards to transport.send()
    /// - Monitors transport health (detects stale connections)
    /// - Probes for transport upgrades (relay -> LAN) periodically
    /// - On transport error, attempts reconnection with message buffering
    pub async fn run(mut self) -> Result<()> {
        let mut transport = self
            .transport
            .take()
            .ok_or_else(|| anyhow::anyhow!("not connected; call connect() first"))?;

        let mut health_interval =
            tokio::time::interval(Duration::from_secs(HEALTH_CHECK_INTERVAL_SECS));
        let mut upgrade_interval =
            tokio::time::interval(Duration::from_secs(UPGRADE_PROBE_INTERVAL_SECS));

        // Skip the immediate first tick so timers don't fire on startup
        health_interval.tick().await;
        upgrade_interval.tick().await;

        loop {
            tokio::select! {
                // Transport -> Session: read from transport, forward to session
                recv_result = transport.recv() => {
                    match recv_result {
                        Ok(data) => {
                            self.last_recv_time = Instant::now();
                            self.consecutive_failures = 0;
                            if self.session_recv_tx.send(data).await.is_err() {
                                log::info!("TransportManager: session channel closed, shutting down");
                                return Ok(());
                            }
                        }
                        Err(e) => {
                            log::warn!("TransportManager: transport recv error: {}", e);
                            self.consecutive_failures += 1;
                            match self.try_reconnect().await {
                                Ok(new_transport) => {
                                    transport = new_transport;
                                    // Replay buffered outgoing messages
                                    for msg in self.pending_outgoing.drain(..) {
                                        if let Err(e) = transport.send(&msg).await {
                                            log::error!("TransportManager: replay send failed: {}", e);
                                            return Err(e);
                                        }
                                    }
                                    continue;
                                }
                                Err(e) => {
                                    log::error!("TransportManager: reconnection failed: {}", e);
                                    let mut state = self.state.lock().unwrap();
                                    *state = TransportState::Disconnected;
                                    return Err(e);
                                }
                            }
                        }
                    }
                }

                // Session -> Transport: read from session, forward to transport
                msg = self.session_send_rx.recv() => {
                    match msg {
                        Some(data) => {
                            if let Err(e) = transport.send(&data).await {
                                log::warn!("TransportManager: transport send error: {}", e);
                                // Buffer the failed message for replay after reconnect
                                self.pending_outgoing.push(data);
                                // Drain any additional pending messages from the channel
                                while let Ok(extra) = self.session_send_rx.try_recv() {
                                    self.pending_outgoing.push(extra);
                                }
                                self.consecutive_failures += 1;
                                match self.try_reconnect().await {
                                    Ok(new_transport) => {
                                        transport = new_transport;
                                        // Replay buffered outgoing messages
                                        for msg in self.pending_outgoing.drain(..) {
                                            if let Err(e) = transport.send(&msg).await {
                                                log::error!("TransportManager: replay send failed: {}", e);
                                                return Err(e);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        log::error!("TransportManager: reconnection failed: {}", e);
                                        let mut state = self.state.lock().unwrap();
                                        *state = TransportState::Disconnected;
                                        return Err(e);
                                    }
                                }
                            }
                        }
                        None => {
                            log::info!("TransportManager: session send channel closed, shutting down");
                            return Ok(());
                        }
                    }
                }

                // Health check: detect stale connections
                _ = health_interval.tick() => {
                    let elapsed = self.last_recv_time.elapsed();
                    if elapsed > Duration::from_secs(HEALTH_TIMEOUT_SECS) {
                        log::warn!(
                            "TransportManager: no data received for {:.0}s (timeout {}s), attempting reconnection",
                            elapsed.as_secs_f64(),
                            HEALTH_TIMEOUT_SECS,
                        );
                        self.consecutive_failures += 1;
                        match self.try_reconnect().await {
                            Ok(new_transport) => {
                                transport = new_transport;
                                // Replay buffered outgoing messages
                                for msg in self.pending_outgoing.drain(..) {
                                    if let Err(e) = transport.send(&msg).await {
                                        log::error!("TransportManager: replay send failed: {}", e);
                                        return Err(e);
                                    }
                                }
                            }
                            Err(e) => {
                                if self.consecutive_failures >= 3 {
                                    log::error!(
                                        "TransportManager: {} consecutive failures, giving up: {}",
                                        self.consecutive_failures,
                                        e,
                                    );
                                    let mut state = self.state.lock().unwrap();
                                    *state = TransportState::Disconnected;
                                    return Err(e);
                                }
                                log::warn!(
                                    "TransportManager: reconnection failed ({} failures): {}",
                                    self.consecutive_failures,
                                    e,
                                );
                            }
                        }
                    }
                }

                // Upgrade probe: if on relay, check if LAN became available
                _ = upgrade_interval.tick() => {
                    let current_name = self.current_transport_name();
                    if current_name == "relay" && !self.peer_info.host_addrs.is_empty() {
                        match try_lan_upgrade(&self.peer_info).await {
                            Ok(new_transport) => {
                                log::info!(
                                    "TransportManager: upgrading transport from relay to lan-tcp"
                                );
                                {
                                    let mut state = self.state.lock().unwrap();
                                    *state = TransportState::Switching {
                                        from: "relay".to_string(),
                                        to: "lan-tcp".to_string(),
                                    };
                                }
                                transport = new_transport;
                                self.last_recv_time = Instant::now();
                                self.consecutive_failures = 0;
                                {
                                    let mut state = self.state.lock().unwrap();
                                    *state = TransportState::Connected {
                                        transport_name: "lan-tcp".to_string(),
                                    };
                                }
                                // Replay any buffered outgoing messages
                                for msg in self.pending_outgoing.drain(..) {
                                    if let Err(e) = transport.send(&msg).await {
                                        log::error!("TransportManager: replay send failed after upgrade: {}", e);
                                        return Err(e);
                                    }
                                }
                            }
                            Err(_) => {
                                // LAN not available yet, will retry next interval
                            }
                        }
                    }
                }
            }
        }
    }

    /// Attempt to reconnect by running discovery again.
    /// Drains any pending session messages into the outgoing buffer during the switch.
    async fn try_reconnect(&mut self) -> Result<Box<dyn Transport>> {
        let old_name = self.current_transport_name();

        log::info!(
            "TransportManager: attempting reconnection (was: {})",
            old_name
        );

        {
            let mut state = self.state.lock().unwrap();
            *state = TransportState::Switching {
                from: old_name.clone(),
                to: "discovering".to_string(),
            };
        }

        // Buffer any messages that arrive during reconnection
        while let Ok(msg) = self.session_send_rx.try_recv() {
            self.pending_outgoing.push(msg);
        }

        let providers = Self::build_providers(&self.peer_info);
        let (transport, name) = discovery::discover(providers).await?;

        log::info!(
            "TransportManager: reconnected via {} (was: {})",
            name,
            old_name
        );
        self.last_recv_time = Instant::now();
        self.consecutive_failures = 0;
        {
            let mut state = self.state.lock().unwrap();
            *state = TransportState::Connected {
                transport_name: name,
            };
        }

        Ok(transport)
    }
}

/// Try to establish a LAN connection for transport upgrade.
/// Uses a short timeout since this is a background probe.
async fn try_lan_upgrade(peer_info: &PeerInfo) -> Result<Box<dyn Transport>> {
    let provider = LanProvider::new(peer_info.host_addrs.clone(), peer_info.port);
    // Quick probe first to avoid expensive full connect if unreachable
    if !provider.probe().await {
        anyhow::bail!("LAN probe: no reachable address");
    }
    // Full connect (with framing handshake)
    tokio::time::timeout(Duration::from_secs(2), provider.connect())
        .await
        .map_err(|_| anyhow::anyhow!("LAN upgrade: connect timeout"))?
}
