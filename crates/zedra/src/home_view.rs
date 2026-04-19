use std::sync::OnceLock;
use std::time::Duration;

use gpui::*;
use zedra_session::ConnectPhase;
use zedra_telemetry::*;

use crate::fonts;
use crate::pending::{PendingSlot, spawn_periodic_task};
use crate::platform_bridge::{self, AlertButton, HapticFeedback};
use crate::theme;
use crate::transport_badge::phase_indicator_color;
use crate::workspaces::Workspaces;

const WEBSITE_URL: &str = "https://www.zedra.dev";
const GITHUB_URL: &str = "https://github.com/tanlethanh/zedra";
const DISCORD_URL: &str = "https://discord.gg/39MmkSS8sc";
const XCOM_URL: &str = "https://x.com/zedradev";

#[derive(Clone, Debug)]
pub enum HomeEvent {
    /// Navigate to a workspace (app should switch screen).
    NavigateToWorkspace,
}

impl EventEmitter<HomeEvent> for HomeView {}

/// Pending workspace delete confirmed via native alert.
static PENDING_DELETE: PendingSlot<String> = PendingSlot::new();

pub struct HomeView {
    workspaces: Entity<Workspaces>,
    focus_handle: FocusHandle,
    _pending_delete_task: Task<()>,
}

impl HomeView {
    pub fn new(workspaces: Entity<Workspaces>, cx: &mut Context<Self>) -> Self {
        let pending_delete_task = spawn_periodic_task(cx, Duration::from_millis(50), |this, cx| {
            if let Some(endpoint_addr) = PENDING_DELETE.take() {
                this.process_pending_delete(endpoint_addr, cx);
            }
        });

        Self {
            workspaces,
            focus_handle: cx.focus_handle(),
            _pending_delete_task: pending_delete_task,
        }
    }

    fn handle_scan_qr(&self) {
        tracing::info!("Home: Scan QR tapped");
        zedra_telemetry::send(Event::QrScanInitiated);
        platform_bridge::bridge().launch_qr_scanner();
    }

    fn handle_workspace_tap(
        &self,
        state_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let states = self.workspaces.read(cx).states();
        let Some(state) = states.get(state_index) else {
            return;
        };

        if let Some(entry_index) = self
            .workspaces
            .read(cx)
            .entry_index_by_endpoint_addr(&state.read(cx).endpoint_addr, cx)
        {
            zedra_telemetry::send(Event::WorkspaceSelected { source: "active" });
            self.workspaces
                .update(cx, |ws, cx| ws.switch_to(entry_index, cx));
        } else {
            zedra_telemetry::send(Event::WorkspaceSelected { source: "saved" });
            self.workspaces.update(cx, |ws, cx| {
                ws.connect_saved(state_index, window, cx);
            });
        }
        cx.emit(HomeEvent::NavigateToWorkspace);
    }

    fn handle_workspace_remove(&self, item_idx: usize, cx: &mut Context<Self>) {
        let states = self.workspaces.read(cx).states();
        let Some(state) = states.get(item_idx) else {
            return;
        };

        let endpoint_addr = state.read(cx).endpoint_addr.to_string();
        let display = state.read(cx).project_name.to_string();

        platform_bridge::show_alert(
            "",
            &format!("Remove {} workspace?", display),
            vec![
                AlertButton::destructive("Delete"),
                AlertButton::cancel("Cancel"),
            ],
            move |button_index| {
                if button_index == 0 {
                    PENDING_DELETE.set(endpoint_addr.clone());
                }
            },
        );
    }

    fn process_pending_delete(&self, endpoint_addr: String, cx: &mut Context<Self>) {
        self.workspaces.update(cx, |ws, cx| {
            ws.disconnect_by_endpoint_addr(&endpoint_addr, cx);
            ws.remove_saved(&endpoint_addr, cx);
        });
    }
}

impl Focusable for HomeView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for HomeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let states = self.workspaces.read(cx).states().to_vec();

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
                    .child(
                        svg()
                            .path("icons/logo.svg")
                            .size(px(60.0))
                            .text_color(rgb(theme::TEXT_PRIMARY))
                            .mb(px(theme::SPACING_LG)),
                    )
                    .child(
                        div()
                            .text_color(rgb(theme::TEXT_PRIMARY))
                            .text_size(px(theme::FONT_APP_TITLE))
                            .font_family(fonts::HEADING_FONT_FAMILY)
                            .font_weight(FontWeight::EXTRA_BOLD)
                            .child("Zedra"),
                    )
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

        if !states.is_empty() {
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

            for (item_idx, state) in states.iter().enumerate() {
                let state = state.read(cx);

                let status_color = match state.connect_phase.clone() {
                    Some(p) => phase_indicator_color(&p),
                    None => theme::ACCENT_DIM,
                };
                let status_label = match state.connect_phase.clone() {
                    Some(ConnectPhase::Connected) => "Connected",
                    Some(p) if p.is_connecting() => "Connecting\u{2026}",
                    Some(ConnectPhase::Reconnecting { .. }) => "Reconnecting\u{2026}",
                    Some(ConnectPhase::Failed(_)) => "Error",
                    _ => "Reconnect",
                };

                let project_name = if state.project_name.is_empty() {
                    "Workspace".to_string()
                } else {
                    state.project_name.to_string()
                };
                let strip_path = state.strip_path.to_string();
                let hostname = state.hostname.to_string();
                let subtitle = match (hostname.is_empty(), strip_path.is_empty()) {
                    (false, false) => format!("{hostname}:{strip_path}"),
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

        // Install guide
        if states.is_empty() {
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

        content = content.child(
            crate::button::outline_button("home-scan-qr", "Scan QR Code")
                .w(px(theme::HOME_CARD_WIDTH))
                .on_click(cx.listener(|this, _event, _window, _cx| {
                    this.handle_scan_qr();
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
                            .child(app_version_text()),
                    ),
            )
    }
}

fn app_version_text() -> String {
    static APP_VERSION_TEXT: OnceLock<String> = OnceLock::new();
    APP_VERSION_TEXT
        .get_or_init(|| format!("v{}", platform_bridge::app_version_with_build_number()))
        .clone()
}

/// Render a workspace card UI element.
fn workspace_card(
    index: usize,
    project_name: String,
    subtitle: String,
    status_label: &'static str,
    status_color: u32,
    cx: &mut Context<HomeView>,
) -> impl IntoElement {
    div()
        .id(SharedString::from(format!("ws-card-{}", index)))
        .w_full()
        .rounded(px(8.0))
        .bg(rgb(theme::BG_CARD))
        .border_1()
        .border_color(rgb(theme::BORDER_SUBTLE))
        .p(px(12.0))
        .cursor_pointer()
        .on_click(cx.listener(move |this, _event, window, cx| {
            platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
            this.handle_workspace_tap(index, window, cx);
        }))
        .on_long_press(cx.listener(move |this, _event, _window, cx| {
            this.handle_workspace_remove(index, cx);
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
