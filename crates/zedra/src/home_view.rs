use gpui::*;

use crate::theme;

#[derive(Clone, Debug)]
pub enum HomeEvent {
    ConnectTapped,
    ScanQrTapped,
}

impl EventEmitter<HomeEvent> for HomeView {}

pub struct HomeView {
    focus_handle: FocusHandle,
}

impl HomeView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
        }
    }
}

impl Focusable for HomeView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for HomeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("home-starting")
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(rgb(theme::BG_PRIMARY))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(16.0))
                    // Logo
                    .child(
                        svg()
                            .path("icons/logo.svg")
                            .size(px(48.0))
                            .text_color(rgb(theme::TEXT_PRIMARY)),
                    )
                    // "Zedra" title
                    .child(
                        div()
                            .text_color(rgb(theme::TEXT_PRIMARY))
                            .text_size(px(theme::FONT_TITLE))
                            .font_weight(FontWeight::BOLD)
                            .child("Zedra"),
                    )
                    // Install guide code block
                    .child(
                        div()
                            .mt_4()
                            .w(px(280.0))
                            .rounded(px(8.0))
                            .bg(rgb(theme::BG_CARD))
                            .border_1()
                            .border_color(rgb(theme::BORDER_SUBTLE))
                            .p_4()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_color(rgb(theme::TEXT_MUTED))
                                    .text_size(px(theme::FONT_DETAIL))
                                    .child("# Install zedra-host on your machine"),
                            )
                            .child(
                                div()
                                    .text_color(rgb(theme::TEXT_SECONDARY))
                                    .text_size(px(theme::FONT_DETAIL))
                                    .child("cargo install zedra-host"),
                            )
                            .child(
                                div()
                                    .text_color(rgb(theme::TEXT_MUTED))
                                    .text_size(px(theme::FONT_DETAIL))
                                    .mt_2()
                                    .child("# Start the daemon"),
                            )
                            .child(
                                div()
                                    .text_color(rgb(theme::TEXT_SECONDARY))
                                    .text_size(px(theme::FONT_DETAIL))
                                    .child("zedra-host listen"),
                            ),
                    )
                    // Connect button (outline)
                    .child(
                        div()
                            .mt_4()
                            .w(px(280.0))
                            .py(px(12.0))
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(rgb(theme::BORDER_DEFAULT))
                            .flex()
                            .justify_center()
                            .cursor_pointer()
                            .hover(|s| s.bg(theme::hover_bg()))
                            .text_color(rgb(theme::TEXT_PRIMARY))
                            .text_size(px(theme::FONT_BODY))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|_this, _event, _window, cx| {
                                    cx.emit(HomeEvent::ConnectTapped);
                                }),
                            )
                            .child("Connect"),
                    )
                    // Scan QR button
                    .child(
                        div()
                            .w(px(280.0))
                            .py(px(12.0))
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(rgb(theme::BORDER_DEFAULT))
                            .flex()
                            .justify_center()
                            .cursor_pointer()
                            .hover(|s| s.bg(theme::hover_bg()))
                            .text_color(rgb(theme::TEXT_PRIMARY))
                            .text_size(px(theme::FONT_BODY))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|_this, _event, _window, cx| {
                                    cx.emit(HomeEvent::ScanQrTapped);
                                }),
                            )
                            .child("Scan QR Code"),
                    )
                    // Footer
                    .child(
                        div()
                            .mt_8()
                            .text_color(rgb(theme::TEXT_MUTED))
                            .text_size(px(theme::FONT_DETAIL))
                            .child("zedra v0.1.0"),
                    ),
            )
    }
}
