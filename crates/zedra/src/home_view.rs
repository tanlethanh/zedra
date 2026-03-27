use gpui::*;

use crate::fonts;
use crate::platform_bridge;
use crate::theme;
use crate::workspace_state::SharedWorkspaceStates;

const WEBSITE_URL: &str = "https://www.zedra.dev";
const GITHUB_URL: &str = "https://github.com/tanlethanh/zedra";
const DISCORD_URL: &str = "https://discord.gg/39MmkSS8sc";
const XCOM_URL: &str = "https://x.com/zedradev";

#[derive(Clone, Debug)]
pub enum HomeEvent {
    ScanQrTapped,
    /// Tap on a workspace card. Carries item index into HomeView::states.
    WorkspaceTapped(usize),
    /// Long-press / delete. Carries the item index into HomeView::states.
    WorkspaceRemoved(usize),
}

impl EventEmitter<HomeEvent> for HomeView {}

pub struct HomeView {
    /// Shared list; read in render (no lock).
    pub states: SharedWorkspaceStates,
    focus_handle: FocusHandle,
}

impl HomeView {
    pub fn new(cx: &mut Context<Self>, states: SharedWorkspaceStates) -> Self {
        Self {
            states,
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn set_states(&mut self, states: SharedWorkspaceStates) {
        self.states = states;
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
            .gap(px(theme::SPACING_LG))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    // Logo
                    .child(
                        svg()
                            .path("icons/logo.svg")
                            .size(px(60.0))
                            .text_color(rgb(theme::TEXT_PRIMARY))
                            .mb(px(theme::SPACING_LG)),
                    )
                    // "Zedra" title
                    .child(
                        div()
                            .text_color(rgb(theme::TEXT_PRIMARY))
                            .text_size(px(theme::FONT_TITLE))
                            .font_family(fonts::HEADING_FONT_FAMILY)
                            .font_weight(FontWeight::EXTRA_BOLD)
                            .child("Zedra"),
                    )
                    // Subtitle
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .text_color(rgb(theme::TEXT_MUTED))
                            .text_size(px(theme::FONT_BODY))
                            .child("Code from anywhere. ")
                            .child(
                                div()
                                    .underline()
                                    .cursor_pointer()
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|_this, _event, _window, _cx| {
                                            platform_bridge::bridge().open_url(WEBSITE_URL);
                                        }),
                                    )
                                    .child("zedra.dev"),
                            ),
                    ),
            );

        if !self.states.is_empty() {
            let mut cards = div()
                .id("home-cards")
                .mt_4()
                .w(px(theme::HOME_CARD_WIDTH))
                .min_h(px(100.0))
                .max_h(px(320.0))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap(px(8.0));

            for (item_idx, state) in self.states.iter().enumerate() {
                let (status_label, status_color) = match state.connect_phase() {
                    Some(zedra_session::ConnectPhase::Connected) => {
                        ("Connected", theme::ACCENT_GREEN)
                    }
                    Some(p) if p.is_connecting() => ("Connecting\u{2026}", theme::ACCENT_YELLOW),
                    Some(zedra_session::ConnectPhase::Reconnecting { .. }) => {
                        ("Reconnecting\u{2026}", theme::ACCENT_YELLOW)
                    }
                    Some(zedra_session::ConnectPhase::Failed(_)) => ("Error", theme::ACCENT_RED),
                    _ => ("Disconnected", theme::ACCENT_RED),
                };
                let (status_label, status_color) = if state.workspace_index().is_some() {
                    (status_label, status_color)
                } else {
                    ("Reconnect", theme::TEXT_MUTED)
                };

                let project_name = if state.project_name().is_empty() {
                    "Workspace".to_string()
                } else {
                    state.project_name().to_string()
                };
                let strip_path = state.strip_path().to_string();
                let hostname = state.hostname().to_string();
                let subtitle = match (hostname.is_empty(), strip_path.is_empty()) {
                    (false, false) => format!("{hostname}@{strip_path}"),
                    (false, true) => hostname,
                    (true, false) => strip_path,
                    (true, true) => String::new(),
                };

                let card = workspace_card(
                    item_idx,
                    project_name,
                    subtitle,
                    status_label,
                    status_color,
                    cx,
                );
                cards = cards.child(card);
            }

            content = content.child(cards);
        }

        // Install guide — shown only when there are no items at all
        if self.states.is_empty() {
            content = content.child(
                div()
                    .w(px(theme::HOME_GUIDE_WIDTH))
                    .min_h(px(100.0))
                    .bg(rgb(theme::ACCENT_BLUE))
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
                            .child("# Install zedra on your desktop"),
                    )
                    .child(
                        div()
                            .text_color(rgb(theme::TEXT_SECONDARY))
                            .text_size(px(theme::FONT_DETAIL))
                            .child("curl -fsSL zedra.dev/install.sh | sh"),
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
                            .child("zedra start"),
                    ),
            );
        }

        // "Scan QR Code" button (always shown)
        content = content.child(
            crate::button::outline_button("home-scan-qr", "Scan QR Code")
                .w(px(theme::HOME_CARD_WIDTH))
                .on_click(cx.listener(|_this, _event, _window, cx| {
                    cx.emit(HomeEvent::ScanQrTapped);
                })),
        );

        let bottom_inset = platform_bridge::home_indicator_inset();

        div()
            .id("home-footer")
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(rgb(theme::BG_PRIMARY))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .child(content)
            .child(
                div()
                    .absolute()
                    .bottom(px(bottom_inset + 20.0))
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(theme::SPACING_MD))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap(px(theme::SPACING_LG))
                            .opacity(0.8)
                            .child(social_button("btn-xcom", "icons/xcom.svg", XCOM_URL, cx))
                            .child(social_button(
                                "btn-github",
                                "icons/github.svg",
                                GITHUB_URL,
                                cx,
                            ))
                            .child(social_button(
                                "btn-discord",
                                "icons/discord.svg",
                                DISCORD_URL,
                                cx,
                            )),
                    )
                    .child(
                        div()
                            .text_color(rgb(theme::TEXT_MUTED))
                            .text_size(px(theme::FONT_DETAIL))
                            .child(concat!("zedra v", env!("CARGO_PKG_VERSION"))),
                    ),
            )
    }
}

fn workspace_card(
    item_idx: usize,
    project_name: String,
    subtitle: String,
    status_label: &'static str,
    status_color: u32,
    cx: &mut Context<HomeView>,
) -> impl IntoElement {
    div()
        .id(SharedString::from(format!("ws-card-{}", item_idx)))
        .w_full()
        .rounded(px(8.0))
        .bg(rgb(theme::BG_CARD))
        .border_1()
        .border_color(rgb(theme::BORDER_SUBTLE))
        .p(px(12.0))
        .cursor_pointer()
        .on_click(cx.listener(move |_this, _event, _window, cx| {
            cx.emit(HomeEvent::WorkspaceTapped(item_idx));
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
                        .child(project_name),
                )
                .child(
                    div()
                        .text_color(rgb(status_color))
                        .text_size(px(theme::FONT_DETAIL))
                        .child(status_label),
                ),
        )
        .children(if subtitle.is_empty() {
            None
        } else {
            Some(
                div()
                    .mt(px(4.0))
                    .text_color(rgb(theme::TEXT_MUTED))
                    .text_size(px(theme::FONT_DETAIL))
                    .text_overflow(TextOverflow::Truncate(SharedString::new("...")))
                    .child(subtitle),
            )
        })
}

fn social_button(
    id: &'static str,
    icon: &'static str,
    url: &'static str,
    cx: &mut Context<HomeView>,
) -> impl IntoElement {
    div()
        .id(id)
        .flex()
        .hit_slop(px(10.0))
        .items_center()
        .justify_center()
        .cursor_pointer()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |_this, _event, _window, _cx| {
                platform_bridge::bridge().open_url(url);
            }),
        )
        .child(
            svg()
                .path(icon)
                .size(px(32.0))
                .text_color(rgb(theme::TEXT_MUTED)),
        )
}
