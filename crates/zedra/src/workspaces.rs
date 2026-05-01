use std::sync::Arc;

use gpui::*;
use tracing::*;
use zedra_rpc::ZedraPairingTicket;
use zedra_session::signer::ClientSigner;

use crate::pending::PendingSlot;
use crate::platform_bridge;
use crate::workspace::{Workspace, WorkspaceEvent};
use crate::workspace_state::WorkspaceState;

static PENDING_TICKET: PendingSlot<ZedraPairingTicket> = PendingSlot::new();

#[derive(Clone, Debug)]
pub enum WorkspacesEvent {
    Connected { index: usize },
    Disconnected { index: usize },
    StatesChanged,
    GoHome,
    OpenQuickAction,
}

impl EventEmitter<WorkspacesEvent> for Workspaces {}

pub struct Workspaces {
    /// Workspace entries, one per state.
    /// The entry is lazily loaded from the state when first opened,
    /// and removed on disconnect.
    entries: Vec<Entity<Workspace>>,
    states: Vec<Entity<WorkspaceState>>,
    active_index: Option<usize>,
    signer: Option<Arc<dyn ClientSigner>>,
    _subscriptions: Vec<Subscription>,
}

impl Workspaces {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let signer = load_client_signer();

        let states = WorkspaceState::load()
            .map_err(|e| error!("Failed to load saved workspace states: {e}"))
            .map(|states| {
                info!("Loaded {} saved workspace(s)", states.len());
                states
                    .into_iter()
                    .map(|s| cx.new(|_cx| s))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let mut this = Self {
            entries: Vec::new(),
            states,
            signer,
            active_index: None,
            _subscriptions: Vec::new(),
        };
        this.emit_states_changed(cx);
        this
    }

    pub fn active(&self) -> Option<&Entity<Workspace>> {
        self.active_index.and_then(|i| self.entries.get(i))
    }

    pub fn active_view(&self) -> Option<AnyView> {
        self.active().map(|e| AnyView::from(e.clone()))
    }

    pub fn workspace_by_index(&self, index: usize) -> Option<&Entity<Workspace>> {
        self.entries.get(index)
    }

    pub fn get(&self, index: usize) -> Option<&Entity<Workspace>> {
        self.entries.get(index)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn states(&self) -> &[Entity<WorkspaceState>] {
        &self.states
    }

    pub fn entry_by_endpoint_addr(
        &self,
        endpoint_addr: &str,
        cx: &App,
    ) -> Option<Entity<Workspace>> {
        self.entries
            .iter()
            .find(|ws| ws.read(cx).endpoint_addr(cx) == endpoint_addr)
            .cloned()
    }

    pub fn entry_index_by_endpoint_addr(&self, endpoint_addr: &str, cx: &App) -> Option<usize> {
        self.entries
            .iter()
            .position(|ws| ws.read(cx).endpoint_addr(cx) == endpoint_addr)
    }

    pub fn switch_to(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.entries.len() {
            self.active_index = Some(index);
            cx.notify();
        } else {
            warn!("Index {index} out of range. Cannot switch to workspace.");
        }
    }

    /// Connect via QR pairing ticket (new device pairing).
    pub fn connect_ticket(
        &mut self,
        ticket: ZedraPairingTicket,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let addr = iroh::EndpointAddr::from(ticket.endpoint_id);
        self.connect_and_intialize_workspace(addr, Some(ticket), None, None, window, cx);
    }

    /// Queue a ticket for deferred connection (when window not available).
    pub fn connect_ticket_deferred(&mut self, ticket: ZedraPairingTicket, cx: &mut Context<Self>) {
        PENDING_TICKET.set(ticket);
        cx.notify();
    }

    pub fn has_pending_ticket() -> bool {
        PENDING_TICKET.has_pending()
    }

    /// Process any pending ticket. Call this when window is available.
    pub fn process_pending_ticket(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ticket) = PENDING_TICKET.take() {
            self.connect_ticket(ticket, window, cx);
        }
    }

    /// Reconnect to a saved workspace by state index.
    pub fn connect_saved(
        &mut self,
        state_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let state = match self.states.get(state_index) {
            Some(s) => s.clone(),
            None => {
                error!("Index {state_index} out of range. Cannot reconnect to saved workspace.");
                return;
            }
        };

        let session_id = state.read(cx).session_id.clone();
        let endpoint_addr = state.read(cx).endpoint_addr.clone();
        match zedra_rpc::pairing::decode_endpoint_addr(&endpoint_addr) {
            Ok(addr) => {
                info!("Reconnecting to workspace: {}", addr.id.fmt_short());
                self.connect_and_intialize_workspace(
                    addr,
                    None,
                    Some(session_id),
                    Some(state.clone()),
                    window,
                    cx,
                );
            }
            Err(e) => {
                error!("Failed to decode endpoint addr: {e}. Removing workspace state.");
                WorkspaceState::remove_by_endpoint_add(&endpoint_addr)
                    .map_err(|e| error!("Failed to remove workspace state: {e}"))
                    .ok();
            }
        }
    }

    fn connect_and_intialize_workspace(
        &mut self,
        addr: iroh::EndpointAddr,
        ticket: Option<ZedraPairingTicket>,
        session_id: Option<String>,
        saved: Option<Entity<WorkspaceState>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(signer) = self.signer.clone() else {
            error!("No client signer available. Cannot connect to workspace.");
            return;
        };

        let encoded_addr = zedra_rpc::pairing::encode_endpoint_addr(&addr).unwrap_or_default();

        // Workspace state (from saved or fresh)
        let workspace_state = saved.unwrap_or_else(|| {
            let workspace_state = cx.new(|_cx| {
                let mut ws = WorkspaceState::default();
                ws.endpoint_addr = encoded_addr.clone();
                ws
            });
            self.states.push(workspace_state.clone());
            workspace_state
        });

        // Create workspace entity
        let workspace = cx.new(|cx| Workspace::new(workspace_state.clone(), window, cx));
        self._subscriptions
            .push(self.subscribe_workspace_event(&workspace, cx));

        // Start connection
        workspace.update(cx, |ws, cx| {
            ws.connect(addr, ticket.clone(), signer, session_id.clone(), window, cx);
        });

        self.entries.push(workspace);
        let ws_idx = self.entries.len() - 1;
        self.active_index = Some(ws_idx);

        // TODO: this is not connected yet, it's just a signal to navigate to the workspace.
        cx.emit(WorkspacesEvent::Connected { index: ws_idx });
        cx.notify();
    }

    fn subscribe_workspace_event(
        &self,
        workspace: &Entity<Workspace>,
        cx: &mut Context<Self>,
    ) -> Subscription {
        let ws_entity = workspace.clone();
        cx.subscribe(
            workspace,
            move |this, _emitter, event: &WorkspaceEvent, cx| match event {
                WorkspaceEvent::GoHome => {
                    cx.emit(WorkspacesEvent::GoHome);
                }
                WorkspaceEvent::OpenQuickAction => {
                    cx.emit(WorkspacesEvent::OpenQuickAction);
                }
                WorkspaceEvent::Disconnected => {
                    let index = this.entries.iter().position(|e| *e == ws_entity);
                    if let Some(index) = index {
                        // Just remove the workspace entry, keep the state.
                        this.remove_entry(index, cx);
                    }
                }
            },
        )
    }

    /// Disconnect the workspace at the given index.
    pub fn disconnect(&mut self, entry_index: usize, cx: &mut Context<Self>) {
        if let Some(entry) = self.entries.get(entry_index) {
            entry.update(cx, |ws, cx| ws.disconnect(cx));
        }
    }

    pub fn disconnect_by_endpoint_addr(&mut self, endpoint_addr: &str, cx: &mut Context<Self>) {
        if let Some(index) = self
            .entries
            .iter()
            .position(|s| s.read(cx).endpoint_addr(cx) == endpoint_addr)
        {
            self.disconnect(index, cx);
        }
    }

    pub fn remove_by_endpoint_addr(&mut self, endpoint_addr: &str, cx: &mut Context<Self>) {
        if let Some(index) = self
            .entries
            .iter()
            .position(|s| s.read(cx).endpoint_addr(cx) == endpoint_addr)
        {
            if let Some(entry) = self.entries.get(index) {
                entry.update(cx, |ws, _cx| ws.prepare_for_saved_removal());
            }
            self.remove_entry(index, cx);
        }

        self.remove_saved(endpoint_addr, cx);
    }

    pub fn remove_saved(&mut self, endpoint_addr: &str, cx: &mut Context<Self>) {
        WorkspaceState::remove_by_endpoint_add(endpoint_addr)
            .map_err(|e| error!("Failed to remove workspace state: {e}"))
            .ok();
        let state_index = self
            .states
            .iter()
            .position(|s| s.read(cx).endpoint_addr == endpoint_addr);
        if let Some(index) = state_index {
            self.states.remove(index);
        }
        cx.notify();
    }

    fn remove_entry(&mut self, index: usize, cx: &mut Context<Self>) {
        self.entries.remove(index);
        self.active_index = if self.entries.is_empty() {
            None
        } else {
            Some(0)
        };

        info!("Workspace disconnected; {} remaining", self.entries.len());
        cx.emit(WorkspacesEvent::Disconnected { index });
    }

    fn emit_states_changed(&mut self, cx: &mut Context<Self>) {
        cx.emit(WorkspacesEvent::StatesChanged);
    }
}

/// Load the persistent client Ed25519 signing key.
fn load_client_signer() -> Option<Arc<dyn ClientSigner>> {
    let data_dir = platform_bridge::bridge().data_directory()?;
    let key_path = std::path::PathBuf::from(data_dir)
        .join("zedra")
        .join("client.key");
    match zedra_session::signer::FileClientSigner::load_or_generate(&key_path) {
        Ok(signer) => Some(Arc::new(signer)),
        Err(e) => {
            error!("Failed to load client signing key: {}", e);
            None
        }
    }
}
