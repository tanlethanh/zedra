// QuickActionPanel — absolute right-side overlay for workspace switching.
// Shown when the ⚡ button is tapped in WorkspaceContent header.

use gpui::*;

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

        // Full-screen backdrop (tap outside panel → close)
        let backdrop = div()
            .absolute()
            .inset_0()
            .bg(hsla(0.0, 0.0, 0.0, 0.4))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_this, _event, _window, cx| {
                    cx.emit(QuickActionEvent::Close);
                }),
            );

        // Right panel
        let mut panel = div()
            .absolute()
            .top_0()
            .bottom_0()
            .right_0()
            .w(panel_width)
            .bg(rgb(theme::BG_PRIMARY))
            .border_l_1()
            .border_color(rgb(theme::BORDER_SUBTLE))
            .flex()
            .flex_col()
            .occlude()
            // Status bar spacer
            .child(div().h(px(top_inset)))
            // Panel header: [× close] [Workspaces title] [home icon]
            .child(
                div()
                    .h(px(48.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .px(px(8.0))
                    .border_b_1()
                    .border_color(rgb(theme::BORDER_SUBTLE))
                    // × close button
                    .child(
                        div()
                            .id("quick-action-close")
                            .w(px(36.0))
                            .h(px(36.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(6.0))
                            .cursor_pointer()
                            .hover(|s| s.bg(theme::hover_bg()))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|_this, _event, _window, cx| {
                                    cx.emit(QuickActionEvent::Close);
                                }),
                            )
                            .child(
                                div()
                                    .text_color(rgb(theme::TEXT_PRIMARY))
                                    .text_size(px(theme::FONT_HEADING))
                                    .child("\u{00d7}"),
                            ),
                    )
                    // Title
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .items_center()
                            .px(px(8.0))
                            .child(
                                div()
                                    .text_color(rgb(theme::TEXT_SECONDARY))
                                    .text_size(px(theme::FONT_BODY))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child("Workspaces"),
                            ),
                    )
                    // Home icon (right side of header, no label)
                    .child(
                        div()
                            .id("quick-action-home-icon")
                            .w(px(36.0))
                            .h(px(36.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(6.0))
                            .cursor_pointer()
                            .hover(|s| s.bg(theme::hover_bg()))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|_this, _event, _window, cx| {
                                    cx.emit(QuickActionEvent::GoHome);
                                }),
                            )
                            .child(
                                svg()
                                    .path("icons/logo.svg")
                                    .size(px(theme::ICON_NAV))
                                    .text_color(rgb(theme::TEXT_PRIMARY)),
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
                // Terminal preview boxes
                for (i, tid) in ws.terminal_ids.iter().enumerate() {
                    let tid = tid.clone();
                    let display_name = format!("Terminal {}", i + 1);

                    panel = panel.child(
                        div()
                            .id(SharedString::from(format!("term-{}-{}", index, i)))
                            .mx(px(16.0))
                            .mb(px(6.0))
                            .px(px(12.0))
                            .py(px(8.0))
                            .rounded(px(6.0))
                            .bg(rgb(theme::BG_CARD))
                            .border_1()
                            .border_color(rgb(theme::BORDER_SUBTLE))
                            .cursor_pointer()
                            .hover(|s| s.bg(theme::hover_bg()).border_color(rgb(theme::BORDER_DEFAULT)))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |_this, _event, _window, cx| {
                                    cx.emit(QuickActionEvent::SwitchToTerminal(index, tid.clone()));
                                }),
                            )
                            .child(
                                div()
                                    .font_family(zedra_terminal::TERMINAL_FONT_FAMILY)
                                    .text_color(rgb(theme::TEXT_SECONDARY))
                                    .text_size(px(theme::FONT_DETAIL))
                                    .child(display_name),
                            ),
                    );
                }
            }

            // Divider between workspace sections
            panel = panel.child(
                div()
                    .h(px(1.0))
                    .mx(px(16.0))
                    .mt(px(4.0))
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
        panel = panel
            .child(div().flex_1())
            .child(div().h(px(bottom_inset)));

        div()
            .track_focus(&self.focus_handle)
            .absolute()
            .inset_0()
            .child(backdrop)
            .child(panel)
    }
}
