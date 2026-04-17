use gpui::*;

use crate::terminal_card::{TerminalCardProps, render_terminal_card};
use crate::workspace_state::WorkspaceState;
use crate::{theme, workspace_action};

pub struct TerminalPanel {
    workspace_state: Entity<WorkspaceState>,
}

impl TerminalPanel {
    pub fn new(workspace_state: Entity<WorkspaceState>, _cx: &mut Context<Self>) -> Self {
        Self { workspace_state }
    }
}

impl Render for TerminalPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.workspace_state.read(cx);

        let is_connected = state
            .connect_phase
            .as_ref()
            .map(|p| p.is_connected())
            .unwrap_or(false);

        if !is_connected {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(theme::TEXT_MUTED))
                .text_size(px(theme::FONT_BODY))
                .child("No active session");
        }

        let active_id = state.active_terminal_id.clone();
        let terminal_ids = state.terminal_ids.clone();

        // Collect terminal metadata while we still hold the borrow
        let terminals: Vec<_> = terminal_ids
            .iter()
            .enumerate()
            .map(|(i, tid)| {
                let is_active = active_id.as_deref() == Some(tid.as_str());
                let meta = state
                    .remote_terminal(tid)
                    .map(|t| t.meta().clone())
                    .unwrap_or_default();
                (i, tid.clone(), is_active, meta)
            })
            .collect();

        // Data already cloned above; borrow ends here naturally.

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

        // "New Terminal" button
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
