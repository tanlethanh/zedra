/// Terminal tab panel for the workspace drawer.
///
/// Lists active terminal sessions with selection and "New Terminal" button.

use gpui::prelude::FluentBuilder;
use gpui::*;

use crate::workspace_drawer::WorkspaceDrawerEvent;
use crate::theme;

/// Render the terminal tab content for the workspace drawer.
pub fn render_terminal_tab(
    active_terminal_id: Option<&str>,
    cx: &mut Context<crate::workspace_drawer::WorkspaceDrawer>,
) -> Div {
    let session = zedra_session::active_session();

    let Some(session) = session else {
        return div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .text_color(rgb(theme::TEXT_MUTED))
            .text_size(px(theme::FONT_BODY))
            .child("No active session");
    };

    let terminal_ids = session.terminal_ids();
    let active_id = active_terminal_id.map(|s| s.to_string());

    let mut content = div().px(px(16.0)).pt(px(12.0)).flex().flex_col().flex_1();

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
        for (index, tid) in terminal_ids.iter().enumerate() {
            let is_active = active_id.as_deref() == Some(tid.as_str());
            let label = format!("Terminal {}", index + 1);
            let tid_clone = tid.clone();

            let row = div()
                .id(SharedString::from(format!("term-row-{}", index)))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .py(px(8.0))
                .px(px(8.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .hover(|s| s.bg(theme::hover_bg()))
                .when(is_active, |s| s.bg(theme::hover_bg()))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |_this, _event, _window, cx| {
                        cx.emit(WorkspaceDrawerEvent::TerminalSelected(tid_clone.clone()));
                    }),
                )
                .child(
                    svg()
                        .path("icons/terminal.svg")
                        .size(px(theme::ICON_NAV))
                        .text_color(if is_active {
                            rgb(theme::TEXT_PRIMARY)
                        } else {
                            rgb(theme::TEXT_MUTED)
                        }),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(px(theme::FONT_BODY))
                        .text_color(if is_active {
                            rgb(theme::TEXT_PRIMARY)
                        } else {
                            rgb(theme::TEXT_SECONDARY)
                        })
                        .when(is_active, |s| s.font_weight(FontWeight::MEDIUM))
                        .child(label),
                )
                .when(is_active, |s| {
                    s.child(
                        div()
                            .w(px(theme::ICON_STATUS))
                            .h(px(theme::ICON_STATUS))
                            .rounded(px(3.0))
                            .bg(rgb(theme::ACCENT_GREEN)),
                    )
                });

            content = content.child(row);

            // Separator between items
            if index < terminal_ids.len() - 1 {
                content =
                    content.child(div().h(px(1.0)).mx(px(8.0)).bg(rgb(theme::BORDER_SUBTLE)));
            }
        }
    }

    // "New Terminal" inline link — no box, dim text, directly below the list
    content = content.child(
        div()
            .id("new-terminal-btn")
            .px(px(8.0))
            .py(px(8.0))
            .cursor_pointer()
            .hover(|s| s.bg(theme::hover_bg()))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_this, _event, _window, cx| {
                    cx.emit(WorkspaceDrawerEvent::NewTerminalRequested);
                }),
            )
            .child(
                div()
                    .text_color(rgb(theme::TEXT_MUTED))
                    .text_size(px(theme::FONT_BODY))
                    .child("+ New Terminal"),
            ),
    );

    content
}
