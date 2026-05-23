use gpui::*;

use crate::platform_bridge::{self, HapticFeedback};
use crate::terminal_card::{TerminalCardProps, render_terminal_card};
use crate::terminal_state::TerminalState;
use crate::workspace_state::WorkspaceState;
use crate::{theme, workspace_action};

pub struct TerminalPanel {
    workspace_state: Entity<WorkspaceState>,
    terminal_state: Entity<TerminalState>,
    _subscriptions: Vec<Subscription>,
}

impl TerminalPanel {
    pub fn new(
        workspace_state: Entity<WorkspaceState>,
        terminal_state: Entity<TerminalState>,
        _cx: &mut Context<Self>,
    ) -> Self {
        Self {
            workspace_state,
            terminal_state,
            _subscriptions: vec![],
        }
    }
}

impl Render for TerminalPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.workspace_state.read(cx);
        let active_id = state.active_terminal_id.clone();
        let terminal_ids = state.terminal_ids.clone();

        let terminals: Vec<_> = terminal_ids
            .iter()
            .enumerate()
            .map(|(i, tid)| {
                let is_active = active_id.as_deref() == Some(tid.as_str());
                let meta = self.terminal_state.read(cx).meta(tid);
                (i, tid.clone(), is_active, meta)
            })
            .collect();

        let mut content = div().pt(px(12.0)).flex().flex_col().flex_1();

        if !terminals.is_empty() {
            content = content.gap_1();
            for (index, tid, is_active, meta) in terminals {
                let tid_del = tid.clone();
                let on_close = Box::new(cx.listener(move |_this, _event, window, cx| {
                    window.dispatch_action(
                        workspace_action::CloseTerminal {
                            id: tid_del.clone(),
                        }
                        .boxed_clone(),
                        cx,
                    );
                }));

                let tid_tap = tid.clone();
                let card = render_terminal_card(
                    cx,
                    TerminalCardProps {
                        id: tid,
                        index: index + 1,
                        is_active,
                        title: meta.title,
                        cwd: meta.cwd,
                        agent_icon: meta.agent_icon,
                        shell_state: meta.shell_state,
                        last_exit_code: meta.last_exit_code,
                        on_close: Some(on_close),
                    },
                )
                .on_press(cx.listener(move |_this, _event, window, cx| {
                    window.dispatch_action(
                        workspace_action::OpenTerminal {
                            id: tid_tap.clone(),
                        }
                        .boxed_clone(),
                        cx,
                    );
                }));

                content = content.child(card);
            }
        }

        content
    }
}

pub fn toolbar_button<V: 'static, A: Action>(
    id: &'static str,
    icon: &'static str,
    cx: &mut Context<V>,
    action: A,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex_1()
        .min_w_0()
        .px(px(6.0))
        .py(px(8.0))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .on_press(cx.listener(move |_this, _event, window, cx| {
            platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
            window.dispatch_action(action.boxed_clone(), cx);
        }))
        .child(
            svg()
                .path(icon)
                .size(px(theme::ICON_SM))
                .text_color(rgb(theme::text_muted(cx))),
        )
}

pub fn divider<V: 'static>(cx: &mut Context<V>) -> Div {
    div()
        .w(px(1.0))
        .self_stretch()
        .bg(rgb(theme::border_subtle(cx)))
}
