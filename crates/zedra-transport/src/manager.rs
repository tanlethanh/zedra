use anyhow::Result;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use crate::discovery;
use crate::durable_queue::DurableQueue;
use crate::frame::{Frame, FrameType};
use crate::mdns::DiscoveryCache;
use crate::providers::lan::LanProvider;
use crate::providers::relay_ws::WsRelayProvider;
use crate::providers::tailscale::TailscaleProvider;
use crate::providers::TransportProvider;
use crate::signaling;
use crate::PeerInfo;
use zedra_rpc::Transport;

/// How often to check for transport liveness (seconds).
const HEALTH_CHECK_INTERVAL_SECS: u64 = 30;
/// If no data received within this window, consider the transport dead (seconds).
const HEALTH_TIMEOUT_SECS: u64 = 45;
/// How often to probe for a better transport when on relay (seconds).
const UPGRADE_PROBE_INTERVAL_SECS: u64 = 30;

/// Reconnection backoff schedule.
const BACKOFF_INITIAL_MS: u64 = 1_000;
const BACKOFF_MAX_MS: u64 = 30_000;
const BACKOFF_MULTIPLIER: u64 = 2;

/// Current state of the transport connection.
#[derive(Debug, Clone)]
pub enum TransportState {
    Discovering,
    Connected { transport_name: String },
    Switching { from: String, to: String },
    Reconnecting {
        generation: u32,
        last_connected: String,
    },
    Disconnected,
}

/// Manages transport discovery, selection, and bridging between an active
/// transport and the session's message channels.
///
/// Includes:
/// - Health monitoring (detects stale connections)
/// - Transport upgrade detection (switches from relay to LAN when available)
/// - L4 durable queue for exactly-once delivery across reconnects
/// - Never-give-up reconnection with exponential backoff
/// - Dynamic address refresh from coordination server and mDNS
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
    // L4 durable message queue
    durable_queue: DurableQueue,
    // Optional mDNS discovery cache (shared with background listener)
    mdns_cache: Option<Arc<Mutex<DiscoveryCache>>>,
    // Host device ID for coord server lookups (extracted from PeerInfo)
    host_device_id: Option<String>,
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
            durable_queue: DurableQueue::new(),
            mdns_cache: None,
            host_device_id: None,
        };

        (mgr, session_recv_rx, session_send_tx)
    }

    /// Set the mDNS discovery cache for dynamic LAN address updates.
    ///
    /// When set, the manager will check the mDNS cache for fresh LAN
    /// addresses during reconnection and upgrade probes.
    pub fn with_mdns_cache(mut self, cache: Arc<Mutex<DiscoveryCache>>) -> Self {
        self.mdns_cache = Some(cache);
        self
    }

    /// Set the host device ID for coordination server lookups.
    ///
    /// When set (along with coord_url in PeerInfo), the manager will
    /// query the coord server for fresh addresses during reconnection.
    pub fn with_host_device_id(mut self, device_id: String) -> Self {
        self.host_device_id = Some(device_id);
        self
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

        // WebSocket relay provider (priority 2) - fallback when direct connections fail
        if !peer_info.relay_url.is_empty() && !peer_info.relay_room.is_empty() {
            providers.push(Box::new(WsRelayProvider::new(
                peer_info.relay_url.clone(),
                peer_info.relay_room.clone(),
                peer_info.relay_secret.clone(),
            )));
        }

        providers
    }

    /// Refresh PeerInfo addresses from dynamic sources (coord server + mDNS).
    ///
    /// Merges newly discovered addresses with the existing ones from QR.
    /// Returns true if addresses were updated.
    async fn refresh_addresses(&mut self) -> bool {
        let mut updated = false;

        // 1. Query coordination server for fresh addresses
        if let (Some(coord_url), Some(device_id)) =
            (&self.peer_info.coord_url, &self.host_device_id)
        {
            match signaling::lookup_host(coord_url, device_id).await {
                Ok(discovery) => {
                    if discovery.online {
                        // Extract IPs from coord server addresses (strips port)
                        let coord_ips = signaling::extract_ips(&discovery.lan_addresses);
                        let new_addrs = merge_addresses(&self.peer_info.host_addrs, &coord_ips);
                        if new_addrs != self.peer_info.host_addrs {
                            log::info!(
                                "TransportManager: coord server updated addresses: {:?} -> {:?}",
                                self.peer_info.host_addrs,
                                new_addrs,
                            );
                            self.peer_info.host_addrs = new_addrs;
                            updated = true;
                        }

                        // Update tailscale address if discovered
                        let ts_ips = signaling::extract_ips(&discovery.tailscale_addresses);
                        if let Some(ts_addr) = ts_ips.first() {
                            if self.peer_info.tailscale_addr.as_deref() != Some(ts_addr) {
                                log::info!(
                                    "TransportManager: discovered tailscale address: {}",
                                    ts_addr
                                );
                                self.peer_info.tailscale_addr = Some(ts_addr.clone());
                                updated = true;
                            }
                        }
                    }
                }
                Err(e) => {
                    log::debug!("TransportManager: coord lookup failed: {}", e);
                }
            }
        }

        // 2. Check mDNS cache for LAN-discovered addresses
        if let (Some(cache), Some(device_id)) = (&self.mdns_cache, &self.host_device_id) {
            if let Ok(c) = cache.lock() {
                let mdns_addrs = c.addresses_for(device_id);
                if !mdns_addrs.is_empty() {
                    // mDNS addresses include port, extract IPs
                    let mdns_ips = signaling::extract_ips(&mdns_addrs);
                    let new_addrs = merge_addresses(&self.peer_info.host_addrs, &mdns_ips);
                    if new_addrs != self.peer_info.host_addrs {
                        log::info!(
                            "TransportManager: mDNS updated addresses: {:?} -> {:?}",
                            self.peer_info.host_addrs,
                            new_addrs,
                        );
                        self.peer_info.host_addrs = new_addrs;
                        updated = true;
                    }
                }
            }
        }

        updated
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
    /// - Reads from transport.recv() -> unwraps L4 -> forwards to session
    /// - Reads from session -> wraps in L4 -> forwards to transport
    /// - Monitors transport health (detects stale connections)
    /// - Probes for transport upgrades (relay -> LAN) periodically
    /// - On transport error, reconnects with L4 RESUME + message replay
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
                // Transport -> Session: read from transport, unwrap L4, forward
                recv_result = transport.recv() => {
                    match recv_result {
                        Ok(data) => {
                            self.last_recv_time = Instant::now();

                            // Try to decode as L4 frame; if it fails, pass raw
                            // (backward compat with pre-L4 peers)
                            match Frame::decode(&data) {
                                Ok(frame) if frame.frame_type == FrameType::Data => {
                                    match self.durable_queue.receive(&frame) {
                                        Ok(Some(payload)) => {
                                            if self.session_recv_tx.send(payload).await.is_err() {
                                                log::info!("TransportManager: session channel closed");
                                                return Ok(());
                                            }
                                            // Send standalone ACK if enough frames received
                                            if let Some(ack) = self.durable_queue.maybe_ack() {
                                                let _ = transport.send(&ack).await;
                                            }
                                        }
                                        Ok(None) => {
                                            // Duplicate or ACK-only — no app data
                                        }
                                        Err(e) => {
                                            log::warn!("TransportManager: L4 receive error: {}", e);
                                        }
                                    }
                                }
                                Ok(frame) if frame.frame_type == FrameType::Ack => {
                                    let _ = self.durable_queue.receive(&frame);
                                }
                                Ok(frame) if frame.frame_type == FrameType::Reset => {
                                    log::warn!("TransportManager: received RESET from peer");
                                    self.durable_queue.reset();
                                    // Notify session of reset via a synthetic error
                                    // (session should re-create terminals)
                                }
                                Ok(_) => {
                                    // Other frame types (RESUME) handled during handshake, not here
                                }
                                Err(_) => {
                                    // Not an L4 frame — pass raw data for backward compatibility
                                    if self.session_recv_tx.send(data).await.is_err() {
                                        log::info!("TransportManager: session channel closed");
                                        return Ok(());
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            log::warn!("TransportManager: transport recv error: {}", e);
                            match self.reconnect_loop().await {
                                Ok(new_transport) => {
                                    transport = new_transport;
                                    continue;
                                }
                                Err(e) => {
                                    // reconnect_loop only returns Err if session channel is closed
                                    log::error!("TransportManager: session closed during reconnect: {}", e);
                                    return Err(e);
                                }
                            }
                        }
                    }
                }

                // Session -> Transport: read from session, wrap in L4, forward
                msg = self.session_send_rx.recv() => {
                    match msg {
                        Some(data) => {
                            // Wrap in L4 DATA frame
                            let encoded = match self.durable_queue.enqueue(data.clone()) {
                                Some(e) => e,
                                None => {
                                    log::warn!("TransportManager: L4 backpressure, dropping message");
                                    continue;
                                }
                            };

                            if let Err(e) = transport.send(&encoded).await {
                                log::warn!("TransportManager: transport send error: {}", e);
                                match self.reconnect_loop().await {
                                    Ok(new_transport) => {
                                        transport = new_transport;
                                    }
                                    Err(e) => {
                                        log::error!("TransportManager: session closed during reconnect: {}", e);
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
                            "TransportManager: no data for {:.0}s (timeout {}s), reconnecting",
                            elapsed.as_secs_f64(),
                            HEALTH_TIMEOUT_SECS,
                        );
                        match self.reconnect_loop().await {
                            Ok(new_transport) => {
                                transport = new_transport;
                            }
                            Err(e) => {
                                log::error!("TransportManager: session closed during reconnect: {}", e);
                                return Err(e);
                            }
                        }
                    }
                }

                // Upgrade probe: if on relay, check if LAN became available
                _ = upgrade_interval.tick() => {
                    let current_name = self.current_transport_name();
                    // Probe for LAN upgrade when on any relay transport
                    if current_name.starts_with("relay") {
                        // Refresh addresses from coord server + mDNS before probing
                        self.refresh_addresses().await;

                        if !self.peer_info.host_addrs.is_empty() {
                            if let Ok(new_transport) = try_lan_upgrade(&self.peer_info).await {
                                log::info!("TransportManager: upgrading relay -> lan-tcp");
                                {
                                    let mut state = self.state.lock().unwrap();
                                    *state = TransportState::Switching {
                                        from: "relay".to_string(),
                                        to: "lan-tcp".to_string(),
                                    };
                                }
                                transport = new_transport;
                                self.last_recv_time = Instant::now();
                                {
                                    let mut state = self.state.lock().unwrap();
                                    *state = TransportState::Connected {
                                        transport_name: "lan-tcp".to_string(),
                                    };
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Never-give-up reconnection loop with exponential backoff.
    ///
    /// Keeps trying until a new transport is established, then performs
    /// L4 RESUME exchange and replays unACKed messages.
    ///
    /// Before each discovery attempt, refreshes addresses from the
    /// coordination server and mDNS cache to handle IP changes.
    ///
    /// Returns Err only if the session channel is closed (app shutting down).
    async fn reconnect_loop(&mut self) -> Result<Box<dyn Transport>> {
        let old_name = self.current_transport_name();
        let generation = self.durable_queue.generation();

        {
            let mut state = self.state.lock().unwrap();
            *state = TransportState::Reconnecting {
                generation,
                last_connected: old_name.clone(),
            };
        }

        log::info!(
            "TransportManager: reconnecting (gen {}, was: {})",
            generation,
            old_name,
        );

        let mut backoff_ms = BACKOFF_INITIAL_MS;

        loop {
            // Drain pending session messages during reconnect so channel doesn't fill
            while let Ok(_msg) = self.session_send_rx.try_recv() {
                if self.durable_queue.can_send() {
                    self.durable_queue.enqueue(_msg);
                }
            }

            // Refresh addresses from coord server + mDNS before discovery
            self.refresh_addresses().await;

            // Try discovery with updated addresses
            let providers = Self::build_providers(&self.peer_info);
            match discovery::discover(providers).await {
                Ok((mut new_transport, name)) => {
                    log::info!(
                        "TransportManager: reconnected via {} (gen {} -> {})",
                        name,
                        generation,
                        self.durable_queue.generation() + 1,
                    );

                    // Perform L4 RESUME exchange
                    if let Err(e) = self.perform_resume(&mut new_transport).await {
                        log::warn!("TransportManager: RESUME exchange failed: {}", e);
                        // Continue anyway — we'll try again on next reconnect
                    }

                    self.last_recv_time = Instant::now();
                    {
                        let mut state = self.state.lock().unwrap();
                        *state = TransportState::Connected {
                            transport_name: name,
                        };
                    }

                    return Ok(new_transport);
                }
                Err(e) => {
                    log::warn!(
                        "TransportManager: discovery failed (backoff {}ms): {}",
                        backoff_ms,
                        e,
                    );

                    // Check if session is still alive before sleeping
                    if self.session_recv_tx.is_closed() {
                        anyhow::bail!("session channel closed during reconnect");
                    }

                    tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                    backoff_ms = (backoff_ms * BACKOFF_MULTIPLIER).min(BACKOFF_MAX_MS);
                }
            }
        }
    }

    /// Perform L4 RESUME exchange on a newly connected transport.
    ///
    /// 1. Send our RESUME frame (generation + last_received_seq)
    /// 2. Read peer's RESUME frame
    /// 3. Replay unACKed messages the peer hasn't received
    async fn perform_resume(&mut self, transport: &mut Box<dyn Transport>) -> Result<()> {
        // Send our RESUME
        let resume = self.durable_queue.build_resume();
        transport.send(&resume).await?;

        // Read peer's RESUME (with timeout)
        let peer_data = tokio::time::timeout(Duration::from_secs(5), transport.recv())
            .await
            .map_err(|_| anyhow::anyhow!("RESUME timeout"))?
            .map_err(|e| anyhow::anyhow!("RESUME recv error: {}", e))?;

        let peer_frame = Frame::decode(&peer_data)?;

        match peer_frame.frame_type {
            FrameType::Resume => {
                let peer_resume = peer_frame.parse_resume_payload()?;
                log::info!(
                    "TransportManager: peer RESUME gen={} last_recv={}",
                    peer_resume.generation,
                    peer_resume.last_received_seq,
                );

                // Replay unACKed messages
                let replay_frames =
                    self.durable_queue.handle_peer_resume(peer_resume.last_received_seq);
                if !replay_frames.is_empty() {
                    log::info!(
                        "TransportManager: replaying {} unACKed messages",
                        replay_frames.len()
                    );
                    for frame_data in replay_frames {
                        transport.send(&frame_data).await?;
                    }
                }
            }
            FrameType::Reset => {
                log::warn!("TransportManager: peer sent RESET during RESUME");
                self.durable_queue.reset();
            }
            _ => {
                // Peer doesn't support L4 — that's OK, we degrade gracefully.
                // Treat as a non-L4 peer and pass this data through as raw.
                log::debug!(
                    "TransportManager: peer sent non-RESUME frame during handshake, assuming pre-L4"
                );
                // Bump generation anyway
                self.durable_queue.handle_peer_resume(0);

                // Forward this data to session if it looks like application data
                if peer_frame.frame_type == FrameType::Data {
                    if let Ok(Some(payload)) = self.durable_queue.receive(&peer_frame) {
                        let _ = self.session_recv_tx.send(payload).await;
                    }
                }
            }
        }

        Ok(())
    }
}

/// Merge new addresses into existing list, preserving order and deduplicating.
fn merge_addresses(existing: &[String], new: &[String]) -> Vec<String> {
    let mut merged = existing.to_vec();
    for addr in new {
        if !merged.contains(addr) {
            merged.push(addr.clone());
        }
    }
    merged
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_addresses_deduplicates() {
        let existing = vec!["10.0.0.1".to_string(), "10.0.0.2".to_string()];
        let new = vec!["10.0.0.2".to_string(), "10.0.0.3".to_string()];
        let merged = merge_addresses(&existing, &new);
        assert_eq!(merged, vec!["10.0.0.1", "10.0.0.2", "10.0.0.3"]);
    }

    #[test]
    fn merge_addresses_preserves_order() {
        let existing = vec!["10.0.0.1".to_string()];
        let new = vec!["10.0.0.5".to_string(), "10.0.0.3".to_string()];
        let merged = merge_addresses(&existing, &new);
        assert_eq!(merged, vec!["10.0.0.1", "10.0.0.5", "10.0.0.3"]);
    }

    #[test]
    fn merge_addresses_empty_new() {
        let existing = vec!["10.0.0.1".to_string()];
        let new: Vec<String> = vec![];
        let merged = merge_addresses(&existing, &new);
        assert_eq!(merged, vec!["10.0.0.1"]);
    }

    #[test]
    fn merge_addresses_empty_existing() {
        let existing: Vec<String> = vec![];
        let new = vec!["10.0.0.1".to_string()];
        let merged = merge_addresses(&existing, &new);
        assert_eq!(merged, vec!["10.0.0.1"]);
    }
}
