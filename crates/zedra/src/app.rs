// ZedraApp — multi-workspace coordinator.
// Manages HomeView, WorkspaceView instances, and QuickActionPanel.

use gpui::*;

use crate::home_view::{HomeEvent, HomeView};
use crate::quick_action_panel::{QuickActionEvent, QuickActionPanel};
use crate::theme;
use crate::workspace_store;
use crate::workspace_view::{WorkspaceEvent, WorkspaceView, compute_terminal_dimensions};
use zedra_session::{RemoteSession, SessionHandle};

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

// ---------------------------------------------------------------------------
// ZedraApp
// ---------------------------------------------------------------------------

pub struct ZedraApp {
    screen: AppScreen,
    home_view: Entity<HomeView>,
    workspaces: Vec<WorkspaceEntry>,
    active_workspace: Option<usize>,
    quick_action: Entity<QuickActionPanel>,
    quick_action_open: bool,
    render_count: u64,
    _subscriptions: Vec<Subscription>,
}

impl ZedraApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Load JetBrains Mono font for all UI text
        zedra_terminal::load_terminal_font(window);

        let mut subscriptions = Vec::new();

        // --- Home view ---
        let home_view = cx.new(|cx| HomeView::new(cx));
        let sub = cx.subscribe_in(
            &home_view,
            window,
            |this: &mut Self, _emitter, event: &HomeEvent, window, cx| match event {
                HomeEvent::ScanQrTapped => {
                    log::info!("Home: Scan QR tapped");
                    crate::platform_bridge::bridge().launch_qr_scanner();
                }
                HomeEvent::WorkspaceTapped(index) => {
                    this.switch_to_workspace(*index, window, cx);
                }
                HomeEvent::SavedWorkspaceTapped(index) => {
                    this.reconnect_saved_workspace(*index, window, cx);
                }
                HomeEvent::SavedWorkspaceRemoved(index) => {
                    let saved = workspace_store::load_workspaces();
                    if let Some(ws) = saved.get(*index) {
                        workspace_store::remove_workspace(&ws.endpoint_addr);
                        this.refresh_saved_workspaces(cx);
                    }
                }
            },
        );
        subscriptions.push(sub);

        // --- Quick action panel ---
        let quick_action = cx.new(|cx| QuickActionPanel::new(cx));
        let sub = cx.subscribe_in(
            &quick_action,
            window,
            |this: &mut Self, _emitter, event: &QuickActionEvent, window, cx| match event {
                QuickActionEvent::Close => {
                    this.quick_action_open = false;
                    cx.notify();
                }
                QuickActionEvent::GoHome => {
                    this.quick_action_open = false;
                    this.screen = AppScreen::Home;
                    cx.notify();
                }
                QuickActionEvent::SwitchToWorkspace(index) => {
                    this.quick_action_open = false;
                    this.switch_to_workspace(*index, window, cx);
                }
                QuickActionEvent::SwitchToTerminal(ws_index, tid) => {
                    this.quick_action_open = false;
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
                log::info!("ZedraApp: window activated, notifying {} workspace(s)", this.workspaces.len());
                for entry in &this.workspaces {
                    zedra_session::notify_foreground_resume(&entry.handle);
                }
            }
        });
        subscriptions.push(sub);

        let app = Self {
            screen: AppScreen::Home,
            home_view,
            workspaces: Vec::new(),
            active_workspace: None,
            quick_action,
            quick_action_open: false,
            render_count: 0,
            _subscriptions: subscriptions,
        };

        // Load saved workspaces from disk and show in HomeView
        app.refresh_saved_workspaces(cx);

        app
    }

    fn switch_to_workspace(&mut self, index: usize, _window: &mut Window, cx: &mut Context<Self>) {
        if index < self.workspaces.len() {
            self.active_workspace = Some(index);
            self.screen = AppScreen::Workspace;
            // Set the active session handle so backward-compat rendering code
            // (active_session(), reconnect_attempt(), etc.) reads the right workspace.
            zedra_session::set_active_handle(self.workspaces[index].handle.clone());
            cx.notify();
        }
    }

    fn refresh_saved_workspaces(&self, cx: &mut Context<Self>) {
        let saved = workspace_store::load_workspaces();
        self.home_view.update(cx, |hv, cx| {
            hv.update_saved_workspaces(saved, cx);
        });
    }

    fn persist_current_workspaces(&self) {
        let persisted: Vec<_> = self
            .workspaces
            .iter()
            .filter_map(|entry| workspace_store::snapshot_from_handle(&entry.handle))
            .collect();
        if !persisted.is_empty() {
            // Upsert each workspace (preserves saved workspaces we're not connected to)
            for ws in persisted {
                workspace_store::upsert_workspace(ws);
            }
        }
    }

    fn reconnect_saved_workspace(
        &mut self,
        saved_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let saved = workspace_store::load_workspaces();
        let ws = match saved.get(saved_index) {
            Some(ws) => ws.clone(),
            None => {
                log::error!("Saved workspace index {} out of range", saved_index);
                return;
            }
        };

        match zedra_rpc::pairing::decode_endpoint_addr(&ws.endpoint_addr) {
            Ok(addr) => {
                log::info!(
                    "Reconnecting to saved workspace: {}",
                    addr.id.fmt_short()
                );
                // Pre-load credentials into the session handle after connection
                let session_id = ws.session_id.clone();
                let auth_token = ws.auth_token.clone();
                self.connect_with_iroh_addr(addr, window, cx);
                // Pre-load saved credentials into the newest workspace's handle
                if let Some(entry) = self.workspaces.last() {
                    entry.handle.store_credentials_pub(session_id, auth_token);
                }
            }
            Err(e) => {
                log::error!("Failed to decode saved endpoint addr: {}", e);
                workspace_store::remove_workspace(&ws.endpoint_addr);
                self.refresh_saved_workspaces(cx);
            }
        }
    }

    fn connect_with_iroh_addr(
        &mut self,
        addr: iroh::EndpointAddr,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let endpoint_short = addr.id.fmt_short().to_string();
        log::info!("QR connect: starting iroh connection to {}", endpoint_short);

        // Create a per-workspace session handle
        let session_handle = SessionHandle::new();

        // Create the workspace view (creates its own terminal view + pending slots)
        let handle_for_view = session_handle.clone();
        let workspace_view = cx.new(|cx| WorkspaceView::new(handle_for_view, window, cx));

        // Grab the workspace's pending_terminal_id so async task can write to it
        let pending_term_id = workspace_view.read(cx).pending_terminal_id.clone();

        // Compute terminal dimensions for the async terminal_create call
        let (cols, rows, _, _) = compute_terminal_dimensions(window);
        let cols_u16 = cols as u16;
        let rows_u16 = rows as u16;

        // Subscribe to workspace events
        let workspace_index = self.workspaces.len();
        let home_view_clone = self.home_view.clone();
        let sub = cx.subscribe_in(
            &workspace_view,
            window,
            move |this: &mut Self,
                  _emitter: &Entity<WorkspaceView>,
                  event: &WorkspaceEvent,
                  _window,
                  cx| {
                match event {
                    WorkspaceEvent::OpenQuickAction => {
                        this.quick_action_open = true;
                        cx.notify();
                    }
                    WorkspaceEvent::Disconnected => {
                        log::info!("Workspace {} disconnected", workspace_index);
                        if workspace_index < this.workspaces.len() {
                            this.workspaces.remove(workspace_index);
                        }
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
                        // Update active handle after workspace removal
                        if let Some(idx) = this.active_workspace {
                            zedra_session::set_active_handle(
                                this.workspaces[idx].handle.clone(),
                            );
                        } else {
                            zedra_session::clear_active_handle();
                        }
                        let summaries: Vec<_> = this
                            .workspaces
                            .iter()
                            .enumerate()
                            .map(|(i, e)| e.view.read(cx).summary(i))
                            .collect();
                        home_view_clone.update(cx, |hv, cx| {
                            hv.update_workspaces(summaries, cx);
                        });
                        // Refresh saved workspaces in HomeView after disconnect
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

        // Set this workspace as the active handle for backward-compat globals
        zedra_session::set_active_handle(session_handle.clone());

        // Connect asynchronously using the workspace's handle
        let handle_for_connect = session_handle.clone();
        let endpoint_display = endpoint_short.clone();
        zedra_session::session_runtime().spawn(async move {
            log::info!(
                "RemoteSession: connecting via iroh to {}...",
                endpoint_display
            );
            match RemoteSession::connect_with_iroh(addr, &handle_for_connect).await {
                Ok(session) => {
                    log::info!("RemoteSession: connected via iroh!");
                    match session.terminal_create(cols_u16, rows_u16, &handle_for_connect).await {
                        Ok(term_id) => {
                            log::info!("Remote terminal created: {}", term_id);
                            pending_term_id.set(term_id);
                        }
                        Err(e) => log::error!("Failed to create remote terminal: {}", e),
                    }
                    handle_for_connect.set_session(session);
                    zedra_session::signal_terminal_data();
                }
                Err(e) => {
                    log::error!("RemoteSession iroh connect failed: {}", e);
                }
            }
        });
    }
}

impl Render for ZedraApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_count += 1;
        if self.render_count % 60 == 1 {
            log::warn!(
                "ZedraApp::render #{}, screen={:?}",
                self.render_count,
                self.screen
            );
        }

        // Check for QR-scanned endpoint address
        if let Some(addr) = PENDING_QR_ADDR.take() {
            self.connect_with_iroh_addr(addr, window, cx);
        }

        // Periodically persist workspace state (~every 5 seconds)
        if self.render_count % 300 == 100 && !self.workspaces.is_empty() {
            self.persist_current_workspaces();
        }

        // Update workspace summaries in HomeView and QuickActionPanel
        let summaries: Vec<_> = self
            .workspaces
            .iter()
            .enumerate()
            .map(|(i, e)| e.view.read(cx).summary(i))
            .collect();
        self.home_view.update(cx, |hv, cx| {
            hv.update_workspaces(summaries.clone(), cx);
        });
        self.quick_action.update(cx, |qa, cx| {
            qa.update_workspaces(summaries, cx);
        });

        let screen_content: AnyElement = match self.screen {
            AppScreen::Home => div()
                .size_full()
                .child(self.home_view.clone())
                .into_any_element(),
            AppScreen::Workspace => {
                if let Some(idx) = self.active_workspace {
                    if let Some(entry) = self.workspaces.get(idx) {
                        div()
                            .size_full()
                            .bg(rgb(theme::BG_PRIMARY))
                            .flex()
                            .flex_col()
                            .child(div().flex_1().child(entry.view.clone()))
                            .into_any_element()
                    } else {
                        div()
                            .size_full()
                            .child(self.home_view.clone())
                            .into_any_element()
                    }
                } else {
                    div()
                        .size_full()
                        .child(self.home_view.clone())
                        .into_any_element()
                }
            }
        };

        let mut root = div()
            .size_full()
            .font_family(zedra_terminal::TERMINAL_FONT_FAMILY)
            .child(screen_content);

        // Quick action overlay (rendered on top with high priority)
        if self.quick_action_open {
            root = root.child(deferred(self.quick_action.clone()).with_priority(200));
        }

        root
    }
}

// ---------------------------------------------------------------------------
// Global pending state for async → main thread
// ---------------------------------------------------------------------------

use crate::pending::PendingSlot;

static PENDING_QR_ADDR: PendingSlot<iroh::EndpointAddr> = PendingSlot::new();

pub fn set_pending_qr_addr(addr: iroh::EndpointAddr) {
    PENDING_QR_ADDR.set(addr);
}

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

/// Decode a QR-scanned endpoint address and register it for the next connection attempt.
pub fn process_qr_result(qr_data: &str) {
    let payload = qr_data.strip_prefix("zedra://").unwrap_or(qr_data);
    match zedra_rpc::pairing::decode_endpoint_addr(payload) {
        Ok(addr) => {
            log::info!("QR scan: decoded EndpointAddr successfully");
            set_pending_qr_addr(addr);
            zedra_session::signal_terminal_data();
        }
        Err(e) => {
            log::error!("QR scan: failed to decode: {}", e);
        }
    }
}
