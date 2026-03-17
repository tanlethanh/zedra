/// Reusable terminal card UI component.
///
/// Returns a `Stateful<Div>` that the caller can chain event handlers onto:
///
/// ```rust
/// render_terminal_card(props)
///     .on_click(cx.listener(...))
///     .on_long_press(cx.listener(...))
/// ```
///
/// Used in the workspace drawer terminal tab and the quick-action panel.
use gpui::prelude::FluentBuilder;
use gpui::*;

use crate::{fonts, theme};

/// Props that describe how a single terminal card should be rendered.
pub struct TerminalCardProps {
    /// Unique server-assigned terminal ID (used as the GPUI element ID base).
    pub id: String,
    /// 1-based display index: shown as "Terminal N" in the label.
    pub index: usize,
    /// Whether this is the currently active / focused terminal.
    pub is_active: bool,
}

/// Render a terminal card element.
///
/// Returns a `Div` — chain `.on_click()` and `.on_long_press()` for tap and
/// long-press actions respectively.
pub fn render_terminal_card(props: TerminalCardProps) -> Stateful<Div> {
    let label = format!("Terminal {}", props.index);
    let card_id = SharedString::from(format!("term-card-{}", props.id));
    let is_active = props.is_active;

    div()
        .id(card_id)
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .mx(px(theme::DRAWER_PADDING))
        .mb(px(6.0))
        .px(px(12.0))
        .py(px(10.0))
        .rounded(px(6.0))
        .bg(rgb(theme::BG_CARD))
        .border_1()
        .border_color(if is_active {
            rgb(theme::BORDER_DEFAULT)
        } else {
            rgb(theme::BORDER_SUBTLE)
        })
        .cursor_pointer()
        .hover(|s| s.bg(theme::hover_bg()))
        .active(|s| s.opacity(0.75))
        // Terminal icon
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
        // Label
        .child(
            div()
                .flex_1()
                .font_family(fonts::MONO_FONT_FAMILY)
                .text_color(if is_active {
                    rgb(theme::TEXT_PRIMARY)
                } else {
                    rgb(theme::TEXT_SECONDARY)
                })
                .text_size(px(theme::FONT_BODY))
                .when(is_active, |s| s.font_weight(FontWeight::MEDIUM))
                .child(label),
        )
        // Active status dot
        .when(is_active, |s| {
            s.child(
                div()
                    .w(px(theme::ICON_STATUS))
                    .h(px(theme::ICON_STATUS))
                    .rounded(px(3.0))
                    .bg(rgb(theme::ACCENT_GREEN)),
            )
        })
}
