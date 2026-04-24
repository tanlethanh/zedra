use gpui::*;

use crate::theme;

pub fn render_placeholder(message: impl Into<String>) -> Div {
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .child(
            div()
                // Magic! It's more balance with this
                .top(px(-32.0))
                .text_color(rgb(theme::TEXT_SECONDARY))
                .text_size(px(theme::FONT_BODY))
                .text_align(TextAlign::Center)
                .child(message.into()),
        )
}

pub fn render_empty() -> Div {
    div()
}
