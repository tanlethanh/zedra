/// Terminal tab panel for the workspace drawer.
///
/// Lists active terminal sessions with selection and "New Terminal" button.
/// Long-press on a card emits `TerminalDeleteRequested` which the subscriber
/// handles by showing a native confirmation dialog.
use gpui::*;

use crate::terminal_card::{TerminalCardProps, render_terminal_card};
use crate::theme;
use crate::workspace_drawer::WorkspaceDrawerEvent;

/// Render the terminal tab content for the workspace drawer.
pub fn render_terminal_tab(
    handle: Option<&zedra_session::SessionHandle>,
    active_terminal_id: Option<&str>,
    cx: &mut Context<crate::workspace_drawer::WorkspaceDrawer>,
) -> Div {
    let Some(handle) = handle else {
        return div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .text_color(rgb(theme::TEXT_MUTED))
            .text_size(px(theme::FONT_BODY))
            .child("No active session");
    };

    let terminal_ids = handle.terminal_ids();
    let active_id = active_terminal_id.map(|s| s.to_string());

    let mut content = div().pt(px(12.0)).flex().flex_col().flex_1();

    if terminal_ids.is_empty() {
        content = content.child(
            div()
                .py(px(16.0))
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(theme::TEXT_MUTED))
                .text_size(px(theme::FONT_BODY))
                .child("No terminals"),
        );
    } else {
        content = content.gap_1();
        for (index, tid) in terminal_ids.iter().enumerate() {
            let is_active = active_id.as_deref() == Some(tid.as_str());
            let tid_tap = tid.clone();
            let tid_del = tid.clone();

            let card = render_terminal_card(TerminalCardProps {
                id: tid.clone(),
                index: index + 1,
                is_active,
            })
            .on_click(cx.listener(move |_this, _event, _window, cx| {
                cx.emit(WorkspaceDrawerEvent::TerminalSelected(tid_tap.clone()));
            }))
            .on_long_press(cx.listener(move |_this, _event, _window, cx| {
                cx.emit(WorkspaceDrawerEvent::TerminalDeleteRequested(
                    tid_del.clone(),
                ));
            }));

            content = content.child(card);
        }
    }

    // "New Terminal" inline link — no box, dim text, directly below the list
    content = content.child(
        div()
            .id("new-terminal-btn")
            .mx(px(theme::DRAWER_PADDING))
            .mt(px(8.0))
            .px(px(8.0))
            .py(px(8.0))
            .cursor_pointer()
            .on_click(cx.listener(|_this, _event, _window, cx| {
                cx.emit(WorkspaceDrawerEvent::NewTerminalRequested);
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
