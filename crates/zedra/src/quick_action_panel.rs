// QuickActionPanel — right-side overlay for workspace switching.

use gpui::*;

use crate::platform_bridge;
use crate::terminal_card::{TerminalCardProps, render_terminal_card};
use crate::theme;
use crate::workspaces::Workspaces;

#[derive(Clone, Debug)]
pub enum QuickActionEvent {
    Close,
    GoHome,
    NavigateToWorkspace,
}

impl EventEmitter<QuickActionEvent> for QuickActionPanel {}

pub struct QuickActionPanel {
    workspaces: Entity<Workspaces>,
    focus_handle: FocusHandle,
}

impl QuickActionPanel {
    pub fn new(workspaces: Entity<Workspaces>, cx: &mut Context<Self>) -> Self {
        Self {
            workspaces,
            focus_handle: cx.focus_handle(),
        }
    }

    fn handle_scan_qr(&self, cx: &mut Context<Self>) {
        cx.emit(QuickActionEvent::Close);
        platform_bridge::bridge().launch_qr_scanner();
    }

    fn handle_switch_workspace(&self, index: usize, cx: &mut Context<Self>) {
        self.workspaces.update(cx, |ws, cx| ws.switch_to(index, cx));
        cx.emit(QuickActionEvent::Close);
        cx.emit(QuickActionEvent::NavigateToWorkspace);
    }

    fn handle_switch_terminal(&self, ws_index: usize, tid: String, cx: &mut Context<Self>) {
        let view = self
            .workspaces
            .read(cx)
            .get(ws_index)
            .map(|e| e.view.clone());
        self.workspaces
            .update(cx, |ws, cx| ws.switch_to(ws_index, cx));
        if let Some(view) = view {
            view.update(cx, |ws, cx| {
                ws.switch_to_terminal(&tid, cx);
            });
        }
        cx.emit(QuickActionEvent::Close);
        cx.emit(QuickActionEvent::NavigateToWorkspace);
    }

    fn handle_terminal_delete(&self, ws_index: usize, tid: String, cx: &mut Context<Self>) {
        let view = self
            .workspaces
            .read(cx)
            .get(ws_index)
            .map(|e| e.view.clone());
        if let Some(view) = view {
            view.update(cx, |ws, cx| {
                ws.request_terminal_delete(tid, cx);
            });
        }
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

        let states = self.workspaces.read(cx).states().to_vec();
        let handles = self.workspaces.read(cx).handles();

        let workspaces: Vec<_> = states
            .iter()
            .filter(|s| s.workspace_index().is_some())
            .cloned()
            .collect();

        let panel = div()
            .w_full()
            .h(viewport_h)
            .bg(rgb(theme::BG_PRIMARY))
            .border_l_1()
            .border_color(rgb(theme::BORDER_SUBTLE))
            .flex()
            .flex_col()
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
                                    cx.emit(QuickActionEvent::Close);
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

        let mut content = div()
            .id("quick-action-panel-content")
            .flex_1()
            .flex()
            .flex_col()
            .overflow_y_scroll();

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
                (false, false) => format!("{}:{}", ws.hostname(), ws.strip_path()),
                (false, true) => ws.hostname().to_string(),
                (true, false) => ws.strip_path().to_string(),
                (true, true) => String::new(),
            };

            content = content.child(
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
                        cx.listener(move |this, _event, _window, cx| {
                            this.handle_switch_workspace(index, cx);
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
                content = content.child(
                    div()
                        .px(px(16.0))
                        .pb(px(8.0))
                        .text_color(rgb(theme::TEXT_MUTED))
                        .text_size(px(theme::FONT_DETAIL))
                        .child("No terminals"),
                );
            } else {
                content = content.gap_1();
                for (i, tid) in ws.terminal_ids().iter().enumerate() {
                    let tid_click = tid.clone();
                    let tid_del = tid.clone();
                    let is_active = ws.active_terminal_id().is_some_and(|id| id == tid);
                    let meta = handles
                        .get(index)
                        .and_then(|h| h.terminal(tid))
                        .map(|t| t.meta())
                        .unwrap_or_default();

                    let on_close =
                        Box::new(cx.listener(move |this, _event: &ClickEvent, _window, cx| {
                            this.handle_terminal_delete(index, tid_del.clone(), cx);
                        }));

                    let card = render_terminal_card(TerminalCardProps {
                        id: format!("{}-{}", index, tid),
                        index: i + 1,
                        is_active,
                        title: meta.title,
                        cwd: meta.cwd,
                        shell_state: meta.shell_state,
                        last_exit_code: meta.last_exit_code,
                        on_close: Some(on_close),
                    })
                    .on_click(cx.listener(
                        move |this, _event, _window, cx| {
                            this.handle_switch_terminal(index, tid_click.clone(), cx);
                        },
                    ));

                    content = content.child(card);
                }
            }

            content = content.child(
                div()
                    .h(px(1.0))
                    .mx(px(16.0))
                    .mt(px(6.0))
                    .bg(rgb(theme::BORDER_SUBTLE)),
            );
        }

        if workspaces.is_empty() {
            content = content.child(
                div()
                    .px(px(16.0))
                    .py(px(16.0))
                    .text_color(rgb(theme::TEXT_MUTED))
                    .text_size(px(theme::FONT_BODY))
                    .child("No active workspaces"),
            );
        }

        content = content.child(
            crate::button::outline_button("quick-action-scan-qr", "Scan QR Code")
                .mx(px(16.0))
                .mt(px(12.0))
                .on_click(cx.listener(|this, _event, _window, cx| {
                    this.handle_scan_qr(cx);
                })),
        );

        content = content
            .child(div().flex_1())
            .child(div().h(px(bottom_inset)));

        panel.track_focus(&self.focus_handle).child(content)
    }
}
