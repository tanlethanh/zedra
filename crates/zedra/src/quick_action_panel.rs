// QuickActionPanel — absolute right-side overlay for workspace switching.
// Shown when the ⚡ button is tapped in WorkspaceContent header.

use gpui::*;

use crate::terminal_card::{TerminalCardProps, render_terminal_card};
use crate::theme;
use crate::workspace_view::WorkspaceSummary;

#[derive(Clone, Debug)]
pub enum QuickActionEvent {
    GoHome,
    SwitchToWorkspace(usize),
    SwitchToTerminal(usize, String),
    Close,
}

impl EventEmitter<QuickActionEvent> for QuickActionPanel {}

pub struct QuickActionPanel {
    workspaces: Vec<WorkspaceSummary>,
    focus_handle: FocusHandle,
}

impl QuickActionPanel {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            workspaces: Vec::new(),
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn update_workspaces(&mut self, summaries: Vec<WorkspaceSummary>, cx: &mut Context<Self>) {
        self.workspaces = summaries;
        cx.notify();
    }
}

impl Focusable for QuickActionPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for QuickActionPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let top_inset = crate::platform_bridge::status_bar_inset();
        let bottom_inset = crate::platform_bridge::home_indicator_inset().max(10.0);

        // Panel width: ~75% of viewport
        let panel_width = viewport.width * 0.75;

        let workspaces = self.workspaces.clone();

        // Right panel
        let mut panel = div()
            .absolute()
            .top_0()
            .bottom_0()
            .right_0()
            .w(panel_width)
            .max_w(px(400.0))
            .bg(rgb(theme::BG_PRIMARY))
            .border_l_1()
            .border_color(rgb(theme::BORDER_SUBTLE))
            .flex()
            .flex_col()
            .occlude()
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
            let index = ws.index;
            let status_color = if ws.is_connected {
                theme::ACCENT_GREEN
            } else {
                theme::ACCENT_RED
            };
            let path_label = ws
                .project_path
                .as_deref()
                .unwrap_or("Workspace")
                .rsplit('/')
                .next()
                .unwrap_or("Workspace")
                .to_string();

            // Section header row: status dot + project path
            panel = panel.child(
                div()
                    .id(SharedString::from(format!("ws-section-{}", index)))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
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
                            .child(path_label),
                    )
                    .child(
                        div()
                            .text_color(rgb(theme::TEXT_MUTED))
                            .text_size(px(theme::FONT_BODY))
                            .child("\u{2192}"),
                    ),
            );

            if ws.terminal_ids.is_empty() {
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
                for (i, tid) in ws.terminal_ids.iter().enumerate() {
                    let tid_click = tid.clone();
                    let is_active = ws.active_terminal_id.as_deref() == Some(tid.as_str());

                    let card = render_terminal_card(TerminalCardProps {
                        id: format!("{}-{}", index, tid),
                        index: i + 1,
                        is_active,
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
