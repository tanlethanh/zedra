use gpui::*;

use crate::{fonts, sheet_demo_state::SheetDemoState, theme};

pub struct SheetDemoView {
    state: Entity<SheetDemoState>,
}

impl SheetDemoView {
    pub fn new(state: Entity<SheetDemoState>, _cx: &mut Context<Self>) -> Self {
        Self { state }
    }
}

impl Render for SheetDemoView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.read(cx);

        div()
            .id("sheet-demo")
            .size_full()
            .bg(rgb(theme::BG_PRIMARY))
            .flex()
            .justify_center()
            .px(px(18.0))
            .py(px(20.0))
            .child(
                div()
                    .w_full()
                    .max_w(px(680.0))
                    .rounded(px(14.0))
                    .border_1()
                    .border_color(rgb(theme::BORDER_DEFAULT))
                    .bg(rgb(theme::BG_CARD))
                    .px(px(20.0))
                    .py(px(18.0))
                    .flex()
                    .flex_col()
                    .gap(px(theme::SPACING_MD))
                    .child(
                        div()
                            .text_color(rgb(theme::TEXT_PRIMARY))
                            .text_size(px(theme::FONT_BODY))
                            .font_family(fonts::MONO_FONT_FAMILY)
                            .font_weight(FontWeight::MEDIUM)
                            .child(state.title.clone()),
                    )
                    .child(
                        div()
                            .text_color(rgb(theme::TEXT_MUTED))
                            .text_size(px(theme::FONT_DETAIL))
                            .font_family(fonts::MONO_FONT_FAMILY)
                            .child(state.subtitle.clone()),
                    )
                    .child(
                        div()
                            .w_full()
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(rgb(theme::BORDER_SUBTLE))
                            .bg(rgb(theme::BG_SURFACE))
                            .px(px(14.0))
                            .py(px(12.0))
                            .flex()
                            .flex_col()
                            .gap(px(6.0))
                            .child(
                                div()
                                    .text_color(rgb(theme::ACCENT_BLUE))
                                    .text_size(px(theme::FONT_DETAIL))
                                    .font_family(fonts::MONO_FONT_FAMILY)
                                    .child("// Mock presentation entrypoint"),
                            )
                            .children(state.mock_code.iter().cloned().map(|line| {
                                div()
                                    .text_color(rgb(theme::TEXT_SECONDARY))
                                    .text_size(px(theme::FONT_DETAIL))
                                    .font_family(fonts::MONO_FONT_FAMILY)
                                    .child(line)
                            })),
                    )
                    .child(
                        div()
                            .text_color(rgb(theme::TEXT_MUTED))
                            .text_size(px(theme::FONT_DETAIL))
                            .font_family(fonts::MONO_FONT_FAMILY)
                            .child(format!(
                                "Use this view to validate GPUI layout inside native sheet gestures and detents. launches={}",
                                state.launch_count
                            )),
                    ),
            )
    }
}
