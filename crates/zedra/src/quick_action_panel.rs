// QuickActionPanel — absolute right-side overlay for workspace switching.
// Shown when the ⚡ button is tapped in WorkspaceContent header.

use gpui::*;

use crate::platform_bridge;
use crate::terminal_card::{TerminalCardProps, render_terminal_card};
use crate::theme;
use crate::workspace_state::SharedWorkspaceStates;

#[derive(Clone, Debug)]
pub enum QuickActionEvent {
    GoHome,
    SwitchToWorkspace(usize),
    SwitchToTerminal(usize, String),
    Close,
}

impl EventEmitter<QuickActionEvent> for QuickActionPanel {}

pub struct QuickActionPanel {
    states: SharedWorkspaceStates,
    /// Session handles indexed by workspace_index, so terminal metadata can be
    /// read from live `RemoteTerminal`s rather than the static state snapshot.
    handles: Vec<zedra_session::SessionHandle>,
    focus_handle: FocusHandle,
}

impl QuickActionPanel {
    pub fn new(cx: &mut Context<Self>, states: SharedWorkspaceStates) -> Self {
        Self {
            states,
            handles: Vec::new(),
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn set_states(&mut self, states: SharedWorkspaceStates) {
        self.states = states;
    }

    pub fn set_handles(&mut self, handles: Vec<zedra_session::SessionHandle>) {
        self.handles = handles;
    }
}

impl Focusable for QuickActionPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for QuickActionPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let top_inset = platform_bridge::status_bar_inset();
        let bottom_inset = platform_bridge::home_indicator_inset().max(10.0);
        let viewport_h = window.viewport_size().height;

        let workspaces: Vec<_> = self
            .states
            .iter()
            .filter(|s| s.workspace_index().is_some())
            .cloned()
            .collect();

        // Right panel
        let mut panel = div()
            .w_full()
            .h(viewport_h)
            .bg(rgb(theme::BG_PRIMARY))
            .border_l_1()
            .border_color(rgb(theme::BORDER_SUBTLE))
            .flex()
            .flex_col()
            // Status bar spacer
            .child(div().h(px(top_inset)))
            // Panel header
            .child(
                div()
                    // !!!!!!!!!! should be 48px
                    .h(px(44.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(theme::BORDER_SUBTLE))
                    .child(
                        div()
                            .id("quick-action-home-icon")
                            .w(px(36.0))
                            .h(px(36.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hit_slop(px(10.0))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|_this, _event, _window, cx| {
                                    cx.emit(QuickActionEvent::GoHome);
                                }),
                            )
                            .child(
                                svg()
                                    .path("icons/logo.svg")
                                    .size(px(theme::ICON_LOGO))
                                    .text_color(rgb(theme::TEXT_SECONDARY)),
                            ),
                    )
                    // Title
                    .child(
                        div().flex_1().flex().flex_col().child(
                            div()
                                .text_color(rgb(theme::TEXT_SECONDARY))
                                .text_size(px(theme::FONT_BODY))
                                .text_center()
                                .font_weight(FontWeight::MEDIUM)
                                .child("Workspaces"),
                        ),
                    )
                    .child(
                        div()
                            .id("quick-action-close")
                            .w(px(36.0))
                            .h(px(36.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hit_slop(px(10.0))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|_this, _event, _window, cx| {
                                    cx.emit(QuickActionEvent::Close);
                                }),
                            )
                            .child(
                                svg()
                                    .path("icons/x.svg")
                                    .size(px(16.0))
                                    .text_color(rgb(theme::TEXT_SECONDARY)),
                            ),
                    ),
            );

        // Workspace sections
        for ws in &workspaces {
            let index = ws.workspace_index().unwrap_or(0);
            let is_connected = ws
                .connect_phase()
                .map(|p| p.is_connected())
                .unwrap_or(false);
            let status_color = if is_connected {
                theme::ACCENT_GREEN
            } else {
                theme::ACCENT_RED
            };
            let subtitle = match (ws.hostname().is_empty(), ws.strip_path().is_empty()) {
                (false, false) => format!("{}@{}", ws.hostname(), ws.strip_path()),
                (false, true) => ws.hostname().to_string(),
                (true, false) => ws.strip_path().to_string(),
                (true, true) => String::new(),
            };

            // Section header row
            panel = panel.child(
                div()
                    .id(SharedString::from(format!("ws-section-{}", index)))
                    .flex()
                    .flex_col()
                    .px(px(16.0))
                    .pt(px(12.0))
                    .pb(px(6.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::hover_bg()))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |_this, _event, _window, cx| {
                            cx.emit(QuickActionEvent::SwitchToWorkspace(index));
                        }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(6.0))
                            .child(
                                div()
                                    .w(px(theme::ICON_STATUS))
                                    .h(px(theme::ICON_STATUS))
                                    .rounded(px(3.0))
                                    .bg(rgb(status_color)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .text_color(rgb(theme::TEXT_PRIMARY))
                                    .text_size(px(theme::FONT_BODY))
                                    .font_weight(FontWeight::MEDIUM)
                                    .min_w_0()
                                    .truncate()
                                    .child(ws.project_name().to_string()),
                            )
                            .child(
                                div()
                                    .text_color(rgb(theme::TEXT_MUTED))
                                    .text_size(px(theme::FONT_BODY))
                                    .child("\u{2192}"),
                            ),
                    )
                    .child(
                        div()
                            .text_color(rgb(theme::TEXT_MUTED))
                            .text_size(px(theme::FONT_BODY))
                            .min_w_0()
                            .truncate()
                            .child(subtitle),
                    ),
            );

            if ws.terminal_ids().is_empty() {
                // No terminals placeholder
                panel = panel.child(
                    div()
                        .px(px(16.0))
                        .pb(px(8.0))
                        .text_color(rgb(theme::TEXT_MUTED))
                        .text_size(px(theme::FONT_DETAIL))
                        .child("No terminals"),
                );
            } else {
                panel = panel.gap_1();
                for (i, tid) in ws.terminal_ids().iter().enumerate() {
                    let tid_click = tid.clone();
                    let is_active = ws.active_terminal_id().is_some_and(|id| id == tid);
                    let meta = self
                        .handles
                        .get(index)
                        .and_then(|h| h.terminal(tid))
                        .map(|t| t.meta())
                        .unwrap_or_default();

                    let card = render_terminal_card(TerminalCardProps {
                        id: format!("{}-{}", index, tid),
                        index: i + 1,
                        is_active,
                        title: meta.title,
                        cwd: meta.cwd,
                        shell_state: meta.shell_state,
                        last_exit_code: meta.last_exit_code,
                    })
                    .on_click(cx.listener(
                        move |_this, _event, _window, cx| {
                            cx.emit(QuickActionEvent::SwitchToTerminal(index, tid_click.clone()));
                        },
                    ));

                    panel = panel.child(card);
                }
            }

            // Divider between workspace sections
            panel = panel.child(
                div()
                    .h(px(1.0))
                    .mx(px(16.0))
                    .mt(px(6.0))
                    .bg(rgb(theme::BORDER_SUBTLE)),
            );
        }

        if workspaces.is_empty() {
            panel = panel.child(
                div()
                    .px(px(16.0))
                    .py(px(16.0))
                    .text_color(rgb(theme::TEXT_MUTED))
                    .text_size(px(theme::FONT_BODY))
                    .child("No active workspaces"),
            );
        }

        // Spacer + bottom inset
        panel = panel.child(div().flex_1()).child(div().h(px(bottom_inset)));

        panel.track_focus(&self.focus_handle)
    }
}
