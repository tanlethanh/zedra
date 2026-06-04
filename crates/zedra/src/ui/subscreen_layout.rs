use gpui::*;

use crate::theme;

/// Subscreen page shell: full-width header band + scroll body.
///
/// Follows `docs/CONVENTIONS.md` (GPUI flex width): viewport nodes use `w_full()`;
/// nested `flex_col` children use `min_w_0()` + stretch instead of `w_full()` so
/// narrow content (empty session list, error text) does not shrink-wrap the header.
pub fn subscreen_page(
    page_id: &'static str,
    bg: impl Into<gpui::Fill>,
    header: impl IntoElement,
    body: impl IntoElement,
) -> impl IntoElement {
    div()
        .id(page_id)
        .size_full()
        .min_h_0()
        .bg(bg)
        .flex()
        .flex_col()
        .child(subscreen_header_band(header))
        .child(subscreen_scroll_body(body))
}

fn subscreen_header_band(header: impl IntoElement) -> impl IntoElement {
    div()
        .id("subscreen-header-band")
        .w_full()
        .child(subscreen_content_column(header))
}

fn subscreen_scroll_body(body: impl IntoElement) -> impl IntoElement {
    div()
        .id("subscreen-scroll")
        .flex_1()
        .min_h_0()
        .min_w_0()
        .w_full()
        .overflow_y_scroll()
        .child(subscreen_content_column(body).pb(px(30.0)))
}

/// Centered content column under a definite-width parent (`w_full` band/scrollport).
fn subscreen_content_column(content: impl IntoElement) -> Div {
    div()
        .w_full()
        .max_w(px(theme::CONTENT_MAX_WIDTH))
        .mx_auto()
        .min_w_0()
        .child(content)
}

/// Horizontal padding wrapper for subscreen scroll content.
pub fn subscreen_padded_body(body: impl IntoElement) -> impl IntoElement {
    div()
        .w_full()
        .min_w_0()
        .px(px(theme::SUBSCREEN_PADDING_X))
        .pb(px(theme::SPACING_MD))
        .flex()
        .flex_col()
        .gap(px(theme::SPACING_SM))
        .child(body)
}
