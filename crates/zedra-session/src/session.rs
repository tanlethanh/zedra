use std::sync::{Arc, Mutex};

use iroh::EndpointAddr;
use tokio::sync::{Notify, mpsc};
use tokio_util::sync::CancellationToken;
use tracing::info;
use zedra_rpc::ZedraPairingTicket;
use zedra_rpc::proto::{HostEvent, ZedraProto};

use crate::RemoteTerminal;
use crate::{
    ConnectEvent, Connector, SessionHandle, SessionState, session_runtime, signer::ClientSigner,
};

#[derive(Clone)]
pub struct Session {
    handle: SessionHandle,
    state: SessionState,
    event_tx: mpsc::Sender<ConnectEvent>,
    /// The event receiver. The caller is responsible for processing events.
    event_rx: Arc<Mutex<Option<mpsc::Receiver<ConnectEvent>>>>,
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

        Self {
            handle,
            state,
            event_tx,
            event_rx: Arc::new(Mutex::new(Some(event_rx))),
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
                let result = match reconnect_reason.take() {
                    Some(reason) => {
                        let max_attempts = 10;
                        info!("reconnect to {addr:?}, session: {session_id:?} reason {reason:?} max_attempts {max_attempts}",);
                        connector
                            .reconnect_loop(
                                addr.clone(),
                                None,
                                signer.clone(),
                                session_id.clone(),
                                session_token,
                                reason,
                                max_attempts,
                                Some(existing_terminals)
                            )
                            .await
                    }
                    None => {
                        info!("start connect to {addr:?}, session: {session_id:?}");
                        connector
                            .connect(
                                addr.clone(),
                                ticket.as_ref(),
                                signer.clone(),
                                session_id.clone(),
                                session_token,
                                Some(existing_terminals)
                            )
                            .await
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
        event: HostEvent,
    ) {
        match event {
            HostEvent::TerminalCreated { id, launch_cmd } => {
                info!(
                    "HostEvent: terminal created id={} launch_cmd={:?}",
                    id, launch_cmd,
                );
                let terminal = handle.terminal(&id).unwrap_or_else(|| {
                    let terminal = RemoteTerminal::new(id);
                    handle.add_terminal(terminal.clone());
                    terminal
                });
                if let Err(e) = terminal.attach_remote(client).await {
                    tracing::warn!(
                        "Failed to attach host-created terminal {}: {e}",
                        terminal.id()
                    );
                }
            }
            HostEvent::GitChanged => {
                info!("HostEvent: git changed");
                handle.set_git_needs_refresh();
            }
            HostEvent::FsChanged { path } => {
                info!("HostEvent: fs changed path={path}");
                handle.add_fs_changed(path);
            }
        }
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}
