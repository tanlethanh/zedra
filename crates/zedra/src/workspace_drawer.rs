use gpui::*;

use zedra_session::Session;
use zedra_session::SessionHandle;
use zedra_session::SessionState;

use crate::file_explorer::FileExplorer;
use crate::git_panel::GitPanel;
use crate::platform_bridge;
use crate::platform_bridge::HapticFeedback;
use crate::session_panel::SessionPanel;
use crate::terminal_panel::TerminalPanel;
use crate::theme;
use crate::transport_badge::phase_indicator_color;
use crate::transport_badge::transport_badge;
use crate::workspace_action;
use crate::workspace_state::WorkspaceState;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DrawerTab {
    FileExplorer,
    GitDiff,
    Terminals,
    Session,
}

pub struct WorkspaceDrawer {
    current_tab: DrawerTab,
    focus_handle: FocusHandle,
    file_explorer: Entity<FileExplorer>,
    git_panel: Entity<GitPanel>,
    terminal_panel: Entity<TerminalPanel>,
    session_panel: Entity<SessionPanel>,
    #[allow(dead_code)]
    workspace_state: Entity<WorkspaceState>,
    #[allow(dead_code)]
    session_state: Entity<SessionState>,
    #[allow(dead_code)]
    session_handle: SessionHandle,
}

impl WorkspaceDrawer {
    pub fn new(
        cx: &mut Context<Self>,
        workspace_state: Entity<WorkspaceState>,
        session_state: Entity<SessionState>,
        session: Session,
        session_handle: SessionHandle,
    ) -> Self {
        let file_explorer = cx.new(|cx| {
            FileExplorer::new(
                workspace_state.clone(),
                session_state.clone(),
                session.clone(),
                session_handle.clone(),
                cx,
            )
        });
        let git_panel = cx.new(|cx| {
            GitPanel::new(
                workspace_state.clone(),
                session_state.clone(),
                session.clone(),
                session_handle.clone(),
                cx,
            )
        });
        let terminal_panel = cx.new(|cx| TerminalPanel::new(workspace_state.clone(), cx));
        let session_panel = cx.new(|cx| {
            SessionPanel::new(
                workspace_state.clone(),
                session_state.clone(),
                session_handle.clone(),
                cx,
            )
        });

        Self {
            current_tab: DrawerTab::FileExplorer,
            focus_handle: cx.focus_handle(),
            file_explorer,
            git_panel,
            terminal_panel,
            session_panel,
            workspace_state,
            session_state,
            session_handle,
        }
    }

    pub fn set_current_tab(&mut self, tab: DrawerTab, cx: &mut Context<Self>) {
        self.current_tab = tab;
        cx.notify();
    }

    pub fn title_by_tab(&self, tab: DrawerTab, cx: &mut Context<Self>) -> (String, String) {
        let workspace_state = self.workspace_state.read(cx);
        let session_state = self.session_state.read(cx);
        let title = workspace_state.project_name.to_string();

        let subtitle = match tab {
            DrawerTab::FileExplorer => workspace_state.strip_path.to_string(),
            DrawerTab::GitDiff => "git".to_string(),
            DrawerTab::Terminals => "terminals".to_string(),
            DrawerTab::Session => {
                let phase = session_state.phase();
                let transport = session_state.snapshot().transport;
                let (label, _) = transport_badge(&phase, transport.as_ref());
                label
            }
        };

        (title, subtitle)
    }

    pub fn tab_icon(&self, tab: DrawerTab) -> &'static str {
        match tab {
            DrawerTab::FileExplorer => "icons/folder.svg",
            DrawerTab::GitDiff => "icons/git-branch.svg",
            DrawerTab::Terminals => "icons/terminal.svg",
            DrawerTab::Session => "icons/server.svg",
        }
    }

    fn nav_icon(&self, tab: DrawerTab, cx: &mut Context<Self>) -> impl IntoElement {
        let is_active = self.current_tab == tab;
        let color = if is_active {
            rgb(theme::TEXT_PRIMARY)
        } else {
            rgb(theme::TEXT_MUTED)
        };

        div()
            .w(px(36.0))
            .h(px(36.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .cursor_pointer()
            .hit_slop(px(10.0))
            .hover(|s| s.bg(theme::hover_bg()))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _event, _window, cx| {
                    platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
                    this.set_current_tab(tab, cx);
                }),
            )
            .child(
                svg()
                    .path(self.tab_icon(tab))
                    .size(px(theme::ICON_NAV))
                    .text_color(color),
            )
    }
}

impl Focusable for WorkspaceDrawer {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for WorkspaceDrawer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport_h = window.viewport_size().height;

        let tab_content: AnyElement = match self.current_tab {
            DrawerTab::FileExplorer => self.file_explorer.clone().into_any_element(),
            DrawerTab::GitDiff => self.git_panel.clone().into_any_element(),
            DrawerTab::Terminals => self.terminal_panel.clone().into_any_element(),
            DrawerTab::Session => self.session_panel.clone().into_any_element(),
        };

        let (title, subtitle) = self.title_by_tab(self.current_tab, cx);
        let status_color = phase_indicator_color(&self.session_state.read(cx).phase());

        let top_inset = platform_bridge::status_bar_inset();
        let bottom_inset = platform_bridge::home_indicator_inset().max(10.0);

        div()
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .w_full()
            .h(viewport_h)
            .bg(rgb(theme::BG_PRIMARY))
            .child(div().h(px(top_inset)))
            // Header
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
                            .id("drawer-home-icon")
                            .w(px(theme::DRAWER_ICON_ZONE))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hit_slop(px(10.0))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|_this, _event, window, cx| {
                                    platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
                                    window.dispatch_action(
                                        workspace_action::GoHome.boxed_clone(),
                                        cx,
                                    );
                                }),
                            )
                            .child(
                                svg()
                                    .path("icons/logo.svg")
                                    .size(px(theme::ICON_LOGO))
                                    .text_color(rgb(theme::TEXT_PRIMARY)),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .items_center()
                            .child(
                                div()
                                    .w_full()
                                    .min_w_0()
                                    .truncate()
                                    .text_center()
                                    .text_color(rgb(theme::TEXT_SECONDARY))
                                    .text_size(px(theme::FONT_BODY))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(title),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .min_w_0()
                                    .truncate()
                                    .text_center()
                                    .text_color(rgb(theme::TEXT_MUTED))
                                    .text_size(px(theme::FONT_BODY))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(subtitle),
                            ),
                    )
                    .child(
                        div()
                            .w(px(theme::DRAWER_ICON_ZONE))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .w(px(6.0))
                                    .h(px(6.0))
                                    .rounded(px(3.0))
                                    .bg(rgb(status_color)),
                            ),
                    ),
            )
            // Tab content
            .child(
                div()
                    .id("drawer-tab-content")
                    .flex_1()
                    .w_full()
                    .h_full()
                    .overflow_y_scroll()
                    .child(tab_content),
            )
            // Footer nav bar
            .child(
                div()
                    .flex()
                    .flex_row()
                    .pt(px(10.0))
                    .pb(px(bottom_inset))
                    .justify_center()
                    .gap(px(36.0))
                    .border_t_1()
                    .border_color(rgb(theme::BORDER_SUBTLE))
                    .child(self.nav_icon(DrawerTab::FileExplorer, cx))
                    .child(self.nav_icon(DrawerTab::GitDiff, cx))
                    .child(self.nav_icon(DrawerTab::Terminals, cx))
                    .child(self.nav_icon(DrawerTab::Session, cx)),
            )
    }
}
