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

        content = content.child(
            div()
                .mx(px(theme::DRAWER_PADDING))
                .mb(px(8.0))
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(6.0))
                .child(toolbar_button(
                    "terminal-create-agent-btn",
                    "icons/plus.svg",
                    "Create agent",
                    cx,
                    workspace_action::CreateAgent,
                ))
                .child(toolbar_button(
                    "terminal-create-terminal-btn",
                    "icons/terminal.svg",
                    "Create terminal",
                    cx,
                    workspace_action::CreateNewTerminal,
                ))
                .child(toolbar_button(
                    "terminal-view-sessions-btn",
                    "icons/list-tree.svg",
                    "View sessions",
                    cx,
                    workspace_action::OpenAgentSessions,
                ))
                .child(toolbar_button(
                    "terminal-manage-agents-btn",
                    "icons/settings.svg",
                    "Manage agents",
                    cx,
                    workspace_action::OpenAgentManage,
                )),
        );

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
                let card = render_terminal_card(TerminalCardProps {
                    id: tid,
                    index: index + 1,
                    is_active,
                    title: meta.title,
                    cwd: meta.cwd,
                    agent_icon: meta.agent_icon,
                    shell_state: meta.shell_state,
                    last_exit_code: meta.last_exit_code,
                    on_close: Some(on_close),
                })
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

fn toolbar_button<A: Action>(
    id: &'static str,
    icon: &'static str,
    label: &'static str,
    cx: &mut Context<TerminalPanel>,
    action: A,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex_1()
        .min_w_0()
        .px(px(6.0))
        .py(px(8.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(rgb(theme::BORDER_SUBTLE))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(4.0))
        .cursor_pointer()
        .on_press(cx.listener(move |_this, _event, window, cx| {
            platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
            window.dispatch_action(action.boxed_clone(), cx);
        }))
        .child(
            svg()
                .path(icon)
                .size(px(theme::ICON_SM))
                .text_color(rgb(theme::TEXT_MUTED)),
        )
        .child(
            div()
                .text_size(px(10.0))
                .text_color(rgb(theme::TEXT_MUTED))
                .text_center()
                .child(label),
        )
}
