// ZedraApp — multi-workspace coordinator.
// Manages HomeView, WorkspaceView instances, and QuickActionPanel.

use std::sync::Arc;
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};

use gpui::*;
use zedra_telemetry::*;

use crate::deeplink::{self, DeeplinkAction};
use crate::fonts;
use crate::home_view::{HomeEvent, HomeView};
use crate::mgpui::{DrawerHost, DrawerSide};
use crate::platform_bridge::{self, AlertButton};
use crate::quick_action_panel::{QuickActionEvent, QuickActionPanel};
use crate::theme;
use crate::workspace_state::{SharedWorkspaceStates, WorkspaceState};
use crate::workspace_view::{WorkspaceEvent, WorkspaceView, compute_terminal_dimensions};
use zedra_session::SessionHandle;

// ---------------------------------------------------------------------------
// AppScreen
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
enum AppScreen {
    Home,
    Workspace,
}

// ---------------------------------------------------------------------------
// WorkspaceEntry
// ---------------------------------------------------------------------------

struct WorkspaceEntry {
    view: Entity<WorkspaceView>,
    handle: SessionHandle,
}

impl PartialEq for WorkspaceEntry {
    fn eq(&self, other: &Self) -> bool {
        self.view == other.view
    }
}

// ---------------------------------------------------------------------------
// ZedraApp
// ---------------------------------------------------------------------------

pub struct ZedraApp {
    screen: AppScreen,
    home_view: Entity<HomeView>,
    workspaces: Vec<WorkspaceEntry>,
    workspace_states: Vec<WorkspaceState>,
    active_workspace: Option<usize>,
    quick_action: Entity<QuickActionPanel>,
    qa_drawer: Entity<DrawerHost>,
    render_count: u64,
    last_states_revision: Option<u64>,
    _subscriptions: Vec<Subscription>,
}

/// Load (or generate) the persistent client Ed25519 signing key.
///
/// The key is stored in `<data_dir>/zedra/client.key` with 0o600 permissions.
/// Returns `None` if the data directory is unavailable (no platform bridge).
fn load_client_signer() -> Option<std::sync::Arc<dyn zedra_session::signer::ClientSigner>> {
    let data_dir = platform_bridge::bridge().data_directory()?;
    let key_path = std::path::PathBuf::from(data_dir)
        .join("zedra")
        .join("client.key");
    match zedra_session::signer::FileClientSigner::load_or_generate(&key_path) {
        Ok(signer) => Some(std::sync::Arc::new(signer)),
        Err(e) => {
            tracing::error!("Failed to load client signing key: {}", e);
            None
        }
    }
}

impl ZedraApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Load JetBrains Mono font for all UI text
        fonts::load_fonts(window);

        let mut subscriptions = Vec::new();

        let empty_states: SharedWorkspaceStates = Arc::new(Vec::new());

        // --- Home view ---
        let home_view = cx.new(|cx| HomeView::new(cx, Arc::clone(&empty_states)));
        let sub = cx.subscribe_in(
            &home_view,
            window,
            |this: &mut Self, _emitter, event: &HomeEvent, window, cx| match event {
                HomeEvent::ScanQrTapped => {
                    tracing::info!("Home: Scan QR tapped");
                    zedra_telemetry::send(Event::QrScanInitiated);
                    platform_bridge::bridge().launch_qr_scanner();
                }
                HomeEvent::WorkspaceTapped(item_idx) => {
                    if let Some(state) = this.workspace_states.get(*item_idx) {
                        if let Some(ws_idx) = state.workspace_index() {
                            this.switch_to_workspace(ws_idx, window, cx);
                        } else {
                            this.reconnect_workspace(*item_idx, window, cx);
                        }
                    }
                }
                HomeEvent::WorkspaceRemoved(item_idx) => {
                    if let Some(item) = this.workspace_states.get(*item_idx) {
                        let endpoint_addr = item.endpoint_addr().to_string();
                        let ws_index_opt = item.workspace_index();
                        let display = item.display_name().to_string();
                        platform_bridge::show_alert(
                            "",
                            &format!("Remove {} workspace?", display),
                            vec![
                                AlertButton::destructive("Delete"),
                                AlertButton::cancel("Cancel"),
                            ],
                            move |button_index| {
                                if button_index == 0 {
                                    PENDING_WORKSPACE_DELETE
                                        .set((endpoint_addr.clone(), ws_index_opt));
                                }
                            },
                        );
                    }
                }
            },
        );
        subscriptions.push(sub);

        // --- Quick action panel ---
        let quick_action = cx.new(|cx| QuickActionPanel::new(cx, Arc::clone(&empty_states)));
        let sub = cx.subscribe_in(
            &quick_action,
            window,
            |this: &mut Self, _emitter, event: &QuickActionEvent, window, cx| match event {
                QuickActionEvent::Close => {
                    this.qa_drawer.update(cx, |h, cx| h.close(cx));
                }
                QuickActionEvent::GoHome => {
                    this.qa_drawer.update(cx, |h, cx| h.close(cx));
                    this.screen = AppScreen::Home;
                }
                QuickActionEvent::ScanQr => {
                    this.qa_drawer.update(cx, |h, cx| h.close(cx));
                    platform_bridge::bridge().launch_qr_scanner();
                }
                QuickActionEvent::SwitchToWorkspace(index) => {
                    this.qa_drawer.update(cx, |h, cx| h.close(cx));
                    this.switch_to_workspace(*index, window, cx);
                }
                QuickActionEvent::SwitchToTerminal(ws_index, tid) => {
                    this.qa_drawer.update(cx, |h, cx| h.close(cx));
                    let ws_index = *ws_index;
                    let tid = tid.clone();
                    this.switch_to_workspace(ws_index, window, cx);
                    if let Some(entry) = this.workspaces.get(ws_index) {
                        entry.view.update(cx, |ws, cx| {
                            ws.switch_to_terminal(&tid, cx);
                        });
                    }
                }
                QuickActionEvent::TerminalDeleteRequested(ws_index, tid) => {
                    let ws_index = *ws_index;
                    let tid = tid.clone();
                    if let Some(entry) = this.workspaces.get(ws_index) {
                        entry.view.update(cx, |ws, cx| {
                            ws.request_terminal_delete(tid, cx);
                        });
                    }
                }
            },
        );
        subscriptions.push(sub);

        // --- Window activation (foreground/background) ---
        let sub = cx.observe_window_activation(window, |this: &mut Self, window, _cx| {
            if window.is_window_active() {
                tracing::info!(
                    "ZedraApp: window activated, notifying {} workspace(s)",
                    this.workspaces.len()
                );
                for entry in &this.workspaces {
                    entry.handle.notify_foreground_resume();
                }
            }
        });
        subscriptions.push(sub);

        let qa_drawer = cx.new(|cx| DrawerHost::new(home_view.clone().into(), cx));
        qa_drawer.update(cx, |h, _| {
            h.set_side(DrawerSide::Right);
            h.set_width(px(theme::QA_DRAWER_WIDTH));
            h.set_drawer(quick_action.clone().into());
        });

        let mut app = Self {
            screen: AppScreen::Home,
            home_view,
            workspaces: Vec::new(),
            workspace_states: Vec::new(),
            active_workspace: None,
            quick_action,
            qa_drawer,
            render_count: 0,
            last_states_revision: None,
            _subscriptions: subscriptions,
        };

        // Load saved workspaces from disk
        app.reload_workspace_states(cx);

        zedra_telemetry::send(Event::AppOpen {
            saved_workspaces: app.workspace_states.len(),
            app_version: env!("CARGO_PKG_VERSION"),
            platform: std::env::consts::OS,
            arch: std::env::consts::ARCH,
        });

        app
    }

    fn switch_to_workspace(&mut self, index: usize, _window: &mut Window, cx: &mut Context<Self>) {
        if index < self.workspaces.len() {
            zedra_telemetry::send(Event::ScreenView {
                screen: "workspace",
            });
            self.active_workspace = Some(index);
            self.screen = AppScreen::Workspace;
            // Notify the workspace's content and drawer so they re-render with the
            // updated global session (badge, terminal list, session info).
            self.workspaces[index].view.update(cx, |ws, cx| {
                ws.on_activate(cx);
            });
            cx.notify();
        }
    }

    /// Reload workspace_states from disk, preserving workspace_index for active connections.
    fn reload_workspace_states(&mut self, cx: &mut Context<Self>) {
        let saved = WorkspaceState::load();
        tracing::info!("Loaded {} workspace(s) from disk", saved.len());
        // Re-link active workspaces by endpoint_addr
        self.workspace_states = saved;
        for (ws_idx, entry) in self.workspaces.iter().enumerate() {
            let encoded = entry
                .handle
                .endpoint_addr()
                .and_then(|a| zedra_rpc::pairing::encode_endpoint_addr(&a).ok());
            if let Some(addr) = &encoded {
                if let Some(pos) = self
                    .workspace_states
                    .iter()
                    .position(|s| s.endpoint_addr() == addr.as_str())
                {
                    self.workspace_states[pos] =
                        WorkspaceState::update_inner(self.workspace_states[pos].clone(), |s| {
                            s.workspace_index = Some(ws_idx);
                        });
                }
            }
        }
        self.push_states_to_views(cx);
    }

    /// Push workspace_states to home view and quick action panel.
    fn push_states_to_views(&mut self, cx: &mut Context<Self>) {
        let revision = Self::states_revision(&self.workspace_states);
        if self.last_states_revision == Some(revision) {
            return;
        }
        self.last_states_revision = Some(revision);
        let shared: SharedWorkspaceStates = Arc::new(self.workspace_states.clone());
        self.home_view.update(cx, |hv, cx| {
            hv.set_states(Arc::clone(&shared));
            cx.notify();
        });
        let handles: Vec<_> = self.workspaces.iter().map(|e| e.handle.clone()).collect();
        self.quick_action.update(cx, |qa, cx| {
            qa.set_states(Arc::clone(&shared));
            qa.set_handles(handles);
            cx.notify();
        });
    }

    fn states_revision(items: &[WorkspaceState]) -> u64 {
        let mut hasher = DefaultHasher::new();
        items.len().hash(&mut hasher);
        for state in items {
            state.workspace_index().hash(&mut hasher);
            state.endpoint_addr().hash(&mut hasher);
            state.session_id().hash(&mut hasher);
            state.project_name().hash(&mut hasher);
            state.strip_path().hash(&mut hasher);
            state.hostname().hash(&mut hasher);
            state.terminal_count().hash(&mut hasher);
            state.terminal_ids().hash(&mut hasher);
            state.active_terminal_id().hash(&mut hasher);
            state
                .connect_phase()
                .map(|phase| format!("{phase:?}"))
                .unwrap_or_default()
                .hash(&mut hasher);
        }
        hasher.finish()
    }

    fn persist_current_workspaces(&self) {
        let persisted: Vec<_> = self
            .workspaces
            .iter()
            .filter_map(|entry| WorkspaceState::from_handle(&entry.handle))
            .collect();
        if !persisted.is_empty() {
            // Upsert each workspace (preserves saved workspaces we're not connected to)
            for ws in persisted {
                WorkspaceState::upsert(ws);
            }
        }
    }

    fn reconnect_workspace(
        &mut self,
        item_idx: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ws = match self.workspace_states.get(item_idx) {
            Some(ws) => ws.clone(),
            None => {
                tracing::error!("Workspace state index {} out of range", item_idx);
                return;
            }
        };

        match zedra_rpc::pairing::decode_endpoint_addr(ws.endpoint_addr()) {
            Ok(addr) => {
                tracing::info!("Reconnecting to workspace: {}", addr.id.fmt_short());
                self.connect_with_iroh_addr(
                    addr,
                    Some(ws.session_id().to_string()),
                    Some(ws.clone()),
                    window,
                    cx,
                );
            }
            Err(e) => {
                tracing::error!("Failed to decode endpoint addr: {}", e);
                WorkspaceState::remove(ws.endpoint_addr());
                self.reload_workspace_states(cx);
            }
        }
    }

    fn handle_deeplink(
        &mut self,
        action: DeeplinkAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            DeeplinkAction::Connect(ticket) => {
                tracing::info!("Deeplink: connect action");
                self.connect_with_pairing_ticket(ticket, window, cx);
            }
        }
    }

    /// Connect after scanning a QR pairing ticket.
    ///
    /// Extracts the EndpointAddr from the ticket, creates the workspace, then
    /// stores the ticket (for one-use Register) and signer on the handle.
    fn connect_with_pairing_ticket(
        &mut self,
        ticket: zedra_rpc::ZedraPairingTicket,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let addr = iroh::EndpointAddr::from(ticket.endpoint_id);
        self.connect_with_iroh_addr(addr, None, None, window, cx);
        // Store the ticket and signer on the just-created handle
        if let Some(entry) = self.workspaces.last() {
            entry.handle.set_pending_ticket(ticket);
            // Signer is already set inside connect_with_iroh_addr, but set again
            // to be explicit (idempotent with same key)
        }
    }

    fn connect_with_iroh_addr(
        &mut self,
        addr: iroh::EndpointAddr,
        // If `None`, a new session ID will be generated.
        session_id: Option<String>,
        // Pre-seeded display state from a saved workspace entry (reconnect case).
        // For new pairings pass `None`; the state starts minimal and fills in post-connect.
        saved: Option<WorkspaceState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let endpoint_short = addr.id.fmt_short().to_string();
        tracing::info!("QR connect: starting iroh connection to {}", endpoint_short);

        let encoded_addr = zedra_rpc::pairing::encode_endpoint_addr(&addr).unwrap_or_default();

        // Create a per-workspace session handle
        let session_handle = SessionHandle::new();

        // Load and store the client signing key for PKI auth
        if let Some(signer) = load_client_signer() {
            session_handle.set_signer(signer);
        } else {
            tracing::warn!("connect: no client signer available — PKI auth will be skipped");
        }

        // Store session ID BEFORE persist and BEFORE spawning the async task.
        // This ensures the async task reads the correct session_id when calling
        // handle.session_id() inside connect_with_iroh, and that
        // persist_current_workspaces() doesn't wipe the saved session_id from disk.
        session_handle.set_session_id(session_id);

        // Create the workspace view (creates its own terminal view + pending slots)
        let handle_for_view = session_handle.clone();
        let workspace_view = cx.new(|cx| WorkspaceView::new(handle_for_view, window, cx));

        // Grab pending slots so the async task can signal the UI
        let pending_term_id = workspace_view.read(cx).pending_terminal_id.clone();
        let pending_existing_terminals = workspace_view.read(cx).pending_existing_terminals.clone();

        // Compute terminal dimensions for the async terminal_create call
        let (cols, rows, _, _) = compute_terminal_dimensions(window);
        let cols_u16 = cols as u16;
        let rows_u16 = rows as u16;

        // Subscribe to workspace events
        let view_entity = workspace_view.clone();
        let sub = cx.subscribe_in(
            &workspace_view,
            window,
            move |this: &mut Self,
                  _emitter: &Entity<WorkspaceView>,
                  event: &WorkspaceEvent,
                  _window,
                  cx| {
                match event {
                    WorkspaceEvent::GoHome => {
                        zedra_telemetry::send(Event::ScreenView { screen: "home" });
                        this.screen = AppScreen::Home;
                        cx.notify();
                    }
                    WorkspaceEvent::OpenQuickAction => {
                        this.qa_drawer.update(cx, |h, cx| h.open(cx));
                    }
                    WorkspaceEvent::Disconnected => {
                        zedra_telemetry::send(Event::Disconnect);
                        this.workspaces.retain(|e| e.view != view_entity);
                        tracing::info!(
                            "Workspace disconnected; {} remaining",
                            this.workspaces.len()
                        );
                        this.active_workspace = if this.workspaces.is_empty() {
                            None
                        } else {
                            Some(0)
                        };
                        this.screen = if this.workspaces.is_empty() {
                            AppScreen::Home
                        } else {
                            AppScreen::Workspace
                        };
                        // Reload from disk; workspace_index links are rebuilt
                        this.reload_workspace_states(cx);
                        cx.notify();
                    }
                }
            },
        );
        self._subscriptions.push(sub);

        self.workspaces.push(WorkspaceEntry {
            view: workspace_view,
            handle: session_handle.clone(),
        });
        let ws_idx = self.workspaces.len() - 1;
        self.active_workspace = Some(ws_idx);
        self.screen = AppScreen::Workspace;

        // Update workspace_states: find matching entry or append a new one.
        // Set workspace_index so the home screen links this entry to the active connection.
        if let Some(pos) = self
            .workspace_states
            .iter()
            .position(|s| s.endpoint_addr() == encoded_addr)
        {
            self.workspace_states[pos] =
                WorkspaceState::update_inner(self.workspace_states[pos].clone(), |s| {
                    s.workspace_index = Some(ws_idx);
                });
        } else {
            // New pairing — create a minimal entry from the saved state or defaults
            let new_state = if let Some(s) = saved {
                WorkspaceState::update_inner(s, |s| {
                    s.workspace_index = Some(ws_idx);
                })
            } else {
                WorkspaceState::update_inner(WorkspaceState::default(), |s| {
                    s.endpoint_addr = encoded_addr.clone();
                    s.workspace_index = Some(ws_idx);
                })
            };
            self.workspace_states.push(new_state);
        }

        // Push the initial WorkspaceState into the view so the header shows
        // the saved title immediately, before the handle fields are populated.
        if let Some(state) = self
            .workspace_states
            .iter()
            .find(|s| s.workspace_index() == Some(ws_idx))
        {
            let state = state.clone();
            self.workspaces[ws_idx]
                .view
                .update(cx, |v, cx| v.set_workspace_state(state, cx));
        }

        // Store endpoint addr synchronously so persist_current_workspaces can snapshot it now.
        session_handle.set_endpoint_addr(addr.clone());
        // Notify UI of state changes from background tasks (no GPUI handles captured).
        session_handle.set_state_notifier(|| {
            zedra_session::push_callback(Box::new(|| {}));
        });
        // Save immediately so the workspace survives a quick app-quit before the next
        // periodic persist tick (render_count % 300 == 100).
        self.persist_current_workspaces();

        // Connect asynchronously using the workspace's handle
        let handle_for_connect = session_handle.clone();
        let endpoint_display = endpoint_short.clone();
        zedra_session::session_runtime().spawn(async move {
            tracing::info!("connecting via iroh to {}...", endpoint_display);
            match handle_for_connect.connect(addr).await {
                Ok(()) => {
                    tracing::info!("connected via iroh!");

                    // Check for existing server-side terminals (session resume case).
                    // If found, attach them and restore the UI; otherwise create a new terminal.
                    match handle_for_connect.terminal_list().await {
                        Ok(server_ids) if !server_ids.is_empty() => {
                            tracing::info!(
                                "Session resumed: attaching {} existing terminal(s)",
                                server_ids.len()
                            );
                            handle_for_connect.set_resuming_terminals();
                            let t_resume = std::time::Instant::now();
                            let mut attached = Vec::new();
                            for id in &server_ids {
                                match handle_for_connect.terminal_attach_existing(id).await {
                                    Ok(()) => attached.push(id.clone()),
                                    Err(e) => {
                                        tracing::warn!("Failed to attach terminal {}: {}", id, e)
                                    }
                                }
                            }
                            let resume_ms = t_resume.elapsed().as_millis() as u64;
                            if !attached.is_empty() {
                                zedra_telemetry::send(Event::SessionResumed {
                                    terminal_count: attached.len(),
                                    resume_ms,
                                });
                                handle_for_connect.mark_connected_after_resume(resume_ms);
                                pending_existing_terminals.set(attached);
                            } else {
                                // All attaches failed — fall back to creating a new terminal
                                handle_for_connect.mark_connected_after_resume(resume_ms);
                                match handle_for_connect.terminal_create(cols_u16, rows_u16).await {
                                    Ok(term_id) => pending_term_id.set(term_id),
                                    Err(e) => {
                                        tracing::error!("Failed to create remote terminal: {}", e)
                                    }
                                }
                            }
                        }
                        Ok(_) => {
                            // No existing terminals (new session) — create one
                            match handle_for_connect.terminal_create(cols_u16, rows_u16).await {
                                Ok(term_id) => {
                                    tracing::info!("Remote terminal created: {}", term_id);
                                    zedra_telemetry::send(Event::TerminalOpened {
                                        source: "new_session",
                                        terminal_count: 1,
                                    });
                                    pending_term_id.set(term_id);
                                }
                                Err(e) => {
                                    tracing::error!("Failed to create remote terminal: {}", e)
                                }
                            }
                        }
                        Err(e) => {
                            tracing::warn!("terminal_list failed ({}), creating new terminal", e);
                            match handle_for_connect.terminal_create(cols_u16, rows_u16).await {
                                Ok(term_id) => {
                                    tracing::info!("Remote terminal created: {}", term_id);
                                    pending_term_id.set(term_id);
                                }
                                Err(e) => {
                                    tracing::error!("Failed to create remote terminal: {}", e)
                                }
                            }
                        }
                    }

                    // Persist now that the session is Connected and has hostname/workdir.
                    if let Some(snapshot) = WorkspaceState::from_handle(&handle_for_connect) {
                        WorkspaceState::upsert(snapshot);
                    }
                    zedra_session::push_callback(Box::new(|| {}));
                }
                Err(e) => {
                    tracing::error!("iroh connect failed: {}", e);
                }
            }
        });
    }
}

impl Render for ZedraApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_count += 1;

        // Check for deeplink actions (QR scan, tapped URLs, NFC, etc.)
        if let Some(action) = deeplink::take_pending() {
            self.handle_deeplink(action, window, cx);
        }

        // Check for workspace delete confirmed via native action sheet
        if let Some((endpoint_addr, ws_index_opt)) = PENDING_WORKSPACE_DELETE.take() {
            WorkspaceState::remove(&endpoint_addr);
            // Also disconnect the active workspace if it is currently connected
            if let Some(ws_index) = ws_index_opt {
                if ws_index < self.workspaces.len() {
                    self.workspaces[ws_index]
                        .view
                        .update(cx, |ws_view: &mut WorkspaceView, cx| {
                            ws_view.disconnect(cx);
                        });
                }
            }
            self.reload_workspace_states(cx);
        }

        // Periodically persist workspace state (~every 5 seconds)
        if self.render_count % 300 == 100 && !self.workspaces.is_empty() {
            self.persist_current_workspaces();
        }

        // Update workspace_states from live handles.
        // Display fields are only overwritten when non-empty so saved info persists
        // while the connection is in progress.
        for state in self.workspace_states.iter_mut() {
            if let Some(ws_idx) = state.workspace_index() {
                if let Some(entry) = self.workspaces.get(ws_idx) {
                    let connect_phase = entry.handle.connect_phase();
                    let (terminal_ids, active_terminal_id) = entry.view.read(cx).terminal_state();
                    let project_name = entry.handle.project_name();
                    let hostname = entry.handle.hostname();
                    let workdir = entry.handle.workdir();
                    let homedir = entry.handle.homedir();
                    let strip_path = entry.handle.strip_path();
                    let session_id = entry.handle.session_id().unwrap_or_default();
                    *state = WorkspaceState::update_inner(state.clone(), |s| {
                        s.connect_phase = Some(connect_phase);
                        s.terminal_count = terminal_ids.len();
                        s.terminal_ids = terminal_ids;
                        s.active_terminal_id = active_terminal_id;
                        if !project_name.is_empty() {
                            s.project_name = project_name;
                        }
                        if !hostname.is_empty() {
                            s.hostname = hostname;
                        }
                        if !workdir.is_empty() {
                            s.workdir = workdir;
                        }
                        if !homedir.is_empty() {
                            s.homedir = homedir;
                        }
                        if !strip_path.is_empty() {
                            s.strip_path = strip_path;
                        }
                        if !session_id.is_empty() {
                            s.session_id = session_id;
                        }
                    });
                } else {
                    // workspace entry was removed — clear the link
                    *state = WorkspaceState::update_inner(state.clone(), |s| {
                        s.workspace_index = None;
                        s.connect_phase = None;
                    });
                }
            }
        }

        // Push updated WorkspaceState into each workspace view.
        for state in &self.workspace_states {
            if let Some(ws_idx) = state.workspace_index() {
                if let Some(entry) = self.workspaces.get(ws_idx) {
                    let s = state.clone();
                    entry.view.update(cx, |v, cx| v.set_workspace_state(s, cx));
                }
            }
        }

        // Push workspace_states to home view and quick action panel.
        self.push_states_to_views(cx);

        // Sync qa_drawer content to the current active screen view
        let screen_view: AnyView = match self.screen {
            AppScreen::Home => self.home_view.clone().into(),
            AppScreen::Workspace => self
                .active_workspace
                .and_then(|i| self.workspaces.get(i))
                .map(|e| e.view.clone().into())
                .unwrap_or_else(|| self.home_view.clone().into()),
        };
        self.qa_drawer.update(cx, |h, _| h.set_content(screen_view));

        div()
            .size_full()
            .font_family(fonts::MONO_FONT_FAMILY)
            .child(self.qa_drawer.clone())
    }
}

// ---------------------------------------------------------------------------
// Global pending state for async → main thread
// ---------------------------------------------------------------------------

use crate::pending::PendingSlot;

static PENDING_WORKSPACE_DELETE: PendingSlot<(String, Option<usize>)> = PendingSlot::new();

/// Open a GPUI window with the correct app view for the current feature flags.
pub fn open_zedra_window(app: &mut App, window_options: WindowOptions) -> Result<AnyWindowHandle> {
    if cfg!(feature = "preview") {
        app.open_window(window_options, |window, cx| {
            let view = cx.new(|cx| crate::app_preview::PreviewApp::new(window, cx));
            window.refresh();
            view
        })
        .map(|h| h.into())
    } else {
        app.open_window(window_options, |window, cx| {
            let view = cx.new(|cx| ZedraApp::new(window, cx));
            window.refresh();
            view
        })
        .map(|h| h.into())
    }
}
