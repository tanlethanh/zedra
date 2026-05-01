use std::sync::{Arc, Mutex};
use tracing::*;

use iroh::EndpointAddr;
use tokio::sync::{Notify, broadcast, mpsc};
use tokio_util::sync::CancellationToken;
use zedra_rpc::ZedraPairingTicket;
use zedra_rpc::proto::{
    HostEvent, HostInfoSnapshot, SubscribeHostInfoReq, SubscribeReq, ZedraProto,
};

use crate::RemoteTerminal;
use crate::{
    ConnectEvent, Connector, SessionHandle, SessionState, session_runtime, signer::ClientSigner,
};

const AUTO_RECONNECT_MAX_ATTEMPTS: u32 = 3;

#[derive(Clone)]
pub struct Session {
    handle: SessionHandle,
    state: SessionState,
    event_tx: mpsc::Sender<ConnectEvent>,
    /// The event receiver. The caller is responsible for processing events.
    event_rx: Arc<Mutex<Option<mpsc::Receiver<ConnectEvent>>>>,
    /// The event broadcast sender for host events from subscription.
    host_event_tx: broadcast::Sender<HostEvent>,
    /// The host info broadcast sender for periodic resource snapshots.
    host_info_tx: broadcast::Sender<HostInfoSnapshot>,
    /// Signal to abort the session from external sources.
    abort_signal: Arc<Mutex<CancellationToken>>,
    /// Notify when the session connection is closed.
    closed_notify: Arc<Notify>,
}

impl Session {
    pub fn new() -> Self {
        let handle = SessionHandle::new();
        let state = SessionState::new();
        let (event_tx, event_rx) = mpsc::channel(64);
        let abort_signal = Arc::new(Mutex::new(CancellationToken::new()));
        let closed_notify = Arc::new(Notify::new());
        let (host_event_tx, _) = broadcast::channel(64);
        let (host_info_tx, _) = broadcast::channel(16);

        Self {
            handle,
            state,
            event_tx,
            event_rx: Arc::new(Mutex::new(Some(event_rx))),
            abort_signal,
            closed_notify,
            host_event_tx,
            host_info_tx,
        }
    }

    pub fn handle(&self) -> &SessionHandle {
        &self.handle
    }

    pub fn state(&self) -> &SessionState {
        &self.state
    }

    /// Take the event receiver. The caller is responsible for processing events
    /// via `SessionState::handle_event()` and calling `closed_notify()` on
    /// `ConnectionClosed` events for the reconnect loop to work.
    pub fn take_event_receiver(&self) -> Option<mpsc::Receiver<ConnectEvent>> {
        self.event_rx.lock().unwrap().take()
    }

    /// Arc<Notify> that must be notified when a `ConnectionClosed` event is
    /// processed. The reconnect loop in `connect()` awaits on this.
    pub fn closed_notify(&self) -> Arc<Notify> {
        self.closed_notify.clone()
    }

    pub fn subscribe_host_events(&self) -> broadcast::Receiver<HostEvent> {
        self.host_event_tx.subscribe()
    }

    pub fn subscribe_host_info(&self) -> broadcast::Receiver<HostInfoSnapshot> {
        self.host_info_tx.subscribe()
    }

    /// Start connection. Returns immediately; progress via `state()`.
    ///
    /// `on_connected` is called on the session runtime after successful connection,
    /// before signaling the main thread. Use it for terminal setup.
    pub fn connect<F>(
        &self,
        addr: EndpointAddr,
        ticket: Option<ZedraPairingTicket>,
        signer: Arc<dyn ClientSigner>,
        session_id: Option<String>,
        on_connected: F,
    ) where
        F: Fn(&SessionHandle) + Send + Sync + 'static,
    {
        let handle = self.handle.clone();
        let event_tx = self.event_tx.clone();
        let abort_signal = self.reset_abort_signal();
        let closed_notify = self.closed_notify.clone();
        let on_connected = Arc::new(on_connected);
        let host_event_tx = self.host_event_tx.clone();
        let host_info_tx = self.host_info_tx.clone();

        // Store credentials on handle
        handle.set_signer(signer.clone());
        handle.set_endpoint_addr(addr.clone());
        handle.set_user_disconnect(false);
        if let Some(ref t) = ticket {
            handle.set_pending_ticket(t.clone());
        }
        handle.set_session_id(session_id.clone());

        session_runtime().spawn(async move {
            let mut connector = Connector::new(event_tx);
            let mut ticket = ticket;
            let mut session_id = session_id;
            let mut session_token = handle.session_token();
            let mut reconnect_reason = None;

            loop {
                if abort_signal.is_cancelled() || handle.user_disconnect() {
                    connector.abort();
                    break;
                }

                let existing_terminals = handle.terminals().clone();
                let result = if let Some(reason) = reconnect_reason.take() {
                    let max_attempts = AUTO_RECONNECT_MAX_ATTEMPTS;
                    info!("reconnect to {addr:?}, session: {session_id:?} reason {reason:?} max_attempts {max_attempts}",);
                    tokio::select! {
                        _ = abort_signal.cancelled() => {
                            connector.abort();
                            break;
                        }
                        result = connector.reconnect_loop(
                            addr.clone(),
                            None,
                            signer.clone(),
                            session_id.clone(),
                            session_token,
                            reason,
                            max_attempts,
                            Some(existing_terminals),
                        ) => result,
                    }
                } else {
                    info!("start connect to {addr:?}, session: {session_id:?}");
                    tokio::select! {
                        _ = abort_signal.cancelled() => {
                            connector.abort();
                            break;
                        }
                        result = connector.connect(
                            addr.clone(),
                            ticket.as_ref(),
                            signer.clone(),
                            session_id.clone(),
                            session_token,
                            Some(existing_terminals),
                        ) => result,
                    }
                };

                match result {
                    Ok((client, sync, terminals)) => {
                        handle.set_user_disconnect(false);
                        handle.set_rpc_client(client.clone());
                        handle.set_session_id(Some(sync.session_id.clone()));
                        handle.set_session_token(Some(sync.session_token));
                        handle.set_terminals(terminals);
                        on_connected(&handle);

                        {
                            let subscribe_handle = handle.clone();
                            let subscribe_client = client.clone();
                            let subscribe_abort = abort_signal.clone();
                            let subscribe_closed = closed_notify.clone();
                            let subscribe_events = host_event_tx.clone();

                            // Subscribe to host events and and broadcast them via host_event_tx.
                            session_runtime().spawn(async move {
                                let mut host_events = match subscribe_client
                                    .server_streaming(SubscribeReq {}, 32)
                                    .await
                                {
                                    Ok(rx) => rx,
                                    Err(e) => {
                                        warn!("subscribe failed: {}", e);
                                        return;
                                    }
                                };

                                loop {
                                    tokio::select! {
                                        _ = subscribe_abort.cancelled() => break,
                                        _ = subscribe_closed.notified() => break,
                                        event = host_events.recv() => {
                                            let event = match event {
                                                Ok(Some(event)) => event,
                                                Ok(None) => {
                                                    warn!("host event recv channel closed");
                                                    break;
                                                },
                                                Err(e) => {
                                                    warn!("host event recv failed: {}", e);
                                                    break;
                                                }
                                            };
                                            Self::handle_host_event(
                                                &subscribe_handle,
                                                &subscribe_client,
                                                &subscribe_events,
                                                event,
                                            ).await;
                                        }
                                    }
                                }
                            });
                        }

                        {
                            let subscribe_client = client.clone();
                            let subscribe_abort = abort_signal.clone();
                            let subscribe_closed = closed_notify.clone();
                            let subscribe_info = host_info_tx.clone();

                            session_runtime().spawn(async move {
                                let mut snapshots = match subscribe_client
                                    .server_streaming(SubscribeHostInfoReq {}, 4)
                                    .await
                                {
                                    Ok(rx) => rx,
                                    Err(e) => {
                                        warn!("host info subscribe failed: {}", e);
                                        return;
                                    }
                                };

                                loop {
                                    tokio::select! {
                                        _ = subscribe_abort.cancelled() => break,
                                        _ = subscribe_closed.notified() => break,
                                        snapshot = snapshots.recv() => {
                                            let snapshot = match snapshot {
                                                Ok(Some(snapshot)) => snapshot,
                                                Ok(None) => {
                                                    warn!("host info recv channel closed");
                                                    break;
                                                },
                                                Err(e) => {
                                                    warn!("host info recv failed: {}", e);
                                                    break;
                                                }
                                            };
                                            let _ = subscribe_info.send(snapshot);
                                        }
                                    }
                                }
                            });
                        }

                        ticket = None;
                        session_id = handle.session_id();
                        session_token = handle.session_token();

                        tokio::select! {
                            _ = abort_signal.cancelled() => {
                                connector.abort();
                                break;
                            }
                            _ = closed_notify.notified() => {
                            }
                        }

                        handle.clear_rpc_client();
                        if abort_signal.is_cancelled() || handle.user_disconnect() {
                            connector.abort();
                            break;
                        }

                        reconnect_reason = Some(crate::ReconnectReason::ConnectionLost);
                    }
                    Err(e) => {
                        connector.abort();
                        error!("connect failed: {}", e);
                        break;
                    }
                }
            }
        });
    }

    /// Disconnect and clear session state.
    pub fn disconnect(&self) {
        self.cancel_abort_signal();
        self.handle.clear_session();
    }

    fn reset_abort_signal(&self) -> CancellationToken {
        let mut guard = self.abort_signal.lock().unwrap();
        guard.cancel();
        *guard = CancellationToken::new();
        guard.clone()
    }

    fn cancel_abort_signal(&self) {
        self.abort_signal.lock().unwrap().cancel();
    }

    pub async fn handle_host_event(
        handle: &SessionHandle,
        client: &irpc::Client<ZedraProto>,
        host_event_tx: &broadcast::Sender<HostEvent>,
        event: HostEvent,
    ) {
        match &event {
            HostEvent::TerminalCreated { id, launch_cmd } => {
                info!(
                    "HostEvent: terminal created id={} launch_cmd={:?}",
                    id, launch_cmd,
                );
                let terminal = RemoteTerminal::new(id.clone());
                handle.add_terminal(terminal.clone());
                if let Err(e) = terminal.attach_remote(client).await {
                    warn!(
                        "Failed to attach host-created terminal {}: {e}",
                        terminal.id()
                    );
                }
            }
            HostEvent::GitChanged => {
                info!("HostEvent: git changed");
            }
            HostEvent::FsChanged { path } => {
                info!("HostEvent: fs changed path={path}");
            }
        }

        let _ = host_event_tx.send(event);
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn subscribe_host_events_fans_out_to_multiple_receivers() {
        let session = Session::new();
        let mut rx1 = session.subscribe_host_events();
        let mut rx2 = session.subscribe_host_events();

        let _ = session.host_event_tx.send(HostEvent::GitChanged);

        assert!(matches!(rx1.recv().await, Ok(HostEvent::GitChanged)));
        assert!(matches!(rx2.recv().await, Ok(HostEvent::GitChanged)));
    }

    #[tokio::test]
    async fn subscribe_host_info_fans_out_to_multiple_receivers() {
        let session = Session::new();
        let mut rx1 = session.subscribe_host_info();
        let mut rx2 = session.subscribe_host_info();

        let snapshot = HostInfoSnapshot {
            captured_at_ms: 1,
            cpu_usage_percent: 12.5,
            cpu_count: 4,
            memory_used_bytes: 1024,
            memory_total_bytes: 2048,
            swap_used_bytes: 0,
            swap_total_bytes: 0,
            system_uptime_secs: 99,
            batteries: Vec::new(),
        };
        let _ = session.host_info_tx.send(snapshot.clone());

        assert_eq!(rx1.recv().await.unwrap(), snapshot);
        assert_eq!(rx2.recv().await.unwrap(), snapshot);
    }
}
