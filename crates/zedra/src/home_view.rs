use gpui::*;

use crate::theme;
use crate::workspace_store::PersistedWorkspace;
use crate::workspace_view::WorkspaceSummary;

/// A single entry in the home workspace list. Carries whichever combination of
/// active (in-memory) and saved (persisted) data applies to this workspace.
#[derive(Clone)]
pub struct HomeWorkspaceItem {
    /// Active workspace index into `ZedraApp.workspaces` and its summary, if connected.
    pub active: Option<(usize, WorkspaceSummary)>,
    /// Saved workspace index into the persisted list and its data, if persisted.
    pub saved: Option<(usize, PersistedWorkspace)>,
}

#[derive(Clone, Debug)]
pub enum HomeEvent {
    ScanQrTapped,
    /// Tap on an active workspace card. Carries workspace_index.
    WorkspaceTapped(usize),
    /// Tap on a saved-only workspace card to reconnect. Carries saved_index.
    SavedWorkspaceTapped(usize),
    /// Long-press / delete. Carries the item index into HomeView::items.
    WorkspaceRemoved(usize),
}

impl EventEmitter<HomeEvent> for HomeView {}

pub struct HomeView {
    pub items: Vec<HomeWorkspaceItem>,
    focus_handle: FocusHandle,
}

impl HomeView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            items: Vec::new(),
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn update_items(&mut self, items: Vec<HomeWorkspaceItem>, cx: &mut Context<Self>) {
        self.items = items;
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

        if !self.items.is_empty() {
            let mut cards = div().mt_4().w(px(280.0)).flex().flex_col().gap(px(8.0));

            for (item_idx, item) in self.items.iter().enumerate() {
                let card = match (&item.active, &item.saved) {
                    (Some((ws_idx, summary)), _) => {
                        let index = *ws_idx;
                        let (status_label, status_color): (&str, u32) = match &summary.session_state
                        {
                            zedra_session::SessionState::Connected { .. } => {
                                ("Connected", theme::ACCENT_GREEN)
                            }
                            zedra_session::SessionState::Connecting { .. } => {
                                ("Connecting\u{2026}", theme::ACCENT_YELLOW)
                            }
                            zedra_session::SessionState::Reconnecting { .. } => {
                                ("Reconnecting\u{2026}", theme::ACCENT_YELLOW)
                            }
                            zedra_session::SessionState::Disconnected => {
                                ("Disconnected", theme::ACCENT_RED)
                            }
                            zedra_session::SessionState::Error(_) => ("Error", theme::ACCENT_RED),
                        };
                        let path_label = summary
                            .project_path
                            .as_deref()
                            .unwrap_or("Workspace")
                            .rsplit('/')
                            .next()
                            .unwrap_or("Workspace")
                            .to_string();
                        let term_label = if summary.terminal_count == 1 {
                            "1 terminal".to_string()
                        } else {
                            format!("{} terminals", summary.terminal_count)
                        };
                        active_workspace_card(
                            item_idx, index, path_label, status_label, status_color, term_label, cx,
                        )
                        .into_any_element()
                    }
                    (None, Some((_, sw))) => {
                        let label = sw
                            .last_hostname
                            .as_deref()
                            .unwrap_or("Saved host")
                            .to_string();
                        let path_label = sw.project_name().unwrap_or_default().to_string();
                        saved_workspace_card(item_idx, label, path_label, cx).into_any_element()
                    }
                    (None, None) => div().into_any_element(),
                };
                cards = cards.child(card);
            }

            content = content.child(cards);
        }

        // Install guide — shown only when there are no items at all
        if self.items.is_empty() {
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
    item_idx: usize,
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
        .active(|s| s.opacity(0.6))
        .on_click(cx.listener(move |_this, _event, _window, cx| {
            cx.emit(HomeEvent::WorkspaceTapped(index));
        }))
        .on_long_press(cx.listener(move |_this, _event, _window, cx| {
            cx.emit(HomeEvent::WorkspaceRemoved(item_idx));
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
    .child(
        div()
            .mt(px(4.0))
            .text_color(rgb(theme::TEXT_MUTED))
            .text_size(px(theme::FONT_DETAIL))
            .child(term_label),
    )
}

fn saved_workspace_card(
    item_idx: usize,
    label: String,
    path_label: String,
    cx: &mut Context<HomeView>,
) -> impl IntoElement {
    div()
        .id(SharedString::from(format!("ws-saved-card-{}", item_idx)))
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
            if let Some(item) = this.items.get(item_idx) {
                if let Some((saved_index, _)) = &item.saved {
                    cx.emit(HomeEvent::SavedWorkspaceTapped(*saved_index));
                }
            }
        }))
        .on_long_press(cx.listener(move |_this, _event, _window, cx| {
            cx.emit(HomeEvent::WorkspaceRemoved(item_idx));
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
        })
}
