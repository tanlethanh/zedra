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
                let (status_label, status_color): (&str, u32) = match &ws.session_state {
                    zedra_session::SessionState::Connected { .. } => ("Connected", theme::ACCENT_GREEN),
                    zedra_session::SessionState::Connecting { .. } => ("Connecting\u{2026}", theme::ACCENT_YELLOW),
                    zedra_session::SessionState::Reconnecting { .. } => ("Reconnecting\u{2026}", theme::ACCENT_YELLOW),
                    zedra_session::SessionState::Disconnected => ("Disconnected", theme::ACCENT_RED),
                    zedra_session::SessionState::Error(_) => ("Error", theme::ACCENT_RED),
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

        // Saved workspace cards (persisted, not yet connected)
        // Filter out any that are already active (matched by endpoint_addr).
        let active_addrs: Vec<String> = Vec::new(); // Active workspaces don't expose addr yet, show all saved
        let saved: Vec<_> = self
            .saved_workspaces
            .iter()
            .enumerate()
            .filter(|(_, w)| !active_addrs.contains(&w.endpoint_addr))
            .collect();

        if !saved.is_empty() && !has_workspaces {
            // Section header
            content = content.child(
                div()
                    .mt_4()
                    .text_color(rgb(theme::TEXT_MUTED))
                    .text_size(px(theme::FONT_DETAIL))
                    .child("Saved Hosts"),
            );
        }

        for (saved_index, ws) in &saved {
            // Don't show saved cards if there are already active workspace cards
            // (the user is connected; they'll see the active cards above)
            if has_workspaces {
                break;
            }
            let saved_index = *saved_index;
            let display_name = ws.display_name();
            let label = ws
                .last_hostname
                .as_deref()
                .unwrap_or("Saved host")
                .to_string();
            let path_label = ws.project_name().unwrap_or_default().to_string();

            let card = div()
                .id(SharedString::from(format!("ws-saved-card-{}", saved_index)))
                .w(px(280.0))
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
                        .child(
                            div()
                                .w(px(theme::ICON_STATUS))
                                .h(px(theme::ICON_STATUS))
                                .rounded(px(3.0))
                                .bg(rgb(theme::TEXT_MUTED)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .text_color(rgb(theme::TEXT_PRIMARY))
                                .text_size(px(theme::FONT_BODY))
                                .font_weight(FontWeight::MEDIUM)
                                .child(label),
                        )
                        .child(
                            div()
                                .text_color(rgb(theme::TEXT_MUTED))
                                .text_size(px(theme::FONT_DETAIL))
                                .child("Reconnect"),
                        ),
                )
                .children(if path_label.is_empty() {
                    None
                } else {
                    Some(
                        div()
                            .mt(px(4.0))
                            .text_color(rgb(theme::TEXT_MUTED))
                            .text_size(px(theme::FONT_DETAIL))
                            .child(path_label),
                    )
                });

            content = content.child(card);
        }

        // Install guide — shown only when no workspaces (above the connect button)
        if !has_workspaces && saved.is_empty() {
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
