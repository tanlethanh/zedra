// ZedraApp — multi-workspace coordinator.
// Manages HomeView, WorkspaceView instances, and QuickActionPanel.

use std::sync::Arc;
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};

use gpui::*;

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
    saved_workspaces: Vec<WorkspaceState>,
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
            log::error!("Failed to load client signing key: {}", e);
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
                    log::info!("Home: Scan QR tapped");
                    platform_bridge::bridge().launch_qr_scanner();
                }
                HomeEvent::WorkspaceTapped(index) => {
                    this.switch_to_workspace(*index, window, cx);
                }
                HomeEvent::SavedWorkspaceTapped(index) => {
                    this.reconnect_saved_workspace(*index, window, cx);
                }
                HomeEvent::WorkspaceRemoved(item_idx) => {
                    let item = this.home_view.read(cx).states.get(*item_idx).cloned();
                    if let Some(item) = item {
                        let saved_index_opt = item.saved_index();
                        let ws_index_opt = item.workspace_index();
                        platform_bridge::show_alert(
                            "",
                            &format!("Remove {} workspace?", item.display_name()),
                            vec![
                                AlertButton::destructive("Delete"),
                                AlertButton::cancel("Cancel"),
                            ],
                            move |button_index| {
                                if button_index == 0 {
                                    PENDING_WORKSPACE_DELETE.set((saved_index_opt, ws_index_opt));
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
            },
        );
        subscriptions.push(sub);

        // --- Window activation (foreground/background) ---
        let sub = cx.observe_window_activation(window, |this: &mut Self, window, _cx| {
            if window.is_window_active() {
                log::info!(
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
            saved_workspaces: Vec::new(),
            active_workspace: None,
            quick_action,
            qa_drawer,
            render_count: 0,
            last_states_revision: None,
            _subscriptions: subscriptions,
        };

        // Load saved workspaces from disk
        app.refresh_saved_workspaces(cx);

        app
    }

    fn switch_to_workspace(&mut self, index: usize, _window: &mut Window, cx: &mut Context<Self>) {
        if index < self.workspaces.len() {
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

    fn refresh_saved_workspaces(&mut self, cx: &mut Context<Self>) {
        self.saved_workspaces = WorkspaceState::load();
        log::info!("Saved workspaces: {}", self.saved_workspaces.len());
        self.sync_workspace_states(self.build_home_items(&[]), cx);
    }

    fn build_home_items(&self, summaries: &[WorkspaceState]) -> Vec<WorkspaceState> {
        let mut matched_ws = vec![false; summaries.len()];
        let mut items: Vec<WorkspaceState> = Vec::new();
        for (saved_idx, sw) in self.saved_workspaces.iter().enumerate() {
            if let Some((ws_idx, summary)) = summaries
                .iter()
                .enumerate()
                .find(|(_, s)| s.endpoint_addr_encoded() == Some(sw.endpoint_addr()))
            {
                matched_ws[ws_idx] = true;
                items.push(summary.clone().with_saved_index(saved_idx));
            } else {
                items.push(sw.clone().for_saved_row(saved_idx));
            }
        }
        for (ws_idx, summary) in summaries.iter().enumerate() {
            if !matched_ws[ws_idx] {
                items.insert(0, summary.clone());
            }
        }
        items
    }

    fn states_revision(items: &[WorkspaceState]) -> u64 {
        let mut hasher = DefaultHasher::new();
        items.len().hash(&mut hasher);
        for state in items {
            state.workspace_index().hash(&mut hasher);
            state.saved_index().hash(&mut hasher);
            state.endpoint_addr().hash(&mut hasher);
            state.endpoint_addr_encoded().hash(&mut hasher);
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

    fn sync_workspace_states(&mut self, items: Vec<WorkspaceState>, cx: &mut Context<Self>) {
        let revision = Self::states_revision(&items);
        if self.last_states_revision == Some(revision) {
            return;
        }
        self.last_states_revision = Some(revision);
        let shared: SharedWorkspaceStates = Arc::new(items);
        self.home_view.update(cx, |hv, cx| {
            hv.set_states(Arc::clone(&shared));
            cx.notify();
        });
        self.quick_action.update(cx, |qa, cx| {
            qa.set_states(Arc::clone(&shared));
            cx.notify();
        });
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

    fn reconnect_saved_workspace(
        &mut self,
        saved_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ws = match self.saved_workspaces.get(saved_index) {
            Some(ws) => ws.clone(),
            None => {
                log::error!("Saved workspace index {} out of range", saved_index);
                return;
            }
        };

        match zedra_rpc::pairing::decode_endpoint_addr(ws.endpoint_addr()) {
            Ok(addr) => {
                log::info!("Reconnecting to saved workspace: {}", addr.id.fmt_short());
                self.connect_with_iroh_addr(addr, Some(ws.session_id().to_string()), window, cx);
            }
            Err(e) => {
                log::error!("Failed to decode saved endpoint addr: {}", e);
                WorkspaceState::remove(ws.endpoint_addr());
                self.refresh_saved_workspaces(cx);
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
                log::info!("Deeplink: connect action");
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
        self.connect_with_iroh_addr(addr, None, window, cx);
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let endpoint_short = addr.id.fmt_short().to_string();
        log::info!("QR connect: starting iroh connection to {}", endpoint_short);

        // Create a per-workspace session handle
        let session_handle = SessionHandle::new();

        // Load and store the client signing key for PKI auth
        if let Some(signer) = load_client_signer() {
            session_handle.set_signer(signer);
        } else {
            log::warn!("connect: no client signer available — PKI auth will be skipped");
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
                        this.screen = AppScreen::Home;
                        cx.notify();
                    }
                    WorkspaceEvent::OpenQuickAction => {
                        this.qa_drawer.update(cx, |h, cx| h.open(cx));
                    }
                    WorkspaceEvent::Disconnected => {
                        this.workspaces.retain(|e| e.view != view_entity);
                        log::info!(
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
                        // Refresh saved workspaces; render() will rebuild the unified home list
                        this.refresh_saved_workspaces(cx);
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
        self.active_workspace = Some(self.workspaces.len() - 1);
        self.screen = AppScreen::Workspace;

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
            log::info!("connecting via iroh to {}...", endpoint_display);
            match handle_for_connect.connect(addr).await {
                Ok(()) => {
                    log::info!("connected via iroh!");

                    // Check for existing server-side terminals (session resume case).
                    // If found, attach them and restore the UI; otherwise create a new terminal.
                    match handle_for_connect.terminal_list().await {
                        Ok(server_ids) if !server_ids.is_empty() => {
                            log::info!(
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
                                        log::warn!("Failed to attach terminal {}: {}", id, e)
                                    }
                                }
                            }
                            let resume_ms = t_resume.elapsed().as_millis() as u64;
                            if !attached.is_empty() {
                                handle_for_connect.mark_connected_after_resume(resume_ms);
                                pending_existing_terminals.set(attached);
                            } else {
                                // All attaches failed — fall back to creating a new terminal
                                handle_for_connect.mark_connected_after_resume(resume_ms);
                                match handle_for_connect.terminal_create(cols_u16, rows_u16).await {
                                    Ok(term_id) => pending_term_id.set(term_id),
                                    Err(e) => {
                                        log::error!("Failed to create remote terminal: {}", e)
                                    }
                                }
                            }
                        }
                        Ok(_) => {
                            // No existing terminals (new session) — create one
                            match handle_for_connect.terminal_create(cols_u16, rows_u16).await {
                                Ok(term_id) => {
                                    log::info!("Remote terminal created: {}", term_id);
                                    pending_term_id.set(term_id);
                                }
                                Err(e) => log::error!("Failed to create remote terminal: {}", e),
                            }
                        }
                        Err(e) => {
                            log::warn!("terminal_list failed ({}), creating new terminal", e);
                            match handle_for_connect.terminal_create(cols_u16, rows_u16).await {
                                Ok(term_id) => {
                                    log::info!("Remote terminal created: {}", term_id);
                                    pending_term_id.set(term_id);
                                }
                                Err(e) => log::error!("Failed to create remote terminal: {}", e),
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
                    log::error!("iroh connect failed: {}", e);
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
        if let Some((saved_index_opt, ws_index_opt)) = PENDING_WORKSPACE_DELETE.take() {
            if let Some(saved_index) = saved_index_opt {
                let saved = WorkspaceState::load();
                if let Some(ws) = saved.get(saved_index) {
                    WorkspaceState::remove(ws.endpoint_addr());
                }
            }
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
            self.refresh_saved_workspaces(cx);
        }

        // Periodically persist workspace state (~every 5 seconds)
        if self.render_count % 300 == 100 && !self.workspaces.is_empty() {
            self.persist_current_workspaces();
        }

        // Build workspace summaries (needed for QuickActionPanel and live home list).
        // Only computed when there are active workspaces; with none, home items are
        // stable and already up-to-date from refresh_saved_workspaces.
        let summaries: Vec<_> = self
            .workspaces
            .iter()
            .enumerate()
            .map(|(i, e)| e.view.read(cx).summary(i))
            .collect();

        // Keep home and quick-action in sync from one shared snapshot regardless of
        // current screen, while avoiding redundant per-frame updates.
        self.sync_workspace_states(self.build_home_items(&summaries), cx);

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

static PENDING_WORKSPACE_DELETE: PendingSlot<(Option<usize>, Option<usize>)> = PendingSlot::new();

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
