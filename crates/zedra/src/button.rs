use gpui::*;

use crate::theme;

/// An outlined button — bordered, centered text, hover highlight.
pub fn outline_button(id: impl Into<ElementId>, label: &str) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .justify_center()
        .py(px(10.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(rgb(theme::BORDER_DEFAULT))
        .cursor_pointer()
        .hover(|s| s.bg(theme::hover_bg()))
        .text_color(rgb(theme::TEXT_PRIMARY))
        .text_size(px(theme::FONT_BODY))
        .font_weight(FontWeight::MEDIUM)
        .child(label.to_string())
}
