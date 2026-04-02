use std::sync::Arc;

use iroh::EndpointAddr;
use tokio::sync::mpsc;
use zedra_rpc::ZedraPairingTicket;

use crate::{
    ConnectError, ConnectEvent, Connector, SessionHandle, SessionState, session_runtime,
    signer::ClientSigner,
};

#[derive(Clone)]
pub struct Session {
    handle: SessionHandle,
    state: SessionState,
    event_tx: mpsc::Sender<ConnectEvent>,
}

impl Session {
    pub fn new() -> Self {
        let handle = SessionHandle::new();
        let (state, event_tx) = SessionState::with_channel();
        Self {
            handle,
            state,
            event_tx,
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
        F: FnOnce(&SessionHandle) + Send + 'static,
    {
        let handle = self.handle.clone();
        let event_tx = self.event_tx.clone();

        // Store credentials on handle
        handle.set_signer(signer.clone());
        handle.set_endpoint_addr(addr.clone());
        if let Some(ref t) = ticket {
            handle.set_pending_ticket(t.clone());
        }
        handle.set_session_id(session_id.clone());

        let session_token = handle.session_token();

        session_runtime().spawn(async move {
            let mut connector = Connector::new(event_tx);

            match connector
                .connect(addr, ticket.as_ref(), signer, session_id, session_token)
                .await
            {
                Ok((client, sync)) => {
                    handle.set_rpc_client(client);
                    handle.set_session_id(Some(sync.session_id.clone()));
                    handle.set_session_token(Some(sync.session_token));
                    handle.sync_terminals(&sync.terminals);

                    on_connected(&handle);
                }
                Err(e) => {
                    tracing::error!("connect failed: {}", e);
                }
            }
        });
    }

    /// Disconnect and clear session state.
    pub fn disconnect(&self) {
        self.handle.clear_session();
    }
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
        F: FnOnce(&SessionHandle) + Send + 'static,
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
