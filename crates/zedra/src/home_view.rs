use gpui::*;

use crate::theme;
use crate::workspace_view::WorkspaceSummary;

#[derive(Clone, Debug)]
pub enum HomeEvent {
    ScanQrTapped,
    WorkspaceTapped(usize),
}

impl EventEmitter<HomeEvent> for HomeView {}

pub struct HomeView {
    workspaces: Vec<WorkspaceSummary>,
    focus_handle: FocusHandle,
}

impl HomeView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            workspaces: Vec::new(),
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn update_workspaces(&mut self, summaries: Vec<WorkspaceSummary>, cx: &mut Context<Self>) {
        self.workspaces = summaries;
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
        let has_workspaces = !self.workspaces.is_empty();

        let mut content = div()
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
            );

        // Workspace cards (when sessions exist)
        if has_workspaces {
            let mut cards = div()
                .mt_4()
                .w(px(280.0))
                .flex()
                .flex_col()
                .gap(px(8.0));

            for ws in &self.workspaces {
                let index = ws.index;
                let status_color = if ws.is_connected {
                    theme::ACCENT_GREEN
                } else {
                    theme::ACCENT_RED
                };
                let status_label = if ws.is_connected {
                    "Connected"
                } else {
                    "Disconnected"
                };
                let path_label = ws
                    .project_path
                    .as_deref()
                    .unwrap_or("Workspace")
                    .rsplit('/')
                    .next()
                    .unwrap_or("Workspace")
                    .to_string();
                let term_label = if ws.terminal_count == 0 {
                    "No terminals".to_string()
                } else if ws.terminal_count == 1 {
                    "1 terminal".to_string()
                } else {
                    format!("{} terminals", ws.terminal_count)
                };

                let card = div()
                    .id(SharedString::from(format!("ws-home-card-{}", index)))
                    .w_full()
                    .rounded(px(8.0))
                    .bg(rgb(theme::BG_CARD))
                    .border_1()
                    .border_color(rgb(theme::BORDER_SUBTLE))
                    .p(px(12.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::hover_bg()))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |_this, _event, _window, cx| {
                            cx.emit(HomeEvent::WorkspaceTapped(index));
                        }),
                    )
                    // Header row: status dot + path name
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(6.0))
                            .child(
                                div()
                                    .w(px(theme::ICON_STATUS))
                                    .h(px(theme::ICON_STATUS))
                                    .rounded(px(3.0))
                                    .bg(rgb(status_color)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .text_color(rgb(theme::TEXT_PRIMARY))
                                    .text_size(px(theme::FONT_BODY))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(path_label),
                            )
                            .child(
                                div()
                                    .text_color(rgb(status_color))
                                    .text_size(px(theme::FONT_DETAIL))
                                    .child(status_label),
                            ),
                    )
                    // Terminal count
                    .child(
                        div()
                            .mt(px(4.0))
                            .text_color(rgb(theme::TEXT_MUTED))
                            .text_size(px(theme::FONT_DETAIL))
                            .child(term_label),
                    );

                cards = cards.child(card);
            }

            content = content.child(cards);
        }

        // Install guide — shown only when no workspaces (above the connect button)
        if !has_workspaces {
            content = content.child(
                div()
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
            );
        }

        // "Scan QR Code" button (always shown)
        content = content.child(
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
                        cx.emit(HomeEvent::ScanQrTapped);
                    }),
                )
                .child("Scan QR Code"),
        );

        // Footer
        content = content.child(
            div()
                .mt_8()
                .text_color(rgb(theme::TEXT_MUTED))
                .text_size(px(theme::FONT_DETAIL))
                .child("zedra v0.1.0"),
        );

        div()
            .id("home-starting")
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(rgb(theme::BG_PRIMARY))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .child(content)
    }
}
