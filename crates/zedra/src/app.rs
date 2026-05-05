use std::time::Duration;

use gpui::*;
use zedra_telemetry::*;

use crate::deeplink::{self, DeeplinkAction};
use crate::fonts;
use crate::home_view::{HomeEvent, HomeView};
use crate::platform_bridge;
use crate::quick_action_panel::{QuickActionEvent, QuickActionPanel};
use crate::settings_view::{SettingsEvent, SettingsView};
use crate::telemetry::view_telemetry::{self, ViewDescriptor};
use crate::ui::{DrawerHost, DrawerSide};
use crate::workspaces::{Workspaces, WorkspacesEvent};

#[derive(Clone, Copy, PartialEq, Debug)]
enum AppScreen {
    Home,
    Settings,
    Workspace,
}

fn app_view_descriptor(screen: AppScreen) -> Option<ViewDescriptor> {
    match screen {
        AppScreen::Home => Some(view_telemetry::HOME),
        AppScreen::Settings => Some(view_telemetry::SETTINGS),
        AppScreen::Workspace => None,
    }
}

/// Workspace content projects `Workspaces.active_index`, so same-screen
/// workspace navigation still needs to remount the active workspace view.
fn should_update_drawer_content(current: AppScreen, next: AppScreen) -> bool {
    current != next || next == AppScreen::Workspace
}

pub struct ZedraApp {
    screen: AppScreen,
    home_view: Entity<HomeView>,
    settings_view: Entity<SettingsView>,
    workspaces: Entity<Workspaces>,
    quick_action_drawer: Entity<DrawerHost>,
    _subscriptions: Vec<Subscription>,
}

impl ZedraApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        fonts::load_fonts(window);

        let mut subscriptions = Vec::new();

        // --- Workspaces ---
        let workspaces = cx.new(|cx| Workspaces::new(cx));

        // --- Home view ---
        let home_view = cx.new(|cx| HomeView::new(workspaces.clone(), cx));
        let sub = cx.subscribe(&home_view, Self::on_home_event);
        subscriptions.push(sub);

        let settings_view = cx.new(SettingsView::new);
        let sub = cx.subscribe(&settings_view, Self::on_settings_event);
        subscriptions.push(sub);

        // --- Quick action panel ---
        let quick_action = cx.new(|cx| QuickActionPanel::new(workspaces.clone(), cx));
        let sub = cx.subscribe(&quick_action, Self::on_quick_action_event);
        subscriptions.push(sub);

        // --- Workspaces events ---
        let sub = cx.subscribe(&workspaces, Self::on_workspaces_event);
        subscriptions.push(sub);

        // --- Window activation: check deeplinks + sync state ---
        let sub = cx.observe_window_activation(window, Self::on_window_activation);
        subscriptions.push(sub);

        // --- Quick action drawer ---
        let quick_action_drawer = cx.new(|cx| {
            DrawerHost::new(
                home_view.clone().into(),
                quick_action.clone().into(),
                DrawerSide::Right,
                cx,
            )
        });

        let saved_count = workspaces.read(cx).states().len();
        zedra_telemetry::send(Event::AppOpen {
            saved_workspaces: saved_count,
            app_version: platform_bridge::app_version_with_build_number(),
            platform: std::env::consts::OS,
            arch: std::env::consts::ARCH,
        });

        let app = Self {
            screen: AppScreen::Home,
            home_view,
            settings_view,
            workspaces,
            quick_action_drawer,
            _subscriptions: subscriptions,
        };
        app.record_current_view(cx);

        // Start background tasks (deeplink + deferred ticket checks)
        app.start_background_tasks(window, cx);

        app
    }

    fn start_background_tasks(&self, window: &mut Window, cx: &mut Context<Self>) {
        // Periodic deeplink + deferred ticket check (every 100ms for responsiveness)
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                let should_continue = this
                    .update_in(cx, |this, window, cx| {
                        this.tick(window, cx);
                    })
                    .is_ok();
                if !should_continue {
                    break;
                }
            }
        })
        .detach();
    }

    /// Called periodically to check deeplinks and deferred window-bound work.
    fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(action) = deeplink::take_pending() {
            self.handle_deeplink_deferred(action, cx);
        }
        self.process_pending_ticket_if_ready(window, cx);
    }

    fn handle_deeplink_deferred(&mut self, action: DeeplinkAction, cx: &mut Context<Self>) {
        match action {
            DeeplinkAction::Connect(ticket) => {
                tracing::info!("Deeplink: connect action (deferred)");
                // Store ticket for processing when a window-aware tick or activation is available.
                self.workspaces.update(cx, |ws, cx| {
                    ws.connect_ticket_deferred(ticket, cx);
                });
            }
        }
    }

    fn on_home_event(&mut self, _: Entity<HomeView>, event: &HomeEvent, cx: &mut Context<Self>) {
        match event {
            HomeEvent::NavigateToWorkspace => {
                self.set_screen(AppScreen::Workspace, cx);
            }
            HomeEvent::NavigateToSettings => {
                self.set_screen(AppScreen::Settings, cx);
            }
        }
    }

    fn on_settings_event(
        &mut self,
        _: Entity<SettingsView>,
        event: &SettingsEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            SettingsEvent::NavigateHome => {
                self.set_screen(AppScreen::Home, cx);
            }
        }
    }

    fn on_quick_action_event(
        &mut self,
        _: Entity<QuickActionPanel>,
        event: &QuickActionEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            QuickActionEvent::Close => {
                self.quick_action_drawer.update(cx, |h, cx| h.close(cx));
            }
            QuickActionEvent::GoHome => {
                self.set_screen(AppScreen::Home, cx);
            }
            QuickActionEvent::NavigateToWorkspace => {
                self.set_screen(AppScreen::Workspace, cx);
            }
            QuickActionEvent::OpenTerminal { tid, ws_index } => {
                self.set_screen(AppScreen::Workspace, cx);
                self.open_terminal_from_quick_action(*ws_index, tid, cx);
            }
            QuickActionEvent::CloseTerminal { tid, ws_index } => {
                self.close_terminal_from_quick_action(*ws_index, tid, cx);
            }
        }
    }

    fn open_terminal_from_quick_action(
        &self,
        ws_index: usize,
        terminal_id: &str,
        cx: &mut Context<Self>,
    ) {
        if let Some(workspace) = self
            .workspaces
            .read(cx)
            .workspace_by_index(ws_index)
            .cloned()
        {
            workspace.update(cx, |ws, cx| {
                ws.open_terminal_from_quick_action(terminal_id.to_string(), cx);
            });
        } else {
            tracing::warn!(
                "workspace not found for index {} to open terminal {} from quick action",
                ws_index,
                terminal_id
            );
        }
    }

    fn close_terminal_from_quick_action(
        &self,
        ws_index: usize,
        terminal_id: &str,
        cx: &mut Context<Self>,
    ) {
        if let Some(workspace) = self
            .workspaces
            .read(cx)
            .workspace_by_index(ws_index)
            .cloned()
        {
            workspace.update(cx, |ws, cx| {
                ws.close_terminal_from_quick_action(terminal_id.to_string(), cx);
            });
        } else {
            tracing::warn!(
                "workspace not found for index {} to close terminal {} from quick action",
                ws_index,
                terminal_id
            );
        }
    }

    fn on_workspaces_event(
        &mut self,
        _: Entity<Workspaces>,
        event: &WorkspacesEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            WorkspacesEvent::Connected { .. } => {
                let screen_changed = self.screen != AppScreen::Workspace;
                self.set_screen(AppScreen::Workspace, cx);
                if !screen_changed {
                    self.record_current_view(cx);
                }
            }
            WorkspacesEvent::Disconnected { .. } => {
                zedra_telemetry::send(Event::Disconnect);
                // A confirmed manual disconnect should leave the workspace surface immediately;
                // the saved Home card remains the reconnect entry point.
                self.set_screen(screen_after_workspace_disconnect(), cx);
            }
            WorkspacesEvent::StatesChanged => {
                cx.notify();
            }
            WorkspacesEvent::GoHome => {
                self.set_screen(AppScreen::Home, cx);
            }
            WorkspacesEvent::OpenQuickAction => {
                self.quick_action_drawer.update(cx, |h, cx| h.open(cx));
                view_telemetry::record(view_telemetry::QUICK_ACTIONS);
            }
        }
    }

    fn on_window_activation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.is_window_active() {
            tracing::info!(
                "ZedraApp: window activated, {} workspace(s)",
                self.workspaces.read(cx).len()
            );
            // Process any pending ticket (from deeplinks)
            self.process_pending_ticket_if_ready(window, cx);
        }
    }

    fn process_pending_ticket_if_ready(&self, window: &mut Window, cx: &mut Context<Self>) {
        if should_process_pending_ticket(
            Workspaces::has_pending_ticket(),
            window.is_window_active(),
        ) {
            self.workspaces
                .update(cx, |ws, cx| ws.process_pending_ticket(window, cx));
        }
    }

    fn set_screen(&mut self, screen: AppScreen, cx: &mut Context<Self>) {
        let screen_changed = self.screen != screen;
        let should_update_content = should_update_drawer_content(self.screen, screen);
        if screen_changed {
            self.screen = screen;
        }

        if should_update_content {
            self.update_drawer_content(cx);
            cx.notify();
        }

        if screen_changed {
            self.record_current_view(cx);
        }
    }

    fn update_drawer_content(&mut self, cx: &mut Context<Self>) {
        let screen_view: AnyView = match self.screen {
            AppScreen::Home => self.home_view.clone().into(),
            AppScreen::Settings => self.settings_view.clone().into(),
            AppScreen::Workspace => self
                .workspaces
                .read(cx)
                .active_view()
                .map(|v| v.into())
                .unwrap_or_else(|| self.home_view.clone().into()),
        };
        self.quick_action_drawer
            .update(cx, |h, _| h.set_content(screen_view));
    }

    fn record_current_view(&self, cx: &mut Context<Self>) {
        if let Some(screen) = app_view_descriptor(self.screen) {
            view_telemetry::record(screen);
            return;
        }

        let active_workspace = self.workspaces.read(cx).active().cloned();
        if let Some(workspace) = active_workspace {
            workspace.update(cx, |workspace, cx| workspace.record_current_view(cx));
        }
    }

    #[cfg(target_os = "ios")]
    pub(crate) fn close_transports_for_lifecycle(
        &mut self,
        reason: &'static [u8],
        cx: &mut Context<Self>,
    ) {
        self.workspaces
            .update(cx, |ws, cx| ws.close_transports_for_lifecycle(reason, cx));
    }
}

impl Render for ZedraApp {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .font_family(fonts::MONO_FONT_FAMILY)
            .child(self.quick_action_drawer.clone())
    }
}

fn should_process_pending_ticket(has_pending_ticket: bool, window_active: bool) -> bool {
    has_pending_ticket && window_active
}

fn screen_after_workspace_disconnect() -> AppScreen {
    AppScreen::Home
}

pub fn open_zedra_window(app: &mut App, window_options: WindowOptions) -> Result<AnyWindowHandle> {
    app.open_window(window_options, |window, cx| {
        let view = cx.new(|cx| ZedraApp::new(window, cx));
        window.refresh();
        view
    })
    .map(|h| h.into())
}

#[cfg(test)]
mod tests {
    use super::{
        AppScreen, app_view_descriptor, screen_after_workspace_disconnect,
        should_process_pending_ticket, should_update_drawer_content,
    };
    use crate::telemetry::view_telemetry;

    #[test]
    fn pending_ticket_processing_requires_ticket_and_active_window() {
        assert!(should_process_pending_ticket(true, true));
        assert!(!should_process_pending_ticket(false, true));
        assert!(!should_process_pending_ticket(true, false));
        assert!(!should_process_pending_ticket(false, false));
    }

    #[test]
    fn app_screen_mapping_uses_logical_gpui_views() {
        assert_eq!(
            app_view_descriptor(AppScreen::Home),
            Some(view_telemetry::HOME)
        );
        assert_eq!(
            app_view_descriptor(AppScreen::Settings),
            Some(view_telemetry::SETTINGS)
        );
        assert_eq!(app_view_descriptor(AppScreen::Workspace), None);
    }

    #[test]
    fn manual_workspace_disconnect_returns_home() {
        assert_eq!(screen_after_workspace_disconnect(), AppScreen::Home);
    }

    #[test]
    fn workspace_navigation_refreshes_content_even_on_same_screen() {
        assert!(should_update_drawer_content(
            AppScreen::Workspace,
            AppScreen::Workspace
        ));
        assert!(should_update_drawer_content(
            AppScreen::Home,
            AppScreen::Workspace
        ));
        assert!(!should_update_drawer_content(
            AppScreen::Home,
            AppScreen::Home
        ));
    }
}
