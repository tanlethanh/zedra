use gpui::*;

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
                let on_close =
                    Box::new(cx.listener(move |_this, _event: &ClickEvent, window, cx| {
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
                    shell_state: meta.shell_state,
                    last_exit_code: meta.last_exit_code,
                    on_close: Some(on_close),
                })
                .on_click(cx.listener(move |_this, _event, window, cx| {
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

        content = content.child(
            div()
                .id("new-terminal-btn")
                .mx(px(theme::DRAWER_PADDING))
                .mt(px(8.0))
                .px(px(8.0))
                .py(px(8.0))
                .cursor_pointer()
                .on_click(cx.listener(|_this, _event, window, cx| {
                    window.dispatch_action(workspace_action::CreateNewTerminal.boxed_clone(), cx);
                }))
                .child(
                    div()
                        .text_color(rgb(theme::TEXT_MUTED))
                        .text_size(px(theme::FONT_BODY))
                        .text_center()
                        .child("+ New Terminal"),
                ),
        );

        content
    }
}
