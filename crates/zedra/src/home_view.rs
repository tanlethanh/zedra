use gpui::*;

use crate::theme;

#[derive(Clone, Debug)]
pub enum HomeEvent {
    ConnectTapped,
    ScanQrTapped,
}

impl EventEmitter<HomeEvent> for HomeView {}

#[derive(Clone, Copy, PartialEq)]
enum HomeState {
    Starting,
    Projects,
}

pub struct HomeView {
    state: HomeState,
    focus_handle: FocusHandle,
}

impl HomeView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            state: HomeState::Starting,
            focus_handle: cx.focus_handle(),
        }
    }

    /// Switch to the Projects state (called after a session is established)
    pub fn show_projects(&mut self, cx: &mut Context<Self>) {
        self.state = HomeState::Projects;
        cx.notify();
    }
}

impl Focusable for HomeView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for HomeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match self.state {
            HomeState::Starting => self.render_starting(cx).into_any_element(),
            HomeState::Projects => self.render_projects(cx).into_any_element(),
        }
    }
}

impl HomeView {
    fn render_starting(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
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

    fn render_projects(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("home-projects")
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(rgb(theme::BG_PRIMARY))
            .flex()
            .flex_col()
            // Header
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .h(px(56.0))
                    .px_4()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(8.0))
                            .child(
                                svg()
                                    .path("icons/logo.svg")
                                    .size(px(theme::ICON_HEADER))
                                    .text_color(rgb(theme::TEXT_PRIMARY)),
                            )
                            .child(
                                div()
                                    .text_color(rgb(theme::TEXT_PRIMARY))
                                    .text_size(px(theme::FONT_BODY))
                                    .font_weight(FontWeight::BOLD)
                                    .child("Zedra"),
                            ),
                    )
                    .child(
                        div()
                            .w(px(28.0))
                            .h(px(28.0))
                            .rounded_full()
                            .bg(rgb(theme::BG_CARD))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(rgb(theme::TEXT_MUTED))
                            .text_size(px(theme::FONT_DETAIL))
                            .child("U"),
                    ),
            )
            // Content
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .p_4()
                    .gap_4()
                    .child(
                        div()
                            .text_color(rgb(theme::TEXT_MUTED))
                            .text_size(px(theme::FONT_BODY))
                            .child("We need money!"),
                    )
                    // Project card placeholders
                    .child(self.project_card("zedra", "Mobile IDE"))
                    .child(self.project_card("relay-worker", "Cloudflare relay"))
                    .child(self.project_card("zedra-host", "Desktop daemon")),
            )
            // Bottom connect button
            .child(
                div()
                    .p_4()
                    .child(
                        div()
                            .w_full()
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
                    ),
            )
    }

    fn project_card(&self, name: &str, description: &str) -> impl IntoElement {
        div()
            .w_full()
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
                    .text_color(rgb(theme::TEXT_PRIMARY))
                    .text_size(px(theme::FONT_BODY))
                    .child(name.to_string()),
            )
            .child(
                div()
                    .text_color(rgb(theme::TEXT_MUTED))
                    .text_size(px(theme::FONT_DETAIL))
                    .child(description.to_string()),
            )
    }
}
