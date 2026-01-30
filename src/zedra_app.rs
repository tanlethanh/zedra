/// Shared Zedra App for Android and iOS
///
/// This module contains the core GPUI application that can be used across platforms.

use gpui::*;

/// The main Zedra application view
pub struct ZedraApp {
    counter: usize,
}

impl ZedraApp {
    pub fn new() -> Self {
        Self {
            counter: 0,
        }
    }
}

impl Render for ZedraApp {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .size_full()
            .bg(rgb(0x1e1e1e)) // Dark background
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_4()
                    .p_4()
                    .child(
                        div()
                            .text_color(rgb(0x61afef))
                            .text_xl()
                            .child("Hello Zedra!")
                    )
                    .child(
                        // Large colored circle (using rounded square)
                        div()
                            .size(px(100.0))
                            .bg(rgb(0x61afef)) // Blue
                            .rounded(px(50.0)) // Make it circular
                    )
                    .child(
                        // Horizontal color bar
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                div()
                                    .w(px(50.0))
                                    .h(px(50.0))
                                    .bg(rgb(0xe06c75)) // Red
                                    .rounded(px(8.0))
                            )
                            .child(
                                div()
                                    .w(px(50.0))
                                    .h(px(50.0))
                                    .bg(rgb(0x98c379)) // Green
                                    .rounded(px(8.0))
                            )
                            .child(
                                div()
                                    .w(px(50.0))
                                    .h(px(50.0))
                                    .bg(rgb(0xe5c07b)) // Yellow
                                    .rounded(px(8.0))
                            )
                            .child(
                                div()
                                    .w(px(50.0))
                                    .h(px(50.0))
                                    .bg(rgb(0xc678dd)) // Purple
                                    .rounded(px(8.0))
                            )
                    )
                    .child(
                        // Counter indicator (grow with taps)
                        div()
                            .flex()
                            .gap_1()
                            .child(
                                div()
                                    .w(px(if self.counter > 0 { 30.0 } else { 10.0 }))
                                    .h(px(30.0))
                                    .bg(rgb(0x61afef))
                                    .rounded(px(4.0))
                            )
                            .child(
                                div()
                                    .w(px(if self.counter > 1 { 30.0 } else { 10.0 }))
                                    .h(px(30.0))
                                    .bg(rgb(0x98c379))
                                    .rounded(px(4.0))
                            )
                            .child(
                                div()
                                    .w(px(if self.counter > 2 { 30.0 } else { 10.0 }))
                                    .h(px(30.0))
                                    .bg(rgb(0xe5c07b))
                                    .rounded(px(4.0))
                            )
                            .child(
                                div()
                                    .w(px(if self.counter > 3 { 30.0 } else { 10.0 }))
                                    .h(px(30.0))
                                    .bg(rgb(0xe06c75))
                                    .rounded(px(4.0))
                            )
                            .child(
                                div()
                                    .w(px(if self.counter > 4 { 30.0 } else { 10.0 }))
                                    .h(px(30.0))
                                    .bg(rgb(0xc678dd))
                                    .rounded(px(4.0))
                            )
                    )
                    .child(
                        // Counter text display
                        div()
                            .mt_4()
                            .text_color(rgb(0xe5c07b))
                            .text_lg()
                            .child(format!("Counter: {}", self.counter))
                    )
                    .child(
                        // Info card with border (no text for now)
                        div()
                            .mt_8()
                            .p_6()
                            .w(px(280.0))
                            .bg(rgb(0x282c34))
                            .border_2()
                            .border_color(rgb(0x61afef))
                            .rounded(px(12.0))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_3()
                                    .child(
                                        // Status indicator bars
                                        div()
                                            .w_full()
                                            .h(px(8.0))
                                            .bg(rgb(0x98c379))
                                            .rounded(px(4.0))
                                    )
                                    .child(
                                        div()
                                            .w_full()
                                            .h(px(8.0))
                                            .bg(rgb(0xe5c07b))
                                            .rounded(px(4.0))
                                    )
                                    .child(
                                        div()
                                            .w_full()
                                            .h(px(8.0))
                                            .bg(rgb(0x61afef))
                                            .rounded(px(4.0))
                                    )
                            )
                    )
            )
    }
}

impl Default for ZedraApp {
    fn default() -> Self {
        Self::new()
    }
}
