use gpui::prelude::FluentBuilder;
use gpui::*;

use zedra_session::Session;
use zedra_session::SessionHandle;
use zedra_session::SessionState;

use crate::docs_tree::DocsTree;
use crate::file_explorer::FileExplorer;
use crate::git_panel::GitPanel;
use crate::platform_bridge;
use crate::platform_bridge::HapticFeedback;
use crate::session_panel::SessionPanel;
use crate::telemetry::view_telemetry::{self, ViewDescriptor};
use crate::terminal_panel::TerminalPanel;
use crate::terminal_state::TerminalState;
use crate::theme;
use crate::transport_badge::ConnectionStatusIndicator;
use crate::transport_badge::transport_badge;
use crate::workspace_action;
use crate::workspace_state::{WorkspaceState, WorkspaceStateEvent};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DrawerTab {
    FileExplorer,
    GitDiff,
    Terminals,
    Session,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FileDisplayMode {
    Explorer,
    DocsTree,
}

fn drawer_view_descriptor(tab: DrawerTab, file_display_mode: FileDisplayMode) -> ViewDescriptor {
    match tab {
        DrawerTab::FileExplorer => match file_display_mode {
            FileDisplayMode::Explorer => view_telemetry::DRAWER_FILES,
            FileDisplayMode::DocsTree => view_telemetry::DRAWER_DOCUMENTS,
        },
        DrawerTab::GitDiff => view_telemetry::DRAWER_GIT_DIFF,
        DrawerTab::Terminals => view_telemetry::DRAWER_TERMINALS,
        DrawerTab::Session => view_telemetry::DRAWER_SESSION,
    }
}

pub struct WorkspaceDrawer {
    current_tab: DrawerTab,
    file_display_mode: FileDisplayMode,
    focus_handle: FocusHandle,
    file_explorer: Entity<FileExplorer>,
    docs_tree: Entity<DocsTree>,
    git_panel: Entity<GitPanel>,
    terminal_panel: Entity<TerminalPanel>,
    session_panel: Entity<SessionPanel>,
    #[allow(dead_code)]
    workspace_state: Entity<WorkspaceState>,
    #[allow(dead_code)]
    session_state: Entity<SessionState>,
    #[allow(dead_code)]
    session_handle: SessionHandle,
    _subscriptions: Vec<Subscription>,
}

impl WorkspaceDrawer {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        workspace_state: Entity<WorkspaceState>,
        terminal_state: Entity<TerminalState>,
        session_state: Entity<SessionState>,
        session: Session,
        session_handle: SessionHandle,
    ) -> Self {
        let workspace_state_subscription = cx.subscribe(
            &workspace_state,
            |_drawer, _workspace_state, _event: &WorkspaceStateEvent, cx| cx.notify(),
        );

        let file_explorer = cx.new(|cx| {
            FileExplorer::new(
                workspace_state.clone(),
                session_state.clone(),
                session.clone(),
                session_handle.clone(),
                cx,
            )
        });
        let docs_tree =
            cx.new(|cx| DocsTree::new(workspace_state.clone(), session_handle.clone(), cx));
        let git_panel = cx.new(|cx| {
            GitPanel::new(
                workspace_state.clone(),
                session_state.clone(),
                session.clone(),
                window.window_handle(),
                session_handle.clone(),
                cx,
            )
        });
        let terminal_panel =
            cx.new(|cx| TerminalPanel::new(workspace_state.clone(), terminal_state, cx));
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
            file_display_mode: FileDisplayMode::Explorer,
            focus_handle: cx.focus_handle(),
            file_explorer,
            docs_tree,
            git_panel,
            terminal_panel,
            session_panel,
            workspace_state,
            session_state,
            session_handle,
            _subscriptions: vec![workspace_state_subscription],
        }
    }

    pub fn set_current_tab(&mut self, tab: DrawerTab, cx: &mut Context<Self>) {
        if self.current_tab == tab {
            return;
        }
        self.current_tab = tab;
        self.record_current_view();
        cx.notify();
    }

    fn set_file_display_mode(&mut self, mode: FileDisplayMode, cx: &mut Context<Self>) {
        if self.file_display_mode == mode {
            if mode == FileDisplayMode::DocsTree {
                self.docs_tree.update(cx, |docs_tree, cx| {
                    docs_tree.ensure_built(cx);
                });
            }
            return;
        }
        self.file_display_mode = mode;
        if mode == FileDisplayMode::DocsTree {
            self.docs_tree.update(cx, |docs_tree, cx| {
                docs_tree.ensure_built(cx);
            });
        }
        self.record_current_view();
        cx.notify();
    }

    pub fn refresh_after_sync(&mut self, cx: &mut Context<Self>) -> Task<()> {
        let file_explorer = self
            .file_explorer
            .update(cx, |file_explorer, cx| file_explorer.refresh_after_sync(cx));
        self.docs_tree
            .update(cx, |docs_tree, cx| docs_tree.refresh_after_sync(cx));
        let git_panel = self
            .git_panel
            .update(cx, |git_panel, cx| git_panel.refresh_after_sync(cx));

        cx.spawn(async move |_this, _cx| {
            futures::future::join(file_explorer, git_panel).await;
        })
    }

    pub fn title_by_tab(&self, tab: DrawerTab, cx: &mut Context<Self>) -> (String, String) {
        let workspace_state = self.workspace_state.read(cx);
        let title = workspace_state.project_name.to_string();

        let subtitle = match tab {
            DrawerTab::FileExplorer => match self.file_display_mode {
                FileDisplayMode::Explorer => workspace_state.strip_path.to_string(),
                FileDisplayMode::DocsTree => "documents".to_string(),
            },
            DrawerTab::GitDiff => self.git_panel.read(cx).branch().to_string(),
            DrawerTab::Terminals => "terminals".to_string(),
            DrawerTab::Session => {
                let session_state = self.session_state.read(cx);
                let phase = session_state.phase();
                let transport = session_state.snapshot().transport;
                let (label, _) = transport_badge(&phase, transport.as_ref());
                label
            }
        };

        (title, subtitle)
    }

    pub fn current_view_descriptor(&self) -> ViewDescriptor {
        drawer_view_descriptor(self.current_tab, self.file_display_mode)
    }

    pub fn record_current_view(&self) {
        view_telemetry::record(self.current_view_descriptor());
    }

    pub fn tab_icon(&self, tab: DrawerTab) -> &'static str {
        match tab {
            DrawerTab::FileExplorer => "icons/folder.svg",
            DrawerTab::GitDiff => "icons/git-branch.svg",
            DrawerTab::Terminals => "icons/terminal.svg",
            DrawerTab::Session => "icons/server.svg",
        }
    }

    fn tab_id(&self, tab: DrawerTab) -> &'static str {
        match tab {
            DrawerTab::FileExplorer => "drawer-tab-file-explorer",
            DrawerTab::GitDiff => "drawer-tab-git-diff",
            DrawerTab::Terminals => "drawer-tab-terminals",
            DrawerTab::Session => "drawer-tab-session",
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
            .id(self.tab_id(tab))
            .w(px(36.0))
            .h(px(36.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .cursor_pointer()
            .hit_slop(px(10.0))
            .on_press(cx.listener(move |this, _event, _window, cx| {
                platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
                this.set_current_tab(tab, cx);
            }))
            .child(
                svg()
                    .path(self.tab_icon(tab))
                    .size(px(theme::ICON_MD))
                    .text_color(color),
            )
    }

    fn file_mode_icon(&self, mode: FileDisplayMode) -> &'static str {
        match mode {
            FileDisplayMode::Explorer => "icons/list-tree.svg",
            FileDisplayMode::DocsTree => "icons/file-text.svg",
        }
    }

    fn file_mode_button(&self, mode: FileDisplayMode, cx: &mut Context<Self>) -> impl IntoElement {
        let is_active = self.file_display_mode == mode;
        let color = if is_active {
            rgb(theme::TEXT_SECONDARY)
        } else {
            rgb(theme::TEXT_MUTED)
        };

        div()
            .id(match mode {
                FileDisplayMode::Explorer => "file-display-mode-explorer",
                FileDisplayMode::DocsTree => "file-display-mode-docs-tree",
            })
            .w(px(32.0))
            .h(px(32.0))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hit_slop(px(8.0))
            .on_pointer_down(|_, _, cx| cx.stop_propagation())
            .on_press(cx.listener(move |this, _event, _window, cx| {
                this.set_file_display_mode(mode, cx);
                cx.stop_propagation();
            }))
            .child(
                svg()
                    .path(self.file_mode_icon(mode))
                    .size(px(theme::ICON_XS))
                    .text_color(color),
            )
    }

    fn render_file_mode_toggle(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("file-display-mode-toggle")
            .absolute()
            .top(px(0.0))
            .right(px(0.0))
            .pb_1()
            .bg(rgb(theme::BG_SURFACE))
            .occlude()
            .on_pointer_down(|_, _, cx| cx.stop_propagation())
            .rounded_bl(px(8.0))
            .border_b_1()
            .border_l_1()
            .border_color(rgb(theme::BORDER_SUBTLE))
            .flex()
            .flex_col()
            .child(self.file_mode_button(FileDisplayMode::Explorer, cx))
            .child(self.file_mode_button(FileDisplayMode::DocsTree, cx))
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
            DrawerTab::FileExplorer => match self.file_display_mode {
                FileDisplayMode::Explorer => self.file_explorer.clone().into_any_element(),
                FileDisplayMode::DocsTree => self.docs_tree.clone().into_any_element(),
            },
            DrawerTab::GitDiff => self.git_panel.clone().into_any_element(),
            DrawerTab::Terminals => self.terminal_panel.clone().into_any_element(),
            DrawerTab::Session => self.session_panel.clone().into_any_element(),
        };

        let (title, subtitle) = self.title_by_tab(self.current_tab, cx);
        let connect_phase = self.workspace_state.read(cx).connect_phase.clone();

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
                            .on_press(cx.listener(|_this, _event, window, cx| {
                                platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
                                window.dispatch_action(workspace_action::GoHome.boxed_clone(), cx);
                            }))
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
                                ConnectionStatusIndicator::from_phase(
                                    "drawer-connect-status",
                                    connect_phase.as_ref(),
                                )
                                .size(6.0),
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
                    .relative()
                    .child(tab_content)
                    .when(self.current_tab == DrawerTab::FileExplorer, |el| {
                        el.child(self.render_file_mode_toggle(cx))
                    }),
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

#[cfg(test)]
mod tests {
    use super::{DrawerTab, FileDisplayMode, drawer_view_descriptor};
    use crate::telemetry::view_telemetry;

    #[test]
    fn drawer_tabs_map_to_logical_view_telemetry() {
        assert_eq!(
            drawer_view_descriptor(DrawerTab::FileExplorer, FileDisplayMode::Explorer),
            view_telemetry::DRAWER_FILES
        );
        assert_eq!(
            drawer_view_descriptor(DrawerTab::FileExplorer, FileDisplayMode::DocsTree),
            view_telemetry::DRAWER_DOCUMENTS
        );
        assert_eq!(
            drawer_view_descriptor(DrawerTab::GitDiff, FileDisplayMode::Explorer),
            view_telemetry::DRAWER_GIT_DIFF
        );
        assert_eq!(
            drawer_view_descriptor(DrawerTab::Terminals, FileDisplayMode::Explorer),
            view_telemetry::DRAWER_TERMINALS
        );
        assert_eq!(
            drawer_view_descriptor(DrawerTab::Session, FileDisplayMode::Explorer),
            view_telemetry::DRAWER_SESSION
        );
    }
}
