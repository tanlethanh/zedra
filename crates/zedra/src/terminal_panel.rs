/// Terminal tab panel for the workspace drawer.
///
/// Lists active terminal sessions with selection and "New Terminal" button.
/// Long-press on a card emits `TerminalDeleteRequested` which the subscriber
/// handles by showing a native confirmation dialog.
/// Tap-drag on a card emits `TerminalReordered` for drag-to-reorder.
use gpui::*;

use crate::terminal_card::{TerminalCardProps, render_terminal_card};
use crate::theme;
use crate::workspace_drawer::{WorkspaceDrawer, WorkspaceDrawerEvent};

/// Render the terminal tab content for the workspace drawer.
///
/// `terminal_ids` is the client-ordered list of terminal IDs to display;
/// it may differ from the server's order after drag-reorder or reconnect.
pub fn render_terminal_tab(
    handle: Option<&zedra_session::SessionHandle>,
    terminal_ids: &[String],
    active_terminal_id: Option<&str>,
    cx: &mut Context<WorkspaceDrawer>,
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
            let _tid_drop = tid.clone();

            let meta = handle.terminal(tid).map(|t| t.meta()).unwrap_or_default();

            // Label used for the drag ghost preview (owned copy — SharedString is 'static).
            let _ghost_label: SharedString = meta
                .title
                .as_deref()
                .map(|s| SharedString::from(s.to_owned()))
                .unwrap_or_else(|| SharedString::from(format!("Terminal {}", index + 1)));

            let on_close = Box::new(cx.listener(move |_this, _event: &ClickEvent, _window, cx| {
                cx.emit(WorkspaceDrawerEvent::TerminalDeleteRequested(
                    tid_del.clone(),
                ));
            }));

            let card = render_terminal_card(TerminalCardProps {
                id: tid.clone(),
                index: index + 1,
                is_active,
                title: meta.title,
                cwd: meta.cwd,
                shell_state: meta.shell_state,
                last_exit_code: meta.last_exit_code,
                on_close: Some(on_close),
            })
            .on_click(cx.listener(move |_this, _event, _window, cx| {
                cx.emit(WorkspaceDrawerEvent::TerminalSelected(tid_tap.clone()));
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
