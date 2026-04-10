use std::time::Duration;

use gpui::*;
use zedra_telemetry::*;

use crate::deeplink::{self, DeeplinkAction};
use crate::fonts;
use crate::home_view::{HomeEvent, HomeView};
use crate::mgpui::{DrawerHost, DrawerSide};
use crate::quick_action_panel::{QuickActionEvent, QuickActionPanel};
use crate::theme;
use crate::workspaces::{Workspaces, WorkspacesEvent};

#[derive(Clone, Copy, PartialEq, Debug)]
enum AppScreen {
    Home,
    Workspace,
}

pub struct ZedraApp {
    screen: AppScreen,
    home_view: Entity<HomeView>,
    workspaces: Entity<Workspaces>,
    qa_drawer: Entity<DrawerHost>,
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
        let qa_drawer = cx.new(|cx| DrawerHost::new(home_view.clone().into(), cx));
        qa_drawer.update(cx, |h, _| {
            h.set_side(DrawerSide::Right);
            h.set_width(px(theme::QA_DRAWER_WIDTH));
            h.set_drawer(quick_action.clone().into());
        });

        let saved_count = workspaces.read(cx).states().len();
        zedra_telemetry::send(Event::AppOpen {
            saved_workspaces: saved_count,
            // TODO: use native app version
            app_version: env!("CARGO_PKG_VERSION"),
            platform: std::env::consts::OS,
            arch: std::env::consts::ARCH,
        });

        let app = Self {
            screen: AppScreen::Home,
            home_view,
            workspaces,
            qa_drawer,
            _subscriptions: subscriptions,
        };

        // Start background tasks (persist timer, state sync)
        app.start_background_tasks(cx);

        app
    }

    fn start_background_tasks(&self, cx: &mut Context<Self>) {
        // Periodic persist (every 5 seconds)
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(5)).await;
                let should_continue = this
                    .update(cx, |this, cx| {
                        if !this.workspaces.read(cx).is_empty() {
                            this.workspaces.read(cx).persist();
                        }
                    })
                    .is_ok();
                if !should_continue {
                    break;
                }
            }
        })
        .detach();

        // Periodic state sync + deeplink check (every 100ms for responsiveness)
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                let should_continue = this
                    .update(cx, |this, cx| {
                        this.tick(cx);
                    })
                    .is_ok();
                if !should_continue {
                    break;
                }
            }
        })
        .detach();
    }

    /// Called periodically to check deeplinks and sync state.
    fn tick(&mut self, cx: &mut Context<Self>) {
        // Check for pending deeplinks
        if let Some(action) = deeplink::take_pending() {
            self.handle_deeplink_deferred(action, cx);
        }

        // TODO: should use direct channel to update state changed immediately
        // Sync session state to workspace state
        self.workspaces.update(cx, |ws, cx| ws.sync_if_needed(cx));
    }

    fn handle_deeplink_deferred(&mut self, action: DeeplinkAction, cx: &mut Context<Self>) {
        match action {
            DeeplinkAction::Connect(ticket) => {
                tracing::info!("Deeplink: connect action (deferred)");
                // Store ticket for processing when window is available
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
                self.qa_drawer.update(cx, |h, cx| h.close(cx));
            }
            QuickActionEvent::GoHome => {
                self.set_screen(AppScreen::Home, cx);
            }
            QuickActionEvent::NavigateToWorkspace => {
                self.set_screen(AppScreen::Workspace, cx);
            }
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
                self.set_screen(AppScreen::Workspace, cx);
            }
            WorkspacesEvent::Disconnected { .. } => {
                zedra_telemetry::send(Event::Disconnect);
                let new_screen = if self.workspaces.read(cx).is_empty() {
                    AppScreen::Home
                } else {
                    AppScreen::Workspace
                };
                self.set_screen(new_screen, cx);
            }
            WorkspacesEvent::StatesChanged => {
                cx.notify();
            }
            WorkspacesEvent::GoHome => {
                self.set_screen(AppScreen::Home, cx);
            }
            WorkspacesEvent::OpenQuickAction => {
                self.qa_drawer.update(cx, |h, cx| h.open(cx));
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
            self.workspaces
                .update(cx, |ws, cx| ws.process_pending_ticket(window, cx));
            // Immediate sync on activation
            self.workspaces.update(cx, |ws, cx| ws.sync_if_needed(cx));
        }
    }

    fn set_screen(&mut self, screen: AppScreen, cx: &mut Context<Self>) {
        if self.screen != screen {
            self.screen = screen;
            let screen_name = match screen {
                AppScreen::Home => "home",
                AppScreen::Workspace => "workspace",
            };
            zedra_telemetry::send(Event::ScreenView {
                screen: screen_name,
            });
            self.update_drawer_content(cx);
            cx.notify();
        }
    }

    fn update_drawer_content(&mut self, cx: &mut Context<Self>) {
        let screen_view: AnyView = match self.screen {
            AppScreen::Home => self.home_view.clone().into(),
            AppScreen::Workspace => self
                .workspaces
                .read(cx)
                .active_view()
                .map(|v| v.into())
                .unwrap_or_else(|| self.home_view.clone().into()),
        };
        self.qa_drawer.update(cx, |h, _| h.set_content(screen_view));
    }
}

impl Render for ZedraApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Process any pending connection ticket (idempotent)
        self.workspaces
            .update(cx, |ws, cx| ws.process_pending_ticket(window, cx));

        div()
            .size_full()
            .font_family(fonts::MONO_FONT_FAMILY)
            .child(self.qa_drawer.clone())
    }
}

/// Open a GPUI window with the correct app view.
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
