// QuickActionPanel — right-side overlay for workspace switching.

use std::time::Duration;

use gpui::*;

use crate::pending::{SharedPendingSlot, shared_pending_slot, spawn_periodic_task};
use crate::platform_bridge::{self, AlertButton, HapticFeedback};
use crate::terminal_card::{TerminalCardProps, render_terminal_card};
use crate::terminal_state::TerminalState;
use crate::theme;
use crate::transport_badge::ConnectionStatusIndicator;
use crate::workspaces::Workspaces;

#[derive(Clone, Debug)]
pub enum QuickActionEvent {
    Close,
    GoHome,
    NavigateToWorkspace,
    OpenTerminal { tid: String, ws_index: usize },
    CloseTerminal { tid: String, ws_index: usize },
}

impl EventEmitter<QuickActionEvent> for QuickActionPanel {}

fn is_active_terminal_card(
    active_workspace_index: Option<usize>,
    workspace_index: usize,
    active_terminal_id: Option<&str>,
    terminal_id: &str,
) -> bool {
    active_workspace_index == Some(workspace_index) && active_terminal_id == Some(terminal_id)
}

enum QuickActionPickerPending {
    CreateAgent { ws_index: usize },
    NewTerminal { ws_index: usize },
    ViewSessions { ws_index: usize },
    ManageAgents { ws_index: usize },
}

pub struct QuickActionPanel {
    workspaces: Entity<Workspaces>,
    focus_handle: FocusHandle,
    pending_picker: SharedPendingSlot<QuickActionPickerPending>,
    _pending_picker_task: Task<()>,
}

impl QuickActionPanel {
    pub fn new(workspaces: Entity<Workspaces>, cx: &mut Context<Self>) -> Self {
        let pending_picker = shared_pending_slot();
        let pending_slot = pending_picker.clone();
        let _pending_picker_task =
            spawn_periodic_task(cx, Duration::from_millis(50), move |this, cx| {
                if let Some(action) = pending_slot.take() {
                    this.process_pending_picker_action(action, cx);
                }
            });
        Self {
            workspaces,
            focus_handle: cx.focus_handle(),
            pending_picker,
            _pending_picker_task,
        }
    }

    fn process_pending_picker_action(
        &mut self,
        action: QuickActionPickerPending,
        cx: &mut Context<Self>,
    ) {
        let ws_index = match action {
            QuickActionPickerPending::CreateAgent { ws_index }
            | QuickActionPickerPending::NewTerminal { ws_index }
            | QuickActionPickerPending::ViewSessions { ws_index }
            | QuickActionPickerPending::ManageAgents { ws_index } => ws_index,
        };
        self.workspaces
            .update(cx, |ws, cx| ws.switch_to(ws_index, cx));
        cx.emit(QuickActionEvent::Close);
        cx.emit(QuickActionEvent::NavigateToWorkspace);
        let Some(ws) = self
            .workspaces
            .read(cx)
            .workspace_by_index(ws_index)
            .cloned()
        else {
            return;
        };
        match action {
            QuickActionPickerPending::CreateAgent { .. } => {
                ws.update(cx, |w, cx| w.create_agent_from_quick_action(cx));
            }
            QuickActionPickerPending::NewTerminal { .. } => {
                let ws_weak = ws.downgrade();
                cx.spawn(async move |_this, cx| {
                    let _ = ws_weak.update_in(cx, |w, window, cx| {
                        w.create_terminal_from_quick_action(window, cx);
                    });
                })
                .detach();
            }
            QuickActionPickerPending::ViewSessions { .. } => {
                ws.update(cx, |w, cx| w.open_agent_sessions_from_quick_action(cx));
            }
            QuickActionPickerPending::ManageAgents { .. } => {
                ws.update(cx, |w, cx| w.open_agent_manage_from_quick_action(cx));
            }
        }
    }

    fn handle_show_quick_action_picker(&mut self, ws_index: usize, _cx: &mut Context<Self>) {
        let buttons = vec![
            AlertButton {
                label: "Create Agent".into(),
                style: platform_bridge::AlertButtonStyle::Default,
                image_name: Some("icons/plus.svg".into()),
            },
            AlertButton {
                label: "New Terminal".into(),
                style: platform_bridge::AlertButtonStyle::Default,
                image_name: Some("icons/terminal.svg".into()),
            },
            AlertButton {
                label: "View Sessions".into(),
                style: platform_bridge::AlertButtonStyle::Default,
                image_name: Some("icons/history.svg".into()),
            },
            AlertButton {
                label: "Manage Agents".into(),
                style: platform_bridge::AlertButtonStyle::Default,
                image_name: Some("icons/layers-2.svg".into()),
            },
            AlertButton {
                label: "Cancel".into(),
                style: platform_bridge::AlertButtonStyle::Cancel,
                image_name: None,
            },
        ];
        let pending = self.pending_picker.clone();
        platform_bridge::show_selection("", "", buttons, move |selection| {
            let Some(idx) = selection else { return };
            let action = match idx {
                0 => QuickActionPickerPending::CreateAgent { ws_index },
                1 => QuickActionPickerPending::NewTerminal { ws_index },
                2 => QuickActionPickerPending::ViewSessions { ws_index },
                3 => QuickActionPickerPending::ManageAgents { ws_index },
                _ => return,
            };
            pending.set(action);
        });
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

    fn show_connecting_for_entry(
        &self,
        entry_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspaces.update(cx, |ws, cx| {
            ws.open_connecting_for_entry(entry_index, window, cx);
        });
        cx.emit(QuickActionEvent::Close);
        cx.emit(QuickActionEvent::NavigateToWorkspace);
    }

    fn handle_switch_terminal(&self, ws_index: usize, tid: String, cx: &mut Context<Self>) {
        self.workspaces
            .update(cx, |ws, cx| ws.switch_to(ws_index, cx));
        cx.emit(QuickActionEvent::Close);
        cx.emit(QuickActionEvent::NavigateToWorkspace);
        cx.emit(QuickActionEvent::OpenTerminal { tid, ws_index });
    }

    fn handle_terminal_delete(&self, ws_index: usize, tid: String, cx: &mut Context<Self>) {
        cx.emit(QuickActionEvent::CloseTerminal { tid, ws_index });
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

        let workspaces = self.workspaces.read(cx);
        let ws_count = workspaces.len();
        let active_workspace_index = workspaces.active_index();

        let panel = div()
            .w_full()
            .h(viewport_h)
            .bg(rgb(theme::bg_primary(cx)))
            .border_l_1()
            .border_color(rgb(theme::border_subtle(cx)))
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
                    .border_color(rgb(theme::border_subtle(cx)))
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
                            .on_press(cx.listener(|_this, _event, _window, cx| {
                                platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
                                cx.emit(QuickActionEvent::Close);
                                cx.emit(QuickActionEvent::GoHome);
                            }))
                            .child(
                                svg()
                                    .path("icons/logo.svg")
                                    .size(px(theme::ICON_LOGO))
                                    .text_color(rgb(theme::text_secondary(cx))),
                            ),
                    )
                    .child(
                        div().flex_1().flex().flex_col().child(
                            div()
                                .text_color(rgb(theme::text_secondary(cx)))
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
                            .on_press(cx.listener(|_this, _event, _window, cx| {
                                platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
                                cx.emit(QuickActionEvent::Close);
                            }))
                            .child(
                                svg()
                                    .path("icons/x.svg")
                                    .size(px(16.0))
                                    .text_color(rgb(theme::text_secondary(cx))),
                            ),
                    ),
            );

        let mut content = div()
            .id("quick-action-panel-content")
            .flex_1()
            .flex()
            .flex_col()
            .overflow_y_scroll();

        for index in 0..ws_count {
            let workspace_entity = workspaces.get(index).unwrap().clone();
            let state = workspace_entity.read(cx).workspace_state(cx);
            let terminal_state: Entity<TerminalState> = workspace_entity.read(cx).terminal_state();

            let connect_phase = state.connect_phase.clone();
            let subtitle = match (state.hostname.is_empty(), state.strip_path.is_empty()) {
                (false, false) => format!("{}:{}", state.hostname, state.strip_path),
                (false, true) => state.hostname.to_string(),
                (true, false) => state.strip_path.to_string(),
                (true, true) => String::new(),
            };

            content = content.child(
                div()
                    .id(SharedString::from(format!("ws-section-{}", index)))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(16.0))
                    .pt(px(12.0))
                    .pb(px(6.0))
                    .on_press(cx.listener(move |this, _event, _window, cx| {
                        this.handle_switch_workspace(index, cx);
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap(px(6.0))
                                    .child(
                                        ConnectionStatusIndicator::from_phase(
                                            ("quick-action-connect-status", index),
                                            connect_phase.as_ref(),
                                            &theme::palette(cx),
                                        )
                                        .on_press(
                                            cx.listener(move |this, _event, window, cx| {
                                                this.show_connecting_for_entry(index, window, cx);
                                            }),
                                        ),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_color(rgb(theme::text_primary(cx)))
                                            .text_size(px(theme::FONT_BODY))
                                            .font_weight(FontWeight::MEDIUM)
                                            .min_w_0()
                                            .truncate()
                                            .child(state.display_name().to_string()),
                                    ),
                            )
                            .child(
                                div()
                                    .text_color(rgb(theme::text_muted(cx)))
                                    .text_size(px(theme::FONT_BODY))
                                    .min_w_0()
                                    .truncate()
                                    .child(subtitle),
                            ),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!(
                                "quick-action-add-terminal-{}",
                                index
                            )))
                            .w(px(36.0))
                            .h(px(36.0))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .hit_slop(px(10.0))
                            .on_pointer_down(|_, _, cx| cx.stop_propagation())
                            .on_press(cx.listener(move |this, _event, _window, cx| {
                                platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
                                this.handle_show_quick_action_picker(index, cx);
                                cx.stop_propagation();
                            }))
                            .child(
                                svg()
                                    .path("icons/plus.svg")
                                    .size(px(16.0))
                                    .text_color(rgb(theme::text_muted(cx))),
                            ),
                    ),
            );

            if !state.terminal_ids.is_empty() {
                content = content.gap_1();
                for (i, tid) in state.terminal_ids.iter().enumerate() {
                    let tid_click = tid.clone();
                    let tid_del = tid.clone();
                    let is_active = is_active_terminal_card(
                        active_workspace_index,
                        index,
                        state.active_terminal_id.as_deref(),
                        tid,
                    );
                    let meta = terminal_state.read(cx).meta(tid);

                    let on_close = Box::new(cx.listener(move |this, _event, _window, cx| {
                        this.handle_terminal_delete(index, tid_del.clone(), cx);
                    }));

                    let card = render_terminal_card(
                        cx,
                        TerminalCardProps {
                            id: format!("{}-{}", index, tid),
                            index: i + 1,
                            is_active,
                            title: meta.title,
                            cwd: meta.cwd,
                            agent_icon: meta.agent_icon,
                            agent_state: meta.agent_state,
                            shell_state: meta.shell_state,
                            last_exit_code: meta.last_exit_code,
                            on_close: Some(on_close),
                        },
                    )
                    .on_press(cx.listener(move |this, _event, _window, cx| {
                        this.handle_switch_terminal(index, tid_click.clone(), cx);
                    }));

                    content = content.child(card);
                }
            }

            content = content.child(
                div()
                    .h(px(1.0))
                    .mx(px(16.0))
                    .mt(px(6.0))
                    .bg(rgb(theme::border_subtle(cx))),
            );
        }

        if ws_count == 0 {
            content = content.child(
                div()
                    .px(px(16.0))
                    .py(px(16.0))
                    .text_color(rgb(theme::text_muted(cx)))
                    .text_size(px(theme::FONT_BODY))
                    .child("No active workspaces"),
            );
        }

        content = content.child(
            crate::button::outline_button(cx, "quick-action-scan-qr", "Scan QR Code")
                .mx(px(16.0))
                .mt(px(12.0))
                .on_press(cx.listener(|this, _event, _window, cx| {
                    this.handle_scan_qr(cx);
                })),
        );

        content = content
            .child(div().flex_1())
            .child(div().h(px(bottom_inset)));

        panel.track_focus(&self.focus_handle).child(content)
    }
}

#[cfg(test)]
mod tests {
    use super::is_active_terminal_card;

    #[test]
    fn terminal_card_active_requires_active_workspace_and_terminal() {
        assert!(is_active_terminal_card(
            Some(1),
            1,
            Some("term-2"),
            "term-2"
        ));
        assert!(!is_active_terminal_card(
            Some(0),
            1,
            Some("term-2"),
            "term-2"
        ));
        assert!(!is_active_terminal_card(
            Some(1),
            1,
            Some("term-1"),
            "term-2"
        ));
        assert!(!is_active_terminal_card(None, 1, Some("term-2"), "term-2"));
    }
}
