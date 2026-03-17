// ZedraApp — multi-workspace coordinator.
// Manages HomeView, WorkspaceView instances, and QuickActionPanel.

use gpui::*;

use crate::deeplink::{self, DeeplinkAction};
use crate::fonts;
use crate::home_view::{HomeEvent, HomeView, HomeWorkspaceItem};
use crate::platform_bridge::{self, AlertButton};
use crate::quick_action_panel::{QuickActionEvent, QuickActionPanel};
use crate::theme;
use crate::workspace_store;
use crate::workspace_store::PersistedWorkspace;
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
    saved_workspaces: Vec<PersistedWorkspace>,
    active_workspace: Option<usize>,
    quick_action: Entity<QuickActionPanel>,
    quick_action_open: bool,
    /// Animating open or close; cleared after animation completes.
    qa_animating: bool,
    /// true = opening, false = closing
    qa_opening: bool,
    qa_animation_id: u64,
    qa_anim_started: Option<std::time::Instant>,
    render_count: u64,
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

const QA_ANIM_DURATION_MS: u64 = 250;

impl ZedraApp {
    fn open_quick_action(&mut self, cx: &mut Context<Self>) {
        self.quick_action_open = true;
        self.qa_animating = true;
        self.qa_opening = true;
        self.qa_animation_id += 1;
        self.qa_anim_started = Some(std::time::Instant::now());
        cx.notify();
    }

    fn close_quick_action(&mut self, cx: &mut Context<Self>) {
        self.qa_animating = true;
        self.qa_opening = false;
        self.qa_animation_id += 1;
        self.qa_anim_started = Some(std::time::Instant::now());
        cx.notify();
    }

    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Load JetBrains Mono font for all UI text
        fonts::load_fonts(window);

        let mut subscriptions = Vec::new();

        // --- Home view ---
        let home_view = cx.new(|cx| HomeView::new(cx));
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
                    let item = this.home_view.read(cx).items.get(*item_idx).cloned();
                    if let Some(item) = item {
                        let saved_index_opt = item.saved.as_ref().map(|(si, _)| *si);
                        let ws_index_opt = item.active.as_ref().map(|(wi, _)| *wi);
                        let title = item
                            .active
                            .as_ref()
                            .and_then(|(_, s)| s.project_path.as_deref())
                            .unwrap_or("Workspace")
                            .rsplit('/')
                            .next()
                            .unwrap_or("Workspace")
                            .to_string();
                        platform_bridge::show_alert(
                            "",
                            &format!("Remove {} workspace?", title),
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
        let quick_action = cx.new(|cx| QuickActionPanel::new(cx));
        let sub = cx.subscribe_in(
            &quick_action,
            window,
            |this: &mut Self, _emitter, event: &QuickActionEvent, window, cx| match event {
                QuickActionEvent::Close => {
                    this.close_quick_action(cx);
                }
                QuickActionEvent::GoHome => {
                    this.close_quick_action(cx);
                    this.screen = AppScreen::Home;
                }
                QuickActionEvent::SwitchToWorkspace(index) => {
                    this.close_quick_action(cx);
                    this.switch_to_workspace(*index, window, cx);
                }
                QuickActionEvent::SwitchToTerminal(ws_index, tid) => {
                    this.close_quick_action(cx);
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

        let mut app = Self {
            screen: AppScreen::Home,
            home_view,
            workspaces: Vec::new(),
            saved_workspaces: Vec::new(),
            active_workspace: None,
            quick_action,
            quick_action_open: false,
            qa_animating: false,
            qa_opening: false,
            qa_animation_id: 0,
            qa_anim_started: None,
            render_count: 0,
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
        self.saved_workspaces = workspace_store::load_workspaces();
        log::info!("Saved workspaces: {}", self.saved_workspaces.len());
        let items = self.build_home_items(&[]);
        self.home_view
            .update(cx, |hv, cx| hv.update_items(items, cx));
    }

    fn build_home_items(
        &self,
        summaries: &[crate::workspace_view::WorkspaceSummary],
    ) -> Vec<HomeWorkspaceItem> {
        let mut matched_ws = vec![false; summaries.len()];
        let mut items: Vec<HomeWorkspaceItem> = Vec::new();
        for (saved_idx, sw) in self.saved_workspaces.iter().enumerate() {
            if let Some((ws_idx, summary)) = summaries.iter().enumerate().find(|(_, s)| {
                s.endpoint_addr_encoded.as_deref() == Some(sw.endpoint_addr.as_str())
            }) {
                matched_ws[ws_idx] = true;
                items.push(HomeWorkspaceItem {
                    active: Some((ws_idx, summary.clone())),
                    saved: Some((saved_idx, sw.clone())),
                });
            } else {
                items.push(HomeWorkspaceItem {
                    active: None,
                    saved: Some((saved_idx, sw.clone())),
                });
            }
        }
        for (ws_idx, summary) in summaries.iter().enumerate() {
            if !matched_ws[ws_idx] {
                items.insert(
                    0,
                    HomeWorkspaceItem {
                        active: Some((ws_idx, summary.clone())),
                        saved: None,
                    },
                );
            }
        }
        items
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
        let ws = match self.saved_workspaces.get(saved_index) {
            Some(ws) => ws.clone(),
            None => {
                log::error!("Saved workspace index {} out of range", saved_index);
                return;
            }
        };

        match zedra_rpc::pairing::decode_endpoint_addr(&ws.endpoint_addr) {
            Ok(addr) => {
                log::info!("Reconnecting to saved workspace: {}", addr.id.fmt_short());
                self.connect_with_iroh_addr(addr, ws.session_id.clone(), window, cx);
            }
            Err(e) => {
                log::error!("Failed to decode saved endpoint addr: {}", e);
                workspace_store::remove_workspace(&ws.endpoint_addr);
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
                        this.open_quick_action(cx);
                    }
                    WorkspaceEvent::Disconnected => {
                        this.workspaces.retain(|e| e.view != view_entity);
                        log::info!("Workspace disconnected; {} remaining", this.workspaces.len());
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
                            let mut attached = Vec::new();
                            for id in &server_ids {
                                match handle_for_connect.terminal_attach_existing(id).await {
                                    Ok(()) => attached.push(id.clone()),
                                    Err(e) => {
                                        log::warn!("Failed to attach terminal {}: {}", id, e)
                                    }
                                }
                            }
                            if !attached.is_empty() {
                                pending_existing_terminals.set(attached);
                            } else {
                                // All attaches failed — fall back to creating a new terminal
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
                    if let Some(snapshot) =
                        workspace_store::snapshot_from_handle(&handle_for_connect)
                    {
                        workspace_store::upsert_workspace(snapshot);
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
                let saved = workspace_store::load_workspaces();
                if let Some(ws) = saved.get(saved_index) {
                    workspace_store::remove_workspace(&ws.endpoint_addr);
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

        // Rebuild the home item list only when on the home screen AND active workspaces
        // exist (their session state changes per-frame). Saved-only items are pushed
        // directly by refresh_saved_workspaces and do not need per-frame rebuilding.
        if self.screen == AppScreen::Home && !summaries.is_empty() {
            let items = self.build_home_items(&summaries);
            self.home_view.update(cx, |hv, cx| {
                hv.update_items(items, cx);
            });
        }
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
            .font_family(fonts::MONO_FONT_FAMILY)
            .child(screen_content);

        // Clear completed close animations
        if let Some(started) = self.qa_anim_started {
            if started.elapsed() >= std::time::Duration::from_millis(QA_ANIM_DURATION_MS + 30) {
                self.qa_animating = false;
                self.qa_anim_started = None;
                if !self.qa_opening {
                    self.quick_action_open = false;
                }
            }
        }

        // Quick action overlay (rendered on top with high priority)
        if self.quick_action_open {
            let animating = self.qa_animating;
            let opening = self.qa_opening;
            let anim_id = self.qa_animation_id;
            let viewport_w = f32::from(window.viewport_size().width);

            // Backdrop
            let backdrop = div()
                .absolute()
                .inset_0()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _event: &MouseDownEvent, _window, cx| {
                        this.close_quick_action(cx);
                    }),
                );

            let backdrop: AnyElement = if animating {
                let (from, to) = if opening { (0.0, 0.4) } else { (0.4, 0.0) };
                backdrop
                    .with_animation(
                        ElementId::NamedInteger("qa-backdrop".into(), anim_id),
                        Animation::new(std::time::Duration::from_millis(QA_ANIM_DURATION_MS))
                            .with_easing(ease_out_quint()),
                        move |elem, delta| {
                            let opacity = from + (to - from) * delta;
                            elem.bg(hsla(0.0, 0.0, 0.0, opacity))
                        },
                    )
                    .into_any_element()
            } else {
                backdrop.bg(hsla(0.0, 0.0, 0.0, 0.4)).into_any_element()
            };

            // Panel wrapper (slides from right).
            // Uses w/h matching viewport so the panel's `right(0)` stays correct.
            let viewport_h = f32::from(window.viewport_size().height);
            let panel_wrapper = div()
                .absolute()
                .top_0()
                .w(px(viewport_w))
                .h(px(viewport_h))
                .child(self.quick_action.clone());

            let panel: AnyElement = if animating {
                let (from, to) = if opening {
                    (viewport_w, 0.0)
                } else {
                    (0.0, viewport_w)
                };
                panel_wrapper
                    .with_animation(
                        ElementId::NamedInteger("qa-panel".into(), anim_id),
                        Animation::new(std::time::Duration::from_millis(QA_ANIM_DURATION_MS))
                            .with_easing(ease_out_quint()),
                        move |elem, delta| {
                            let offset = from + (to - from) * delta;
                            elem.left(px(offset))
                        },
                    )
                    .into_any_element()
            } else {
                panel_wrapper.into_any_element()
            };

            root = root.child(
                deferred(
                    div()
                        .absolute()
                        .inset_0()
                        .child(backdrop)
                        .child(panel),
                )
                .with_priority(200),
            );
        }

        root
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
