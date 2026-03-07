use gpui::*;

use crate::theme;
use crate::workspace_store::PersistedWorkspace;
use crate::workspace_view::WorkspaceSummary;

#[derive(Clone, Debug)]
pub enum HomeEvent {
    ScanQrTapped,
    /// Tap on an active (in-memory) workspace card.
    WorkspaceTapped(usize),
    /// Tap on a saved (persisted, not yet connected) workspace card to reconnect.
    SavedWorkspaceTapped(usize),
    /// Long-press / delete a saved workspace. Carries (index, display_name).
    SavedWorkspaceRemoved(usize, String),
}

impl EventEmitter<HomeEvent> for HomeView {}

pub struct HomeView {
    workspaces: Vec<WorkspaceSummary>,
    saved_workspaces: Vec<PersistedWorkspace>,
    focus_handle: FocusHandle,
    /// Set to true when a long press fires; cleared when the next click resolves.
    /// Prevents the reconnect action from also firing after a long press.
    long_press_active: bool,
}

impl HomeView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            workspaces: Vec::new(),
            saved_workspaces: Vec::new(),
            focus_handle: cx.focus_handle(),
            long_press_active: false,
        }
    }

    pub fn update_workspaces(&mut self, summaries: Vec<WorkspaceSummary>, cx: &mut Context<Self>) {
        self.workspaces = summaries;
        cx.notify();
    }

    pub fn update_saved_workspaces(
        &mut self,
        saved: Vec<PersistedWorkspace>,
        cx: &mut Context<Self>,
    ) {
        self.saved_workspaces = saved;
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

        // Unified list: active workspaces not in the saved list appear first,
        // then saved workspaces — replaced by the active card if currently connected.
        let mut cards = div()
            .mt_4()
            .w(px(280.0))
            .flex()
            .flex_col()
            .gap(px(8.0));
        let mut any_cards = false;

        // 1. Active workspaces with no saved match (brand-new unsaved connections)
        for ws in &self.workspaces {
            let matched = ws.endpoint_addr_encoded.as_deref().map_or(false, |addr| {
                self.saved_workspaces.iter().any(|sw| sw.endpoint_addr == addr)
            });
            if !matched {
                let index = ws.index;
                let (status_label, status_color): (&str, u32) = match &ws.session_state {
                    zedra_session::SessionState::Connected { .. } => ("Connected", theme::ACCENT_GREEN),
                    zedra_session::SessionState::Connecting { .. } => ("Connecting\u{2026}", theme::ACCENT_YELLOW),
                    zedra_session::SessionState::Reconnecting { .. } => ("Reconnecting\u{2026}", theme::ACCENT_YELLOW),
                    zedra_session::SessionState::Disconnected => ("Disconnected", theme::ACCENT_RED),
                    zedra_session::SessionState::Error(_) => ("Error", theme::ACCENT_RED),
                };
                let path_label = ws.project_path.as_deref().unwrap_or("Workspace").rsplit('/').next().unwrap_or("Workspace").to_string();
                let term_label = if ws.terminal_count == 1 { "1 terminal".to_string() } else { format!("{} terminals", ws.terminal_count) };
                cards = cards.child(active_workspace_card(index, path_label, status_label, status_color, term_label, cx));
                any_cards = true;
            }
        }

        // 2. Saved workspaces — show active card if connected, reconnect card otherwise
        for (saved_index, sw) in self.saved_workspaces.iter().enumerate() {
            let active = self.workspaces.iter().find(|ws| {
                ws.endpoint_addr_encoded.as_deref() == Some(sw.endpoint_addr.as_str())
            });
            if let Some(ws) = active {
                let index = ws.index;
                let (status_label, status_color): (&str, u32) = match &ws.session_state {
                    zedra_session::SessionState::Connected { .. } => ("Connected", theme::ACCENT_GREEN),
                    zedra_session::SessionState::Connecting { .. } => ("Connecting\u{2026}", theme::ACCENT_YELLOW),
                    zedra_session::SessionState::Reconnecting { .. } => ("Reconnecting\u{2026}", theme::ACCENT_YELLOW),
                    zedra_session::SessionState::Disconnected => ("Disconnected", theme::ACCENT_RED),
                    zedra_session::SessionState::Error(_) => ("Error", theme::ACCENT_RED),
                };
                let path_label = ws.project_path.as_deref().unwrap_or("Workspace").rsplit('/').next().unwrap_or("Workspace").to_string();
                let term_label = if ws.terminal_count == 1 { "1 terminal".to_string() } else { format!("{} terminals", ws.terminal_count) };
                cards = cards.child(active_workspace_card(index, path_label, status_label, status_color, term_label, cx));
            } else {
                let display_name = sw.display_name();
                let label = sw.last_hostname.as_deref().unwrap_or("Saved host").to_string();
                let path_label = sw.project_name().unwrap_or_default().to_string();
                cards = cards.child(
                    div()
                        .id(SharedString::from(format!("ws-saved-card-{}", saved_index)))
                        .w_full()
                        .rounded(px(8.0))
                        .bg(rgb(theme::BG_CARD))
                        .border_1()
                        .border_color(rgb(theme::BORDER_SUBTLE))
                        .p(px(12.0))
                        .cursor_pointer()
                        .hover(|s| s.bg(theme::hover_bg()))
                        .active(|s| s.opacity(0.6))
                        .on_click(cx.listener(move |this, _event, _window, cx| {
                            if this.long_press_active {
                                this.long_press_active = false;
                            } else {
                                cx.emit(HomeEvent::SavedWorkspaceTapped(saved_index));
                            }
                        }))
                        .on_long_press(cx.listener(move |this, _event, _window, cx| {
                            this.long_press_active = true;
                            cx.emit(HomeEvent::SavedWorkspaceRemoved(saved_index, display_name.clone()));
                        }))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(6.0))
                                .child(div().w(px(theme::ICON_STATUS)).h(px(theme::ICON_STATUS)).rounded(px(3.0)).bg(rgb(theme::TEXT_MUTED)))
                                .child(div().flex_1().text_color(rgb(theme::TEXT_PRIMARY)).text_size(px(theme::FONT_BODY)).font_weight(FontWeight::MEDIUM).child(label))
                                .child(div().text_color(rgb(theme::TEXT_MUTED)).text_size(px(theme::FONT_DETAIL)).child("Reconnect")),
                        )
                        .children(if path_label.is_empty() {
                            None
                        } else {
                            Some(div().mt(px(4.0)).text_color(rgb(theme::TEXT_MUTED)).text_size(px(theme::FONT_DETAIL)).child(path_label))
                        }),
                );
            }
            any_cards = true;
        }

        if any_cards {
            content = content.child(cards);
        }

        // Install guide — shown only when there are no workspaces and no saved hosts
        if !has_workspaces && self.saved_workspaces.is_empty() {
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

fn active_workspace_card(
    index: usize,
    path_label: String,
    status_label: &'static str,
    status_color: u32,
    term_label: String,
    cx: &mut Context<HomeView>,
) -> impl IntoElement {
    div()
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
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .child(div().w(px(theme::ICON_STATUS)).h(px(theme::ICON_STATUS)).rounded(px(3.0)).bg(rgb(status_color)))
                .child(div().flex_1().text_color(rgb(theme::TEXT_PRIMARY)).text_size(px(theme::FONT_BODY)).font_weight(FontWeight::MEDIUM).child(path_label))
                .child(div().text_color(rgb(status_color)).text_size(px(theme::FONT_DETAIL)).child(status_label)),
        )
        .child(div().mt(px(4.0)).text_color(rgb(theme::TEXT_MUTED)).text_size(px(theme::FONT_DETAIL)).child(term_label))
}
