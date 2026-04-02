use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::StreamExt;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui::*;
use zedra_rpc::ZedraPairingTicket;
use zedra_session::{Session, SessionHandle, signer::ClientSigner};

use crate::pending::PendingSlot;
use crate::platform_bridge;
use crate::workspace_state::{SharedWorkspaceStates, WorkspaceState};
use crate::workspace_view::{WorkspaceEvent, WorkspaceView, compute_terminal_dimensions};

static PENDING_TICKET: PendingSlot<ZedraPairingTicket> = PendingSlot::new();

pub struct Workspace {
    pub view: Entity<WorkspaceView>,
    pub session: Session,
    needs_sync: Arc<AtomicBool>,
    _state_listener: Task<()>,
}

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
    entries: Vec<Workspace>,
    states: Vec<WorkspaceState>,
    active: Option<usize>,
    signer: Option<Arc<dyn ClientSigner>>,
    _subscriptions: Vec<Subscription>,
    sync_notify_tx: UnboundedSender<()>,
    _sync_listener: Task<()>,
}

impl Workspaces {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let signer = load_client_signer();
        let states = WorkspaceState::load();
        tracing::info!("Workspaces: loaded {} saved workspace(s)", states.len());

        let (sync_notify_tx, mut sync_notify_rx) = unbounded::<()>();
        let sync_listener = cx.spawn(async move |weak, cx| {
            while sync_notify_rx.next().await.is_some() {
                if weak.update(cx, |ws, cx| ws.sync_if_needed(cx)).is_err() {
                    break;
                }
            }
        });

        let mut this = Self {
            entries: Vec::new(),
            states,
            active: None,
            signer,
            _subscriptions: Vec::new(),
            sync_notify_tx,
            _sync_listener: sync_listener,
        };
        this.emit_states_changed(cx);
        this
    }

    // ─── Accessors ───────────────────────────────────────────────────────────

    pub fn active(&self) -> Option<&Workspace> {
        self.active.and_then(|i| self.entries.get(i))
    }

    pub fn active_index(&self) -> Option<usize> {
        self.active
    }

    pub fn active_view(&self) -> Option<Entity<WorkspaceView>> {
        self.active().map(|w| w.view.clone())
    }

    pub fn get(&self, index: usize) -> Option<&Workspace> {
        self.entries.get(index)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn states(&self) -> &[WorkspaceState] {
        &self.states
    }

    pub fn shared_states(&self) -> SharedWorkspaceStates {
        Arc::new(self.states.clone())
    }

    pub fn handles(&self) -> Vec<SessionHandle> {
        self.entries
            .iter()
            .map(|e| e.session.handle().clone())
            .collect()
    }

    // ─── Navigation ──────────────────────────────────────────────────────────

    pub fn switch_to(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.entries.len() {
            self.active = Some(index);
            self.entries[index].view.update(cx, |ws, cx| {
                ws.on_activate(cx);
            });
            cx.notify();
        }
    }

    // ─── Connection ──────────────────────────────────────────────────────────

    /// Connect via QR pairing ticket (new device pairing).
    pub fn connect_ticket(
        &mut self,
        ticket: ZedraPairingTicket,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let addr = iroh::EndpointAddr::from(ticket.endpoint_id);
        self.connect_internal(addr, Some(ticket), None, None, window, cx);
    }

    /// Queue a ticket for deferred connection (when window not available).
    /// Call `process_pending_ticket()` when window becomes available.
    pub fn connect_ticket_deferred(&mut self, ticket: ZedraPairingTicket, cx: &mut Context<Self>) {
        PENDING_TICKET.set(ticket);
        cx.notify();
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
                tracing::error!("connect_saved: index {} out of range", state_index);
                return;
            }
        };

        match zedra_rpc::pairing::decode_endpoint_addr(state.endpoint_addr()) {
            Ok(addr) => {
                tracing::info!("Reconnecting to workspace: {}", addr.id.fmt_short());
                self.connect_internal(
                    addr,
                    None,
                    Some(state.session_id().to_string()),
                    Some(state),
                    window,
                    cx,
                );
            }
            Err(e) => {
                tracing::error!("Failed to decode endpoint addr: {}", e);
                WorkspaceState::remove(state.endpoint_addr());
                self.reload_states(cx);
            }
        }
    }

    /// Internal connection flow.
    fn connect_internal(
        &mut self,
        addr: iroh::EndpointAddr,
        ticket: Option<ZedraPairingTicket>,
        session_id: Option<String>,
        saved: Option<WorkspaceState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(signer) = self.signer.clone() else {
            tracing::error!("connect: no client signer available");
            return;
        };

        let encoded_addr = zedra_rpc::pairing::encode_endpoint_addr(&addr).unwrap_or_default();

        let session = Session::new();
        let handle = session.handle().clone();
        let state = session.state().clone();
        let needs_sync = Arc::new(AtomicBool::new(false));

        let mut state_notify_rx = state.subscribe();
        let sync_tx = self.sync_notify_tx.clone();
        let flag = needs_sync.clone();
        let state_listener = cx.spawn(async move |_, _| {
            while state_notify_rx.next().await.is_some() {
                flag.store(true, Ordering::Release);
                let _ = sync_tx.unbounded_send(());
            }
        });

        if let Some(ref t) = ticket {
            handle.set_pending_ticket(t.clone());
        }
        handle.set_session_id(session_id.clone());

        let view = cx.new(|cx| WorkspaceView::new(handle.clone(), state.clone(), window, cx));
        let pending_term_id = view.read(cx).pending_terminal_id.clone();
        let pending_existing = view.read(cx).pending_existing_terminals.clone();
        let (cols, rows, _, _) = compute_terminal_dimensions(window);

        self._subscriptions
            .push(self.subscribe_workspace(&view, cx));
        self.entries.push(Workspace {
            view: view.clone(),
            session: session.clone(),
            needs_sync,
            _state_listener: state_listener,
        });
        let ws_idx = self.entries.len() - 1;
        self.active = Some(ws_idx);

        self.link_or_create_state(&encoded_addr, ws_idx, saved, cx);
        if let Some(ws_state) = self
            .states
            .iter()
            .find(|s| s.workspace_index() == Some(ws_idx))
        {
            let s = ws_state.clone();
            view.update(cx, |v, cx| v.set_workspace_state(s, cx));
        }

        handle.set_endpoint_addr(addr.clone());
        self.persist_active_workspaces();

        let session_id_for_connect =
            session_id.or_else(|| ticket.as_ref().map(|t| t.session_id.clone()));
        session.connect(
            addr,
            ticket,
            signer,
            session_id_for_connect,
            move |handle| {
                let server_ids = handle.terminal_ids();
                if !server_ids.is_empty() {
                    tracing::info!("Session resumed: {} terminal(s)", server_ids.len());
                    pending_existing.set(server_ids);
                } else {
                    let h = handle.clone();
                    let slot = pending_term_id.clone();
                    let (cols, rows) = (cols as u16, rows as u16);
                    zedra_session::session_runtime().spawn(async move {
                        match h.terminal_create(cols, rows).await {
                            Ok(tid) => {
                                tracing::info!("Terminal created: {tid}");
                                slot.set(tid);
                            }
                            Err(e) => tracing::error!("terminal_create failed: {e}"),
                        }
                    });
                }
            },
        );

        cx.emit(WorkspacesEvent::Connected { index: ws_idx });
        cx.notify();
    }

    fn subscribe_workspace(
        &self,
        view: &Entity<WorkspaceView>,
        cx: &mut Context<Self>,
    ) -> Subscription {
        let view_entity = view.clone();
        cx.subscribe(view, move |this, _emitter, event: &WorkspaceEvent, cx| {
            match event {
                WorkspaceEvent::GoHome => {
                    cx.emit(WorkspacesEvent::GoHome);
                }
                WorkspaceEvent::OpenQuickAction => {
                    cx.emit(WorkspacesEvent::OpenQuickAction);
                }
                WorkspaceEvent::Disconnected => {
                    let index = this.entries.iter().position(|e| e.view == view_entity);
                    if let Some(idx) = index {
                        this.entries.remove(idx);
                        tracing::info!("Workspace disconnected; {} remaining", this.entries.len());

                        // Update active index
                        this.active = if this.entries.is_empty() {
                            None
                        } else {
                            Some(0)
                        };

                        this.reload_states(cx);
                        cx.emit(WorkspacesEvent::Disconnected { index: idx });
                    }
                }
            }
        })
    }

    pub fn disconnect(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(entry) = self.entries.get(index) {
            entry.view.update(cx, |ws, cx| ws.disconnect(cx));
        }
    }

    // ─── State Management ────────────────────────────────────────────────────

    fn link_or_create_state(
        &mut self,
        encoded_addr: &str,
        ws_idx: usize,
        saved: Option<WorkspaceState>,
        cx: &mut Context<Self>,
    ) {
        if let Some(pos) = self
            .states
            .iter()
            .position(|s| s.endpoint_addr() == encoded_addr)
        {
            self.states[pos] = WorkspaceState::update_inner(self.states[pos].clone(), |s| {
                s.workspace_index = Some(ws_idx);
            });
        } else {
            let new_state = if let Some(s) = saved {
                WorkspaceState::update_inner(s, |s| {
                    s.workspace_index = Some(ws_idx);
                })
            } else {
                WorkspaceState::update_inner(WorkspaceState::default(), |s| {
                    s.endpoint_addr = encoded_addr.to_string();
                    s.workspace_index = Some(ws_idx);
                })
            };
            self.states.push(new_state);
        }
        self.emit_states_changed(cx);
    }

    pub fn reload_states(&mut self, cx: &mut Context<Self>) {
        let saved = WorkspaceState::load();
        self.states = saved;

        // Re-link active workspaces
        for (ws_idx, entry) in self.entries.iter().enumerate() {
            let encoded = entry
                .session
                .handle()
                .endpoint_addr()
                .and_then(|a| zedra_rpc::pairing::encode_endpoint_addr(&a).ok());
            if let Some(addr) = &encoded {
                if let Some(pos) = self
                    .states
                    .iter()
                    .position(|s| s.endpoint_addr() == addr.as_str())
                {
                    self.states[pos] =
                        WorkspaceState::update_inner(self.states[pos].clone(), |s| {
                            s.workspace_index = Some(ws_idx);
                        });
                }
            }
        }
        self.emit_states_changed(cx);
    }

    /// Sync workspace states that have been flagged by session state changes.
    /// Call this in render() — it only syncs workspaces whose sessions signaled changes.
    pub fn sync_if_needed(&mut self, cx: &mut Context<Self>) {
        let mut any_changed = false;

        for (ws_idx, entry) in self.entries.iter().enumerate() {
            if !entry.needs_sync.swap(false, Ordering::AcqRel) {
                continue;
            }

            // Find and update the corresponding WorkspaceState
            if let Some(state) = self
                .states
                .iter_mut()
                .find(|s| s.workspace_index() == Some(ws_idx))
            {
                let sess = entry.session.state().get();
                let (terminal_ids, active_terminal_id) = entry.view.read(cx).terminal_state();
                let session_id = entry.session.handle().session_id().unwrap_or_default();

                *state = WorkspaceState::update_inner(state.clone(), |s| {
                    s.connect_phase = Some(sess.phase);
                    s.terminal_count = terminal_ids.len();
                    s.terminal_ids = terminal_ids;
                    s.active_terminal_id = active_terminal_id;

                    let snap = &sess.snapshot;
                    if !snap.hostname.is_empty() {
                        s.hostname = snap.hostname.clone();
                    }
                    if !snap.workdir.is_empty() {
                        s.workdir = snap.workdir.clone();
                    }
                    if !snap.project_name.is_empty() {
                        s.project_name = snap.project_name.clone();
                    }
                    if !snap.strip_path.is_empty() {
                        s.strip_path = snap.strip_path.clone();
                    }
                    if !snap.homedir.is_empty() {
                        s.homedir = snap.homedir.clone();
                    }
                    if !session_id.is_empty() {
                        s.session_id = session_id;
                    }
                });

                // Push to view
                let s = state.clone();
                entry.view.update(cx, |v, cx| v.set_workspace_state(s, cx));
                any_changed = true;
            }
        }

        if any_changed {
            self.emit_states_changed(cx);
        }
    }

    pub fn remove_saved(&mut self, endpoint_addr: &str, cx: &mut Context<Self>) {
        WorkspaceState::remove(endpoint_addr);
        self.reload_states(cx);
    }

    fn persist_active_workspaces(&self) {
        for entry in &self.entries {
            if let Some(ws) =
                WorkspaceState::from_session(entry.session.handle(), entry.session.state())
            {
                WorkspaceState::upsert(ws);
            }
        }
    }

    /// Persist workspace states. Call periodically.
    pub fn persist(&self) {
        self.persist_active_workspaces();
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
            tracing::error!("Failed to load client signing key: {}", e);
            None
        }
    }
}
