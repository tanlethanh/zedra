use std::sync::{Arc, Mutex};

use iroh::EndpointAddr;
use tokio::sync::{Notify, mpsc};
use tokio_util::sync::CancellationToken;
use zedra_rpc::ZedraPairingTicket;
use zedra_rpc::proto::{HostEvent, SubscribeReq, SyncSessionResult, ZedraProto};

use crate::{
    ConnectError, ConnectEvent, Connector, SessionHandle, SessionState, session_runtime,
    signer::ClientSigner,
};

#[derive(Clone)]
pub struct Session {
    handle: SessionHandle,
    state: SessionState,
    event_tx: mpsc::Sender<ConnectEvent>,
    abort_signal: Arc<Mutex<CancellationToken>>,
    closed_notify: Arc<Notify>,
}

impl Session {
    pub fn new() -> Self {
        let handle = SessionHandle::new();
        let state = SessionState::new();
        let (event_tx, mut event_rx) = mpsc::channel(64);
        let abort_signal = Arc::new(Mutex::new(CancellationToken::new()));
        let closed_notify = Arc::new(Notify::new());

        {
            let state = state.clone();
            let closed_notify = closed_notify.clone();
            session_runtime().spawn(async move {
                while let Some(event) = event_rx.recv().await {
                    let is_closed = matches!(event, ConnectEvent::ConnectionClosed);
                    state.handle_event(event);
                    if is_closed {
                        closed_notify.notify_one();
                    }
                }
            });
        }

        Self {
            handle,
            state,
            event_tx,
            abort_signal,
            closed_notify,
        }
    }

    pub fn handle(&self) -> &SessionHandle {
        &self.handle
    }

    pub fn state(&self) -> &SessionState {
        &self.state
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
        let state = self.state.clone();
        let event_tx = self.event_tx.clone();
        let abort_signal = self.reset_abort_signal();
        let closed_notify = self.closed_notify.clone();
        let on_connected = Arc::new(on_connected);

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

                let result = match reconnect_reason.take() {
                    Some(reason) => {
                        connector
                            .reconnect_loop(
                                addr.clone(),
                                None,
                                signer.clone(),
                                session_id.clone(),
                                session_token,
                                reason,
                                10,
                            )
                            .await
                    }
                    None => {
                        connector
                            .connect(
                                addr.clone(),
                                ticket.as_ref(),
                                signer.clone(),
                                session_id.clone(),
                                session_token,
                            )
                            .await
                    }
                };

                match result {
                    Ok((client, sync)) => {
                        if Self::finish_connection(
                            &handle,
                            &state,
                            client,
                            sync,
                            on_connected.clone(),
                            abort_signal.clone(),
                        )
                        .await
                        .is_err()
                        {
                            break;
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
                        state.notify();
                        if abort_signal.is_cancelled() || handle.user_disconnect() {
                            connector.abort();
                            break;
                        }

                        reconnect_reason = Some(crate::ReconnectReason::ConnectionLost);
                    }
                    Err(e) => {
                        tracing::error!("connect failed: {}", e);
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
        self.state.notify();
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

    async fn finish_connection(
        handle: &SessionHandle,
        state: &SessionState,
        client: irpc::Client<ZedraProto>,
        sync: SyncSessionResult,
        on_connected: Arc<dyn Fn(&SessionHandle) + Send + Sync>,
        abort_signal: CancellationToken,
    ) -> Result<(), ConnectError> {
        handle.set_user_disconnect(false);
        handle.set_rpc_client(client.clone());
        handle.set_session_id(Some(sync.session_id.clone()));
        handle.set_session_token(Some(sync.session_token));
        handle.sync_terminals(&sync.terminals);
        handle
            .reattach_terminals()
            .await
            .map_err(|e| ConnectError::Other(e.to_string()))?;

        Self::spawn_host_event_subscription(
            handle.clone(),
            state.clone(),
            client,
            abort_signal,
        );

        on_connected(handle);
        state.notify();
        Ok(())
    }

    fn spawn_host_event_subscription(
        handle: SessionHandle,
        state: SessionState,
        client: irpc::Client<ZedraProto>,
        abort_signal: CancellationToken,
    ) {
        session_runtime().spawn(async move {
            let stream_fut = client.server_streaming::<SubscribeReq, HostEvent>(SubscribeReq {}, 32);
            let mut rx = match tokio::time::timeout(std::time::Duration::from_secs(10), stream_fut)
                .await
            {
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

            tracing::info!("Subscribed to host events");
            loop {
                tokio::select! {
                    _ = abort_signal.cancelled() => break,
                    event = rx.recv() => {
                        match event {
                            Ok(Some(event)) => {
                                Self::handle_host_event(&handle, &state, &client, event).await;
                            }
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
                }
            }
        });
    }

    async fn handle_host_event(
        handle: &SessionHandle,
        state: &SessionState,
        client: &irpc::Client<ZedraProto>,
        event: HostEvent,
    ) {
        match event {
            HostEvent::TerminalCreated { id, launch_cmd } => {
                tracing::info!(
                    "HostEvent: terminal created id={} launch_cmd={:?}",
                    id,
                    launch_cmd,
                );
                let terminal = handle
                    .terminal(&id)
                    .unwrap_or_else(|| create_host_terminal(handle, id));
                if let Err(e) = handle.attach_terminal(client, &terminal).await {
                    tracing::warn!("Failed to attach host-created terminal {}: {e}", terminal.id);
                }
                state.notify();
            }
            HostEvent::GitChanged => {
                handle.set_git_needs_refresh();
                state.notify();
            }
            HostEvent::FsChanged { path } => {
                handle.add_fs_changed(path);
                state.notify();
            }
        }
    }
}

fn create_host_terminal(
    handle: &SessionHandle,
    id: String,
) -> Arc<crate::terminal::RemoteTerminal> {
    let terminal = crate::terminal::RemoteTerminal::new(id);
    handle.add_terminal(terminal.clone());
    terminal
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of a successful connection, passed to on_connected callback.
pub struct ConnectResult {
    pub session_id: String,
    pub terminal_ids: Vec<String>,
}

/// Builder for configuring and starting a session connection.
pub struct SessionBuilder {
    session: Session,
    addr: Option<EndpointAddr>,
    ticket: Option<ZedraPairingTicket>,
    signer: Option<Arc<dyn ClientSigner>>,
    session_id: Option<String>,
    session_token: Option<[u8; 32]>,
}

impl SessionBuilder {
    pub fn new() -> Self {
        Self {
            session: Session::new(),
            addr: None,
            ticket: None,
            signer: None,
            session_id: None,
            session_token: None,
        }
    }

    /// Use an existing session (for reconnection).
    pub fn with_session(mut self, session: Session) -> Self {
        self.session = session;
        self
    }

    pub fn addr(mut self, addr: EndpointAddr) -> Self {
        self.addr = Some(addr);
        self
    }

    pub fn ticket(mut self, ticket: ZedraPairingTicket) -> Self {
        self.addr = Some(EndpointAddr::from(ticket.endpoint_id));
        self.ticket = Some(ticket);
        self
    }

    pub fn signer(mut self, signer: Arc<dyn ClientSigner>) -> Self {
        self.signer = Some(signer);
        self
    }

    pub fn session_id(mut self, id: String) -> Self {
        self.session_id = Some(id);
        self
    }

    pub fn session_token(mut self, token: [u8; 32]) -> Self {
        self.session_token = Some(token);
        self
    }

    /// Start connection and return the session.
    ///
    /// `on_connected` is called after successful auth/sync, before UI notification.
    pub fn connect<F>(self, on_connected: F) -> Result<Session, ConnectError>
    where
        F: Fn(&SessionHandle) + Send + Sync + 'static,
    {
        let addr = self
            .addr
            .ok_or_else(|| ConnectError::Other("addr required".into()))?;
        let signer = self
            .signer
            .ok_or_else(|| ConnectError::Other("signer required".into()))?;

        if let Some(token) = self.session_token {
            self.session.handle.set_session_token(Some(token));
        }

        self.session
            .connect(addr, self.ticket, signer, self.session_id, on_connected);

        Ok(self.session)
    }
}

impl Default for SessionBuilder {
    fn default() -> Self {
        Self::new()
    }
}
