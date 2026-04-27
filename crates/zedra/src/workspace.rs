use std::sync::Arc;
use std::time::Duration;

use gpui::{prelude::FluentBuilder as _, *};
use tokio::sync::broadcast;
use tracing::*;
use zedra_rpc::ZedraPairingTicket;
use zedra_rpc::proto::HostEvent;
use zedra_session::{ConnectEvent, Session, SessionHandle, SessionState, signer::ClientSigner};

use crate::active_terminal;
use crate::editor::git_sidebar::GitFileSection;
use crate::pending::{SharedPendingSlot, shared_pending_slot, spawn_periodic_task};
use crate::platform_bridge::{self, AlertButton, HapticFeedback, status_bar_inset};
use crate::terminal_card::{agent_icon, strip_ps1_prefix};
use crate::terminal_state::TerminalState;
use crate::theme;
use crate::transport_badge::phase_indicator_color;
use crate::ui::{DrawerHost, DrawerSide};
use crate::workspace_action::{self, GoHome, OpenQuickAction, RequestDisconnect};
use crate::workspace_action::{
    CloseDrawer, CloseTerminal, CreateNewTerminal, GitCommit, GitShowItemActions, GitStage,
    GitUnstage, OpenFile, OpenGitDiff, OpenTerminal, ShowConnecting, ToggleDrawer,
};
use crate::workspace_connecting::WorkspaceConnecting;
use crate::workspace_drawer::WorkspaceDrawer;
use crate::workspace_editor::WorkspaceEditor;
use crate::workspace_gitdiff::{GitdiffHeaderChanged, WorkspaceGitdiff};
use crate::workspace_state::{WorkspaceState, WorkspaceStateEvent};
use crate::workspace_terminal::{TERMINAL_PENDING_ID, WorkspaceTerminal};
use zedra_terminal::view::TerminalView;

/// Events emitted by the workspace.
/// The receiver is mostly app/workspaces
#[derive(Clone, Debug)]
pub enum WorkspaceEvent {
    GoHome,
    OpenQuickAction,
    Disconnected,
}

impl EventEmitter<WorkspaceEvent> for Workspace {}

pub struct Workspace {
    drawer_host: Entity<DrawerHost>,
    #[allow(dead_code)]
    drawer: Entity<WorkspaceDrawer>,
    content: Entity<WorkspaceContent>,
    workspace_state: Entity<WorkspaceState>,
    session_state: Entity<SessionState>,
    terminal_state: Entity<TerminalState>,
    session: Session,
    editor: Entity<WorkspaceEditor>,
    gitdiff: Entity<WorkspaceGitdiff>,
    terminals: Vec<Entity<WorkspaceTerminal>>,
    /// Becomes true once a ReconnectStarted event is seen; gates initial auto-open/create.
    seen_reconnect: bool,
    /// Listens for connect events and syncs them into SessionState/WorkspaceState.
    _connect_listener: Option<Task<()>>,
    /// Listens for host events/actions from the remote host.
    _host_event_listener: Option<Task<()>>,
    /// Listens for periodic host resource snapshots.
    _host_info_listener: Option<Task<()>>,
    pending_confirmation: SharedPendingSlot<PendingWorkspaceConfirmation>,
    _pending_confirmation_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

enum PendingWorkspaceConfirmation {
    DisconnectSession,
    DeleteTerminal { id: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SyncRefreshMode {
    InitialConnect,
    Reconnect,
}

fn sync_refresh_mode_for_event(
    event: &ConnectEvent,
    seen_reconnect: &mut bool,
) -> Option<SyncRefreshMode> {
    if matches!(event, ConnectEvent::ReconnectStarted { .. }) {
        *seen_reconnect = true;
    }

    if !matches!(event, ConnectEvent::SyncComplete { .. }) {
        return None;
    }

    if *seen_reconnect {
        Some(SyncRefreshMode::Reconnect)
    } else {
        Some(SyncRefreshMode::InitialConnect)
    }
}

fn terminal_id_in_sync(id: &str, terminal_ids: &[String]) -> bool {
    terminal_ids.iter().any(|synced_id| synced_id == id)
}

fn should_keep_terminal_entity(id: &str, terminal_ids: &[String]) -> bool {
    id == TERMINAL_PENDING_ID || terminal_id_in_sync(id, terminal_ids)
}

fn active_terminal_is_stale_after_sync(
    active_terminal_id: Option<&str>,
    terminal_ids: &[String],
) -> bool {
    active_terminal_id.is_some_and(|id| !terminal_id_in_sync(id, terminal_ids))
}

impl Workspace {
    pub fn new(
        workspace_state: Entity<WorkspaceState>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let session = Session::new();
        let session_state = cx.new(|_cx| session.state().clone());
        let terminal_state = cx.new(|_| TerminalState::new());

        let editor = cx.new(|cx| WorkspaceEditor::new(session.handle().clone(), cx));
        let gitdiff = cx.new(|cx| WorkspaceGitdiff::new(session.handle().clone(), cx));

        let content = cx.new(|cx| {
            WorkspaceContent::new(
                workspace_state.clone(),
                terminal_state.clone(),
                session_state.clone(),
                session.handle().clone(),
                cx,
            )
        });
        let drawer = cx.new(|cx| {
            WorkspaceDrawer::new(
                _window,
                cx,
                workspace_state.clone(),
                terminal_state.clone(),
                session_state.clone(),
                session.clone(),
                session.handle().clone(),
            )
        });
        let drawer_host = cx.new(|cx| {
            DrawerHost::new(
                content.clone().into(),
                drawer.clone().into(),
                DrawerSide::Left,
                cx,
            )
        });

        let workspace_state_subscription = cx.subscribe(
            &workspace_state,
            |_workspace, workspace_state, event: &WorkspaceStateEvent, _cx| {
                if matches!(event, WorkspaceStateEvent::StateChanged) {
                    WorkspaceState::upsert(workspace_state.read(_cx).clone())
                        .map_err(|e| warn!("failed to upsert workspace state: {}", e))
                        .ok();
                }
            },
        );
        let gitdiff_subscription = cx.subscribe(
            &gitdiff,
            |this, _gitdiff, event: &GitdiffHeaderChanged, cx| {
                this.content.update(cx, |content, cx| {
                    content.set_git_diff_subtitle(
                        event.filename.clone(),
                        event.added,
                        event.removed,
                        cx,
                    );
                });
            },
        );

        let mut host_event_rx = session.subscribe_host_events();
        let host_event_listener = cx.spawn(async move |workspace, cx| {
            loop {
                match host_event_rx.recv().await {
                    Ok(HostEvent::TerminalCreated { .. }) => {
                        let should_break = workspace
                            .update(cx, |ws, cx| {
                                let session_state = ws.session_state.read(cx).clone();
                                ws.workspace_state.update(cx, |this, cx| {
                                    this.sync_from_session(ws.session_handle(), &session_state, cx);
                                    cx.notify();
                                });
                            })
                            .is_err();
                        if should_break {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!("workspace host event listener lagged by {}", skipped);
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        let mut host_info_rx = session.subscribe_host_info();
        let host_info_listener = cx.spawn(async move |workspace, cx| {
            loop {
                match host_info_rx.recv().await {
                    Ok(snapshot) => {
                        let should_break = workspace
                            .update(cx, |ws, cx| {
                                ws.workspace_state.update(cx, |this, cx| {
                                    this.update_host_info(snapshot, cx);
                                    cx.notify();
                                });
                            })
                            .is_err();
                        if should_break {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!("workspace host info listener lagged by {}", skipped);
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        let pending_confirmation = shared_pending_slot();
        let confirmation_slot = pending_confirmation.clone();
        let pending_confirmation_task =
            spawn_periodic_task(cx, Duration::from_millis(50), move |this, cx| {
                if let Some(confirmation) = confirmation_slot.take() {
                    this.process_pending_confirmation(confirmation, cx);
                }
            });

        Self {
            drawer_host,
            drawer,
            content,
            workspace_state,
            session_state,
            terminal_state,
            session,
            editor,
            gitdiff,
            // Terminals will be created after connection is established
            terminals: vec![],
            seen_reconnect: false,
            _connect_listener: None,
            _host_event_listener: Some(host_event_listener),
            _host_info_listener: Some(host_info_listener),
            pending_confirmation,
            _pending_confirmation_task: pending_confirmation_task,
            _subscriptions: vec![workspace_state_subscription, gitdiff_subscription],
        }
    }

    /// Start connection to remote host.
    pub fn connect(
        &mut self,
        addr: iroh::EndpointAddr,
        ticket: Option<ZedraPairingTicket>,
        signer: Arc<dyn ClientSigner>,
        session_id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Spawn GPUI task: reads ConnectEvents → applies to SessionState entity → cx.notify()
        if let Some(mut event_rx) = self.session.take_event_receiver() {
            let closed_notify = self.session.closed_notify();
            self._connect_listener = Some(cx.spawn_in(window, async move |workspace, cx| {
                while let Some(event) = event_rx.recv().await {
                    let is_closed = matches!(event, ConnectEvent::ConnectionClosed);
                    if is_closed {
                        closed_notify.notify_waiters();
                    }

                    let sync_refresh_mode = match workspace.update(cx, |ws, cx| {
                        let sync_refresh_mode =
                            sync_refresh_mode_for_event(&event, &mut ws.seen_reconnect);
                        ws.session_state.update(cx, |state, cx| {
                            state.apply_event(event.clone());
                            cx.notify();
                            ws.workspace_state.update(cx, |this, cx| {
                                this.sync_from_session(ws.session_handle(), state, cx);
                            });
                        });
                        sync_refresh_mode
                    }) {
                        Ok(sync_refresh_mode) => sync_refresh_mode,
                        Err(_) => break,
                    };

                    if let Some(sync_refresh_mode) = sync_refresh_mode {
                        let is_initial_connect =
                            sync_refresh_mode == SyncRefreshMode::InitialConnect;
                        let mut client_ready = false;
                        for _ in 0..200 {
                            client_ready = match workspace
                                .update(cx, |ws, _cx| ws.session_handle().has_client())
                            {
                                Ok(ready) => ready,
                                Err(_) => break,
                            };
                            if client_ready {
                                break;
                            }
                            cx.background_executor()
                                .timer(Duration::from_millis(10))
                                .await;
                        }
                        if !client_ready {
                            warn!("session handle was not ready after SyncComplete");
                        }

                        let refresh_task = match workspace.update(cx, |ws, cx| {
                            ws.drawer
                                .update(cx, |drawer, cx| drawer.refresh_after_sync(cx))
                        }) {
                            Ok(task) => task,
                            Err(_) => break,
                        };

                        if is_initial_connect {
                            refresh_task.await;
                        } else {
                            refresh_task.detach();
                        }

                        let should_initialize = match workspace.update(cx, |ws, cx| {
                            let session_handle = ws.session.handle().clone();
                            let session_state = ws.session_state.clone();
                            let workspace_state = ws.workspace_state.clone();
                            session_state.update(cx, |state, cx| {
                                workspace_state.update(cx, |this, cx| {
                                    this.sync_from_session(&session_handle, state, cx);
                                });
                            });
                            ws.reconcile_terminals_after_sync(cx);
                            ws.workspace_state.update(cx, |this, cx| {
                                this.emit_sync_complete(cx);
                            });
                            ws.content.update(cx, |c, cx| c.hide_connecting_view(cx));
                            is_initial_connect
                        }) {
                            Ok(should_initialize) => should_initialize,
                            Err(_) => break,
                        };

                        if !should_initialize {
                            continue;
                        }

                        let workspace = workspace.clone();
                        cx.on_next_frame(move |window, cx| {
                            let _ = workspace.update(cx, |ws, cx| {
                                ws.initialize_workspace_terminals(window, cx);
                            });
                        });
                    }
                }
            }));
        }

        let session_id = session_id.or_else(|| ticket.as_ref().map(|t| t.session_id.clone()));
        self.session
            .connect(addr, ticket, signer, session_id.clone(), move |_handle| {
                info!("session {:?} connected", session_id);
            });

        self.content.update(cx, |c, cx| c.show_connecting_view(cx));
    }

    /// Called when this workspace becomes the active workspace.
    pub fn on_activate(&mut self, _cx: &mut Context<Self>) {
        //
    }

    pub fn session_handle(&self) -> &SessionHandle {
        self.session.handle()
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Read current workspace state (cheap Arc clone).
    pub fn workspace_state(&self, cx: &App) -> WorkspaceState {
        self.workspace_state.read(cx).clone()
    }

    /// Used to link between Workspace and WorkspaceState
    pub fn endpoint_addr(&self, cx: &App) -> String {
        self.workspace_state.read(cx).endpoint_addr.clone()
    }

    pub fn terminal_state(&self) -> Entity<TerminalState> {
        self.terminal_state.clone()
    }

    /// Programmatically disconnect this workspace.
    pub fn disconnect(&mut self, cx: &mut Context<Self>) {
        self.session.disconnect();
        self.workspace_state.update(cx, |state, cx| {
            state.mark_disconnected(cx);
        });
        cx.emit(WorkspaceEvent::Disconnected);
        cx.notify();
    }

    pub fn open_terminal_from_quick_action(&mut self, id: String, cx: &mut Context<Self>) {
        self.drawer_host.update(cx, |host, cx| host.close(cx));

        if let Some(terminal_entity) = self.terminal_by_id(&id, cx) {
            self.activate_terminal(id, terminal_entity, cx);
        } else {
            warn!("quick action requested uninitialized terminal {}", id);
        }
    }

    pub fn close_terminal_from_quick_action(&mut self, id: String, _cx: &mut Context<Self>) {
        self.request_terminal_delete_confirmation(id);
    }

    // ─── Action Handlers ─────────────────────────────────────────────────────

    fn handle_go_home(&mut self, _action: &GoHome, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(WorkspaceEvent::GoHome);
    }

    fn handle_open_quick_action(
        &mut self,
        _action: &OpenQuickAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        info!("handle OpenQuickAction from workspace");
        window.hide_soft_keyboard();
        cx.emit(WorkspaceEvent::OpenQuickAction);
    }

    fn handle_request_disconnect(
        &mut self,
        _action: &RequestDisconnect,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        info!("handle RequestDisconnect from workspace");
        window.hide_soft_keyboard();

        let pending_confirmation = self.pending_confirmation.clone();
        platform_bridge::show_alert(
            "",
            "Disconnect this session?",
            vec![
                AlertButton::destructive("Disconnect"),
                AlertButton::cancel("Cancel"),
            ],
            move |button_index| {
                if button_index == 0 {
                    pending_confirmation.set(PendingWorkspaceConfirmation::DisconnectSession);
                }
            },
        );
    }

    fn handle_toggle_drawer(
        &mut self,
        _action: &ToggleDrawer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        info!("handle ToggleDrawer from workspace");
        let is_open = self.drawer_host.read(cx).is_open();
        self.drawer_host.update(cx, |host, cx| {
            if is_open {
                host.close_with_window(&mut *window, cx);
            } else {
                host.open_with_window(&mut *window, cx);
            }
        });
    }

    fn handle_close_drawer(
        &mut self,
        _action: &CloseDrawer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        info!("handle CloseDrawer from workspace");
        self.drawer_host
            .update(cx, |host, cx| host.close_with_window(&mut *window, cx));
    }

    fn handle_show_connecting(
        &mut self,
        _action: &ShowConnecting,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        info!("handle ShowConnecting from workspace");
        self.drawer_host
            .update(cx, |host, cx| host.close_with_window(&mut *window, cx));
        self.content.update(cx, |c, cx| c.show_connecting_view(cx));
    }

    fn handle_open_file(&mut self, action: &OpenFile, window: &mut Window, cx: &mut Context<Self>) {
        info!("handle OpenFile from workspace");
        self.drawer_host
            .update(cx, |host, cx| host.close_with_window(&mut *window, cx));

        self.editor.update(cx, |e, cx| {
            e.open_file(action.path.clone(), cx);
        });

        let editor = self.editor.clone();
        self.content.update(cx, move |c, cx| {
            c.clear_subtitle(cx);
            c.set_main_view(editor.into(), cx);
        });
    }

    fn handle_open_git_diff(
        &mut self,
        action: &OpenGitDiff,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        info!("handle OpenGitDiff from workspace");
        self.drawer_host
            .update(cx, |host, cx| host.close_with_window(&mut *window, cx));

        let section = section_from_u8(action.section);
        self.gitdiff.update(cx, |g, cx| {
            g.open_diff(action.path.clone(), section, cx);
        });

        let gitdiff = self.gitdiff.clone();
        self.content.update(cx, move |c, cx| {
            c.set_main_view(gitdiff.into(), cx);
        });
    }

    fn handle_git_stage(
        &mut self,
        action: &GitStage,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        info!("handle GitStage from workspace");
        let handle = self.session.handle().clone();
        let path = action.path.clone();
        cx.spawn(async move |_workspace, _cx| {
            if let Err(e) = handle.git_stage(&[path]).await {
                tracing::error!("git stage failed: {}", e);
            }
        })
        .detach();
    }

    fn handle_git_unstage(
        &mut self,
        action: &GitUnstage,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        info!("handle GitUnstage from workspace");
        let handle = self.session.handle().clone();
        let path = action.path.clone();
        cx.spawn(async move |_workspace, _cx| {
            if let Err(e) = handle.git_unstage(&[path]).await {
                tracing::error!("git unstage failed: {}", e);
            }
        })
        .detach();
    }

    fn handle_git_item_long_press(
        &mut self,
        action: &GitShowItemActions,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        info!("handle GitItemLongPress from workspace");
        let path = action.path.clone();
        let section = section_from_u8(action.section);
        let main_action_label = match section {
            GitFileSection::Staged => "Unstage",
            GitFileSection::Unstaged | GitFileSection::Untracked => "Stage",
        };

        let handle = self.session.handle().clone();
        let display_path = path.clone();

        platform_bridge::show_selection(
            "",
            &display_path,
            vec![AlertButton::default(main_action_label)],
            move |selection| {
                match selection {
                    Some(0) => {
                        let h = handle.clone();
                        let p = path.clone();
                        match section {
                            GitFileSection::Staged => {
                                // TODO: use cx.spawn
                                zedra_session::session_runtime().spawn(async move {
                                    if let Err(e) = h.git_unstage(&[p]).await {
                                        tracing::error!("git unstage failed: {}", e);
                                    }
                                });
                            }
                            _ => {
                                // TODO: use cx.spawn
                                zedra_session::session_runtime().spawn(async move {
                                    if let Err(e) = h.git_stage(&[p]).await {
                                        tracing::error!("git stage failed: {}", e);
                                    }
                                });
                            }
                        }
                    }
                    _ => {}
                }
            },
        );
    }

    fn handle_git_commit(
        &mut self,
        action: &GitCommit,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        info!("handle GitCommit from workspace");
        let message = action.message.trim().to_string();
        let paths = action.paths.clone();
        if message.is_empty() || paths.is_empty() {
            return;
        }

        let file_label = if paths.len() == 1 {
            "1 staged file".to_string()
        } else {
            format!("{} staged files", paths.len())
        };
        let confirm_message = format!("Commit {file_label}?\n\n{message}");

        let handle = self.session.handle().clone();
        window.hide_soft_keyboard();
        platform_bridge::show_alert(
            "",
            &confirm_message,
            vec![
                AlertButton::default("Commit"),
                AlertButton::cancel("Cancel"),
            ],
            move |button_index| {
                if button_index == 0 {
                    let h = handle.clone();
                    let m = message.clone();
                    let p = paths.clone();
                    // TODO: use cx.spawn
                    zedra_session::session_runtime().spawn(async move {
                        match h.git_commit(&m, &p).await {
                            Ok(_) => {
                                tracing::info!("git commit succeeded");
                            }
                            Err(e) => {
                                tracing::error!("git commit failed: {}", e);
                            }
                        }
                    });
                }
            },
        );
    }

    fn handle_create_new_terminal(
        &mut self,
        _action: &CreateNewTerminal,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        info!("handle CreateNewTerminal from workspace");
        self.drawer_host
            .update(cx, |host, cx| host.close_with_window(&mut *window, cx));

        let session_handle = self.session.handle().clone();
        let initial_viewport = self.mainview_viewport(window, cx);
        let initial_grid_size = TerminalView::compute_grid_size(window, initial_viewport);
        let cols = initial_grid_size.columns;
        let rows = initial_grid_size.rows;

        // Pre-create a workspace terminal entity with a placeholder terminal ID.
        // This is required due to the terminal view requires `window` to be available from main thread.
        let workspace_terminal =
            self.create_terminal_entity(TERMINAL_PENDING_ID.to_string(), window, cx);

        cx.spawn(async move |workspace, cx| {
            let terminal_id = match session_handle
                .terminal_create(cols as u16, rows as u16)
                .await
            {
                Ok(id) => id,
                Err(e) => {
                    tracing::error!("terminal_create failed: {}", e);
                    return;
                }
            };

            let _ = workspace.update(cx, |ws, cx| {
                workspace_terminal.update(cx, |terminal, cx| {
                    terminal.set_terminal_id(terminal_id.clone(), cx);
                });

                ws.workspace_state.update(cx, |_state, cx| {
                    // The WorkspaceTerminal will need this subscription to attach the input/output channel.
                    cx.emit(WorkspaceStateEvent::TerminalCreated {
                        id: terminal_id.clone(),
                    });
                });

                ws.activate_terminal(terminal_id, workspace_terminal.into(), cx);
            });
        })
        .detach();
    }

    fn handle_open_terminal(
        &mut self,
        action: &OpenTerminal,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        info!("handle OpenTerminal from workspace");
        self.drawer_host
            .update(cx, |host, cx| host.close_with_window(&mut *window, cx));

        let id = &action.id;
        let terminal_entity = self.terminal_by_id(id, cx).unwrap_or_else(|| {
            info!("terminal not yet tracked locally, creating view for {}", id);
            self.create_terminal_entity(id.clone(), window, cx)
        });

        self.activate_terminal(id.clone(), terminal_entity, cx);
    }

    fn handle_close_terminal(
        &mut self,
        action: &CloseTerminal,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        info!("handle CloseTerminal from workspace");
        window.hide_soft_keyboard();

        self.request_terminal_delete_confirmation(action.id.clone());
    }

    fn activate_terminal(
        &mut self,
        id: String,
        terminal_entity: Entity<WorkspaceTerminal>,
        cx: &mut Context<Self>,
    ) {
        let previous_active_id = self.workspace_state.read(cx).active_terminal_id.clone();
        if previous_active_id.as_deref() != Some(id.as_str()) {
            if let Some(previous_terminal) =
                previous_active_id.and_then(|active_id| self.terminal_by_id(&active_id, cx))
            {
                previous_terminal.update(cx, |terminal, cx| {
                    terminal.deactivate(cx);
                });
            }
        }

        let subtitle_id = id.clone();
        self.workspace_state.update(cx, |state, cx| {
            state.active_terminal_id = Some(id.clone());
            cx.emit(WorkspaceStateEvent::TerminalOpened { id });
            cx.notify();
        });
        self.content.update(cx, |c, cx| {
            c.set_terminal_subtitle(subtitle_id, cx);
            c.set_main_view(terminal_entity.into(), cx);
        });
    }

    fn close_terminal_by_id(&mut self, id: String, cx: &mut Context<Self>) {
        self.terminals.retain(|t| t.read(cx).terminal_id() != id);
        self.session.handle().remove_terminal(&id);

        self.workspace_state.update(cx, |state, cx| {
            state.terminal_ids.retain(|terminal_id| terminal_id != &id);
            if state.active_terminal_id.as_deref() == Some(id.as_str()) {
                state.active_terminal_id = None;
                active_terminal::clear_active_input();
            }
            cx.notify();
        });

        let handle = self.session.handle().clone();
        cx.spawn(async move |_workspace, _cx| {
            if let Err(e) = handle.terminal_close(&id).await {
                tracing::error!("terminal_close failed: {}", e);
            }
        })
        .detach();
    }

    fn reconcile_terminals_after_sync(&mut self, cx: &mut Context<Self>) {
        let terminal_ids = self.workspace_state.read(cx).terminal_ids.clone();
        self.terminals.retain(|terminal| {
            let id = terminal.read(cx).terminal_id().to_string();
            should_keep_terminal_entity(&id, &terminal_ids)
        });

        let active_terminal_id = self.workspace_state.read(cx).active_terminal_id.clone();
        let active_terminal_is_stale =
            active_terminal_is_stale_after_sync(active_terminal_id.as_deref(), &terminal_ids);

        if active_terminal_is_stale {
            active_terminal::clear_active_input();
            self.workspace_state.update(cx, |state, cx| {
                state.active_terminal_id = None;
                cx.notify();
            });
            let editor = self.editor.clone();
            self.content.update(cx, |content, cx| {
                content.clear_subtitle(cx);
                content.set_main_view(editor.into(), cx);
            });
        }
    }

    fn process_pending_confirmation(
        &mut self,
        confirmation: PendingWorkspaceConfirmation,
        cx: &mut Context<Self>,
    ) {
        match confirmation {
            PendingWorkspaceConfirmation::DisconnectSession => self.disconnect(cx),
            PendingWorkspaceConfirmation::DeleteTerminal { id } => {
                self.close_terminal_by_id(id, cx)
            }
        }
    }

    fn request_terminal_delete_confirmation(&self, terminal_id: String) {
        let pending_confirmation = self.pending_confirmation.clone();
        platform_bridge::show_alert(
            "",
            "Delete this terminal?",
            vec![
                AlertButton::destructive("Delete"),
                AlertButton::cancel("Cancel"),
            ],
            move |button_index| {
                if button_index == 0 {
                    pending_confirmation.set(PendingWorkspaceConfirmation::DeleteTerminal {
                        id: terminal_id.clone(),
                    });
                }
            },
        );
    }

    /// Create a new terminal entity and add it to the terminals vec.
    fn create_terminal_entity(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<WorkspaceTerminal> {
        let initial_viewport = self.mainview_viewport(window, cx);
        let entity = cx.new(|cx| {
            WorkspaceTerminal::new(
                id,
                self.workspace_state.clone(),
                self.terminal_state.clone(),
                self.session.handle().clone(),
                window,
                initial_viewport,
                cx,
            )
        });
        self.terminals.push(entity.clone());
        entity
    }

    fn terminal_by_id(
        &self,
        id: &str,
        cx: &mut Context<Self>,
    ) -> Option<Entity<WorkspaceTerminal>> {
        self.terminals
            .iter()
            .find(|t| t.read(cx).terminal_id() == id)
            .cloned()
    }

    fn mainview_viewport(&self, window: &mut Window, cx: &App) -> Size<Pixels> {
        self.content
            .read(cx)
            .mainview_viewport()
            .unwrap_or_else(|| WorkspaceContent::fallback_mainview_viewport(window))
    }

    /// Pre-create WorkspaceTerminal entities for all known IDs so their
    /// Open or create the first terminal.
    fn initialize_workspace_terminals(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let terminal_ids = self.workspace_state.read(cx).terminal_ids.clone();
        let first_id = terminal_ids.first().cloned();

        for id in terminal_ids {
            if self.terminal_by_id(&id, cx).is_none() {
                self.create_terminal_entity(id.clone(), window, cx);
            }
        }

        if let Some(first_id) = first_id {
            info!("auto-opening first terminal on connect: {}", first_id);
            self.handle_open_terminal(&OpenTerminal { id: first_id }, window, cx);
        } else {
            info!("no terminals on connect, creating new terminal");
            self.handle_create_new_terminal(&CreateNewTerminal, window, cx);
        }
    }
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("workspace")
            .key_context("workspace")
            .on_action(cx.listener(Self::handle_go_home))
            .on_action(cx.listener(Self::handle_open_quick_action))
            .on_action(cx.listener(Self::handle_request_disconnect))
            .on_action(cx.listener(Self::handle_toggle_drawer))
            .on_action(cx.listener(Self::handle_close_drawer))
            .on_action(cx.listener(Self::handle_show_connecting))
            .on_action(cx.listener(Self::handle_open_file))
            .on_action(cx.listener(Self::handle_open_git_diff))
            .on_action(cx.listener(Self::handle_git_stage))
            .on_action(cx.listener(Self::handle_git_unstage))
            .on_action(cx.listener(Self::handle_git_item_long_press))
            .on_action(cx.listener(Self::handle_git_commit))
            .on_action(cx.listener(Self::handle_create_new_terminal))
            .on_action(cx.listener(Self::handle_open_terminal))
            .on_action(cx.listener(Self::handle_close_terminal))
            .size_full()
            .child(self.drawer_host.clone())
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn section_from_u8(v: u8) -> GitFileSection {
    match v {
        0 => GitFileSection::Staged,
        1 => GitFileSection::Unstaged,
        _ => GitFileSection::Untracked,
    }
}

pub fn section_to_u8(section: GitFileSection) -> u8 {
    match section {
        GitFileSection::Staged => 0,
        GitFileSection::Unstaged => 1,
        GitFileSection::Untracked => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zedra_rpc::proto::SyncSessionResult;
    use zedra_session::ReconnectReason;

    fn sync_complete_event() -> ConnectEvent {
        ConnectEvent::SyncComplete {
            sync: SyncSessionResult {
                session_id: "session-1".into(),
                session_token: [1; 32],
                hostname: "host".into(),
                workdir: "/workspace".into(),
                username: "user".into(),
                home_dir: Some("/home/user".into()),
                os: Some("macos".into()),
                arch: Some("aarch64".into()),
                os_version: Some("26.0".into()),
                host_version: Some("0.1.1".into()),
                terminals: Vec::new(),
            },
            sync_ms: 7,
        }
    }

    #[::core::prelude::v1::test]
    fn initial_sync_waits_for_drawer_refresh() {
        let mut seen_reconnect = false;

        let mode = sync_refresh_mode_for_event(&sync_complete_event(), &mut seen_reconnect);

        assert_eq!(mode, Some(SyncRefreshMode::InitialConnect));
        assert!(!seen_reconnect);
    }

    #[::core::prelude::v1::test]
    fn reconnect_sync_refreshes_drawer_in_background() {
        let mut seen_reconnect = false;

        let mode = sync_refresh_mode_for_event(
            &ConnectEvent::ReconnectStarted {
                reason: ReconnectReason::ConnectionLost,
            },
            &mut seen_reconnect,
        );

        assert_eq!(mode, None);
        assert!(seen_reconnect);

        let mode = sync_refresh_mode_for_event(&sync_complete_event(), &mut seen_reconnect);

        assert_eq!(mode, Some(SyncRefreshMode::Reconnect));
    }

    #[::core::prelude::v1::test]
    fn terminal_sync_keeps_only_pending_or_synced_terminal_views() {
        let synced = vec!["remote-active".to_string()];

        assert!(should_keep_terminal_entity("remote-active", &synced));
        assert!(should_keep_terminal_entity(TERMINAL_PENDING_ID, &synced));
        assert!(!should_keep_terminal_entity("stale-local", &synced));
    }

    #[::core::prelude::v1::test]
    fn terminal_sync_treats_active_terminal_missing_from_host_as_stale() {
        let synced = vec!["remote-active".to_string()];

        assert!(!active_terminal_is_stale_after_sync(None, &synced));
        assert!(!active_terminal_is_stale_after_sync(
            Some("remote-active"),
            &synced
        ));
        assert!(active_terminal_is_stale_after_sync(
            Some("stale-local"),
            &synced
        ));
        assert!(active_terminal_is_stale_after_sync(
            Some("stale-local"),
            &[]
        ));
    }
}

pub struct WorkspaceContent {
    workspace_state: Entity<WorkspaceState>,
    terminal_state: Entity<TerminalState>,
    #[allow(dead_code)]
    session_handle: SessionHandle,
    subtitle: WorkspaceSubtitle,
    main_view: AnyView,
    focus_handle: FocusHandle,
    show_connecting: bool,
    connecting_view: Entity<WorkspaceConnecting>,
    mainview_bounds: Option<Bounds<Pixels>>,
    _subscriptions: Vec<Subscription>,
}

enum WorkspaceSubtitle {
    Default,
    Terminal {
        id: String,
    },
    GitDiff {
        filename: SharedString,
        added: usize,
        removed: usize,
    },
}

impl WorkspaceContent {
    pub fn new(
        workspace_state: Entity<WorkspaceState>,
        terminal_state: Entity<TerminalState>,
        session_state: Entity<SessionState>,
        session_handle: SessionHandle,
        cx: &mut Context<Self>,
    ) -> Self {
        let empty_view = cx.new(|_cx| Empty);
        let connecting = cx.new(|_cx| WorkspaceConnecting::new(session_state));

        let terminal_state_sub = cx.observe(&terminal_state, |_, _, cx| cx.notify());
        let workspace_state_sub = cx.observe(&workspace_state, |_, _, cx| cx.notify());

        Self {
            main_view: empty_view.into(),
            subtitle: WorkspaceSubtitle::Default,
            focus_handle: cx.focus_handle(),
            session_handle,
            workspace_state,
            terminal_state,
            show_connecting: false,
            connecting_view: connecting,
            mainview_bounds: None,
            _subscriptions: vec![terminal_state_sub, workspace_state_sub],
        }
    }

    pub fn set_main_view(&mut self, view: AnyView, cx: &mut Context<Self>) {
        self.main_view = view;
        cx.notify();
    }

    pub fn clear_subtitle(&mut self, cx: &mut Context<Self>) {
        self.subtitle = WorkspaceSubtitle::Default;
        cx.notify();
    }

    pub fn set_terminal_subtitle(&mut self, id: String, cx: &mut Context<Self>) {
        self.subtitle = WorkspaceSubtitle::Terminal { id };
        cx.notify();
    }

    pub fn set_git_diff_subtitle(
        &mut self,
        filename: String,
        added: usize,
        removed: usize,
        cx: &mut Context<Self>,
    ) {
        self.subtitle = WorkspaceSubtitle::GitDiff {
            filename: filename.into(),
            added,
            removed,
        };
        cx.notify();
    }

    pub fn show_connecting_view(&mut self, cx: &mut Context<Self>) {
        self.show_connecting = true;
        cx.notify();
    }

    pub fn hide_connecting_view(&mut self, cx: &mut Context<Self>) {
        self.show_connecting = false;
        cx.notify();
    }

    pub fn mainview_viewport(&self) -> Option<Size<Pixels>> {
        self.mainview_bounds.as_ref().map(|bounds| bounds.size)
    }

    pub fn fallback_mainview_viewport(window: &mut Window) -> Size<Pixels> {
        let viewport = window.viewport_size();

        Size {
            width: viewport.width,
            height: (viewport.height - px(status_bar_inset() + theme::HEADER_HEIGHT)).max(px(0.0)),
        }
    }

    fn update_mainview_bounds(&mut self, bounds: Bounds<Pixels>) {
        if self.mainview_bounds == Some(bounds) {
            return;
        }

        self.mainview_bounds = Some(bounds);
    }
}

impl Focusable for WorkspaceContent {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for WorkspaceContent {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let top_inset = status_bar_inset();
        let this = cx.weak_entity();
        let workspace_state = self.workspace_state.read(cx);
        let title = workspace_state.project_name.to_string();
        let default_subtitle = workspace_state.strip_path.to_string();
        let net_dot_color = match workspace_state.connect_phase.clone() {
            Some(phase) => phase_indicator_color(&phase),
            None => theme::ACCENT_DIM,
        };
        let mainview_measure = canvas(
            |bounds, _, _| bounds,
            move |_bounds, measured_bounds, _window, cx| {
                cx.defer(move |cx| {
                    let _ = this.update(cx, |this, _cx| {
                        this.update_mainview_bounds(measured_bounds);
                    });
                });
            },
        )
        .absolute()
        .inset_0();

        div()
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .bg(rgb(theme::BG_PRIMARY))
            .child(div().h(px(top_inset)))
            .child(
                div()
                    .h(px(theme::HEADER_HEIGHT))
                    .flex()
                    .flex_row()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(theme::BORDER_SUBTLE))
                    .child(
                        div()
                            .id("drawer-toggle-btn")
                            .w(px(theme::HEADER_BUTTON_SIZE))
                            .h(px(theme::HEADER_BUTTON_SIZE))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hit_slop(px(20.0))
                            .on_press(cx.listener(|_this, _event, window, cx| {
                                platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
                                window.dispatch_action(
                                    workspace_action::ToggleDrawer.boxed_clone(),
                                    cx,
                                );
                            }))
                            .child(
                                svg()
                                    .path("icons/menu.svg")
                                    .size(px(16.0))
                                    .text_color(rgb(theme::TEXT_SECONDARY)),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .items_center()
                                    .w_full()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .gap(px(5.0))
                                            .max_w_full()
                                            .child(
                                                div()
                                                    .w(px(6.0))
                                                    .h(px(6.0))
                                                    .rounded(px(3.0))
                                                    .flex_shrink_0()
                                                    .bg(rgb(net_dot_color)),
                                            )
                                            .child(
                                                div()
                                                    .min_w_0()
                                                    .truncate()
                                                    .text_color(rgb(theme::TEXT_MUTED))
                                                    .text_size(px(theme::FONT_DETAIL))
                                                    .child(title),
                                            ),
                                    ),
                            )
                            .child(match &self.subtitle {
                                WorkspaceSubtitle::Default => div()
                                    .w_full()
                                    .min_w_0()
                                    .truncate()
                                    .text_center()
                                    .text_color(rgb(theme::TEXT_SECONDARY))
                                    .text_size(px(theme::FONT_BODY))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(default_subtitle)
                                    .into_any_element(),
                                WorkspaceSubtitle::Terminal { id } => {
                                    let meta = self.terminal_state.read(cx).meta(id);
                                    let subtitle = if let Some(title) = meta.title.as_deref() {
                                        let stripped = strip_ps1_prefix(title);
                                        if stripped.is_empty() {
                                            default_subtitle.clone()
                                        } else {
                                            stripped.to_owned()
                                        }
                                    } else {
                                        default_subtitle.clone()
                                    };
                                    let agent_icon_path = agent_icon(
                                        meta.title.as_deref(),
                                        meta.current_command.as_deref(),
                                    );

                                    div()
                                        .w_full()
                                        .min_w_0()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .justify_center()
                                        .gap(px(6.0))
                                        .when_some(agent_icon_path, |d, path| {
                                            d.child(
                                                svg()
                                                    .path(path)
                                                    .size(px(14.0))
                                                    .flex_shrink_0()
                                                    .text_color(rgb(theme::TEXT_SECONDARY)),
                                            )
                                        })
                                        .child(
                                            div()
                                                .min_w_0()
                                                .truncate()
                                                .text_color(rgb(theme::TEXT_SECONDARY))
                                                .text_size(px(theme::FONT_BODY))
                                                .font_weight(FontWeight::MEDIUM)
                                                .child(subtitle),
                                        )
                                        .into_any_element()
                                }
                                WorkspaceSubtitle::GitDiff {
                                    filename,
                                    added,
                                    removed,
                                } => div()
                                    .w_full()
                                    .min_w_0()
                                    .px_2()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .justify_center()
                                    .gap(px(6.0))
                                    .text_size(px(theme::FONT_BODY))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(
                                        div()
                                            .min_w_0()
                                            .flex_shrink()
                                            .truncate()
                                            .text_center()
                                            .text_color(rgb(theme::TEXT_SECONDARY))
                                            .child(filename.clone()),
                                    )
                                    .when(*added > 0, |this| {
                                        this.child(
                                            div()
                                                .flex_shrink_0()
                                                .text_color(rgb(0x6fc17a))
                                                .child(format!("+{}", added)),
                                        )
                                    })
                                    .when(*removed > 0, |this| {
                                        this.child(
                                            div()
                                                .flex_shrink_0()
                                                .text_color(rgb(0xd57a7a))
                                                .child(format!("-{}", removed)),
                                        )
                                    })
                                    .into_any_element(),
                            }),
                    )
                    .child(
                        div()
                            .id("quick-action-btn")
                            .w(px(theme::HEADER_BUTTON_SIZE))
                            .h(px(theme::HEADER_BUTTON_SIZE))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hit_slop(px(20.0))
                            .on_press(cx.listener(|_this, _event, window, cx| {
                                platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
                                window.dispatch_action(
                                    workspace_action::OpenQuickAction.boxed_clone(),
                                    cx,
                                );
                            }))
                            .child(
                                svg()
                                    .path("icons/package.svg")
                                    .size(px(16.0))
                                    .text_color(rgb(theme::TEXT_SECONDARY)),
                            ),
                    ),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(mainview_measure)
                    .when_else(
                        self.show_connecting,
                        |d: Div| d.child(self.connecting_view.clone()),
                        |d: Div| d.child(self.main_view.clone()),
                    ),
            )
    }
}
