use std::sync::OnceLock;
use std::time::Duration;

use gpui::*;
use zedra_session::ConnectPhase;
use zedra_telemetry::*;

use crate::button::outline_button;
use crate::fonts;
use crate::pending::{PendingSlot, spawn_periodic_task};
use crate::platform_bridge::{self, AlertButton, HapticFeedback};
use crate::theme;
use crate::transport_badge::{ConnectionStatusIndicator, phase_indicator_color};
use crate::workspaces::Workspaces;

const WEBSITE_URL: &str = "https://www.zedra.dev";
const GITHUB_URL: &str = "https://github.com/tanlethanh/zedra";
const DISCORD_URL: &str = "https://discord.gg/39MmkSS8sc";
const XCOM_URL: &str = "https://x.com/zedradev";

#[derive(Clone, Debug)]
pub enum HomeEvent {
    /// Navigate to a workspace (app should switch screen).
    NavigateToWorkspace,
    NavigateToSettings,
}

impl EventEmitter<HomeEvent> for HomeView {}

/// Pending workspace delete confirmed via native alert.
static PENDING_DELETE: PendingSlot<String> = PendingSlot::new();

pub struct HomeView {
    workspaces: Entity<Workspaces>,
    focus_handle: FocusHandle,
    selected_guide_tab: GuideTab,
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
            selected_guide_tab: GuideTab::Curl,
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
            ws.remove_by_endpoint_addr(&endpoint_addr, cx);
        });
    }

    fn select_guide_tab(&mut self, tab: GuideTab, cx: &mut Context<Self>) {
        if self.selected_guide_tab == tab {
            return;
        }

        self.selected_guide_tab = tab;
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
        let states = self.workspaces.read(cx).states().to_vec();

        let header = div()
            .id("home-header")
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
                            .id("home-website-link")
                            .underline()
                            .cursor_pointer()
                            .hit_slop(px(10.0))
                            .on_press(cx.listener(|_this, _event, _window, _cx| {
                                platform_bridge::bridge().open_url(WEBSITE_URL);
                            }))
                            .child("zedra.dev"),
                    ),
            )
            .mb(px(theme::SPACING_LG));

        #[cfg(debug_assertions)]
        let settings_button = div()
            .id("home-settings-button")
            .absolute()
            .top(px(platform_bridge::status_bar_inset() + 12.0))
            .right(px(12.0))
            .cursor_pointer()
            .gap(px(6.0))
            .hit_slop(px(10.0))
            .on_press(cx.listener(|_this, _event, _window, cx| {
                platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
                cx.emit(HomeEvent::NavigateToSettings);
            }))
            .child(
                svg()
                    .path("icons/settings.svg")
                    .size(px(theme::ICON_LG))
                    .text_color(rgb(theme::TEXT_MUTED)),
            );

        let mut content = div()
            .id("home-main-content")
            .flex()
            .flex_col()
            .items_center()
            .min_h(px(240.0))
            .gap(px(theme::SPACING_LG));

        if !states.is_empty() {
            let mut cards_container = div()
                .id("home-cards")
                .mt_4()
                .w(px(theme::HOME_CARD_WIDTH))
                .min_h(px(120.0))
                .max_h(px(320.0))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap(px(8.0));

            for (item_idx, state) in states.iter().enumerate() {
                let state = state.read(cx);

                let connect_phase = state.connect_phase.clone();
                let status_color = match connect_phase.as_ref() {
                    Some(p) => phase_indicator_color(p),
                    None => theme::ACCENT_DIM,
                };
                let status_label = match connect_phase.as_ref() {
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
                    connect_phase,
                    status_label,
                    status_color,
                    cx,
                );
                cards_container = cards_container.child(card);
            }

            content = content.child(cards_container);
        }

        // Install guide
        if states.is_empty() {
            content = content.child(install_guide(self.selected_guide_tab, cx));
        }

        content = content.child(
            outline_button("home-scan-qr", "Scan QR Code")
                .w(px(theme::HOME_CARD_WIDTH))
                .on_press(cx.listener(|this, _event, _window, _cx| {
                    this.handle_scan_qr();
                })),
        );

        let bottom_inset = platform_bridge::home_indicator_inset();

        let footer = div()
            .id("home-footer")
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
                    .child(social_button(
                        "btn-xcom",
                        "icons/xcom.svg",
                        26.0,
                        XCOM_URL,
                        cx,
                    ))
                    .child(social_button(
                        "btn-github",
                        "icons/github.svg",
                        32.0,
                        GITHUB_URL,
                        cx,
                    ))
                    .child(social_button(
                        "btn-discord",
                        "icons/discord.svg",
                        36.0,
                        DISCORD_URL,
                        cx,
                    )),
            )
            .child(
                div()
                    .text_color(rgb(theme::TEXT_MUTED))
                    .text_size(px(theme::FONT_DETAIL))
                    .child(app_version_text()),
            );

        let root = div()
            .id("home-view")
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(rgb(theme::BG_PRIMARY))
            .flex()
            .flex_col()
            .items_center()
            .justify_center();

        #[cfg(debug_assertions)]
        let root = root.child(settings_button);

        root.child(header).child(content).child(footer)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GuideTab {
    Curl,
    Claude,
    Codex,
    OpenCode,
    Gemini,
}

struct GuideTabSpec {
    tab: GuideTab,
    label: &'static str,
    icon: &'static str,
    icon_size: f32,
}

struct GuideBlock {
    lines: &'static [GuideLine],
}

struct GuideLine {
    text: &'static str,
    comment: bool,
}

static GUIDE_TABS: &[GuideTabSpec] = &[
    GuideTabSpec {
        tab: GuideTab::Curl,
        label: "curl",
        icon: "icons/terminal.svg",
        icon_size: 17.0,
    },
    GuideTabSpec {
        tab: GuideTab::Claude,
        label: "claude",
        icon: "icons/claude.svg",
        icon_size: 16.5,
    },
    GuideTabSpec {
        tab: GuideTab::Codex,
        label: "codex",
        icon: "icons/openai.svg",
        icon_size: 16.0,
    },
    GuideTabSpec {
        tab: GuideTab::OpenCode,
        label: "opencode",
        icon: "icons/opencode.svg",
        icon_size: 14.5,
    },
    GuideTabSpec {
        tab: GuideTab::Gemini,
        label: "gemini",
        icon: "icons/gemini.svg",
        icon_size: 19.0,
    },
];

static CURL_INSTALL_LINES: &[GuideLine] = &[
    GuideLine {
        text: "# Install Zedra CLI",
        comment: true,
    },
    GuideLine {
        text: "curl -fsSL zedra.dev/install.sh | sh",
        comment: false,
    },
];

static CURL_RUN_LINES: &[GuideLine] = &[
    GuideLine {
        text: "# Start Zedra in the working directory",
        comment: true,
    },
    GuideLine {
        text: "zedra start",
        comment: false,
    },
];

static CLAUDE_INSTALL_LINES: &[GuideLine] = &[
    GuideLine {
        text: "# Set up Zedra for Claude Code",
        comment: true,
    },
    GuideLine {
        text: "zedra setup claude",
        comment: false,
    },
];

static CLAUDE_RUN_LINES: &[GuideLine] = &[
    GuideLine {
        text: "# In Claude, reload plugins and start",
        comment: true,
    },
    GuideLine {
        text: "/reload-plugins",
        comment: false,
    },
    GuideLine {
        text: "/zedra-start",
        comment: false,
    },
];

static CODEX_INSTALL_LINES: &[GuideLine] = &[
    GuideLine {
        text: "# Set up Zedra for Codex",
        comment: true,
    },
    GuideLine {
        text: "zedra setup codex",
        comment: false,
    },
];

static CODEX_RUN_LINES: &[GuideLine] = &[
    GuideLine {
        text: "# In Codex, reload skills and start",
        comment: true,
    },
    GuideLine {
        text: "$zedra-start",
        comment: false,
    },
];

static OPENCODE_INSTALL_LINES: &[GuideLine] = &[
    GuideLine {
        text: "# Set up Zedra for OpenCode",
        comment: true,
    },
    GuideLine {
        text: "zedra setup opencode",
        comment: false,
    },
];

static OPENCODE_RUN_LINES: &[GuideLine] = &[
    GuideLine {
        text: "# In OpenCode, reload skills if needed",
        comment: true,
    },
    GuideLine {
        text: "/zedra-start",
        comment: false,
    },
];

static GEMINI_INSTALL_LINES: &[GuideLine] = &[
    GuideLine {
        text: "# Set up Zedra for Gemini",
        comment: true,
    },
    GuideLine {
        text: "zedra setup gemini",
        comment: false,
    },
];

static GEMINI_RUN_LINES: &[GuideLine] = &[
    GuideLine {
        text: "# In Gemini, reload skills and start",
        comment: true,
    },
    GuideLine {
        text: "/zedra-start",
        comment: false,
    },
];

static CURL_BLOCKS: &[GuideBlock] = &[
    GuideBlock {
        lines: CURL_INSTALL_LINES,
    },
    GuideBlock {
        lines: CURL_RUN_LINES,
    },
];

static CLAUDE_BLOCKS: &[GuideBlock] = &[
    GuideBlock {
        lines: CLAUDE_INSTALL_LINES,
    },
    GuideBlock {
        lines: CLAUDE_RUN_LINES,
    },
];

static CODEX_BLOCKS: &[GuideBlock] = &[
    GuideBlock {
        lines: CODEX_INSTALL_LINES,
    },
    GuideBlock {
        lines: CODEX_RUN_LINES,
    },
];

static OPENCODE_BLOCKS: &[GuideBlock] = &[
    GuideBlock {
        lines: OPENCODE_INSTALL_LINES,
    },
    GuideBlock {
        lines: OPENCODE_RUN_LINES,
    },
];

static GEMINI_BLOCKS: &[GuideBlock] = &[
    GuideBlock {
        lines: GEMINI_INSTALL_LINES,
    },
    GuideBlock {
        lines: GEMINI_RUN_LINES,
    },
];

fn guide_blocks(tab: GuideTab) -> &'static [GuideBlock] {
    match tab {
        GuideTab::Curl => CURL_BLOCKS,
        GuideTab::Claude => CLAUDE_BLOCKS,
        GuideTab::Codex => CODEX_BLOCKS,
        GuideTab::OpenCode => OPENCODE_BLOCKS,
        GuideTab::Gemini => GEMINI_BLOCKS,
    }
}

fn install_guide(selected_tab: GuideTab, cx: &mut Context<HomeView>) -> impl IntoElement {
    let blocks = guide_blocks(selected_tab);
    let line_count = blocks.iter().map(|block| block.lines.len()).sum::<usize>();
    let mut selection_order = 0_u64;

    let mut tab_list = div()
        .id("home-guide-tabs")
        .w_full()
        .flex()
        .flex_row()
        .flex_wrap()
        .gap(px(theme::SPACING_XS))
        .mb(px(theme::SPACING_SM));

    for spec in GUIDE_TABS {
        tab_list = tab_list.child(guide_tab_button(spec, selected_tab == spec.tab, cx));
    }

    let mut guide_body = div()
        .id("home-guide-body")
        .min_h(px(106.0))
        .max_h(px(160.0))
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(px(theme::SPACING_SM));

    for (block_ix, block) in blocks.iter().enumerate() {
        let mut block_el = div().flex().flex_col().font_family(fonts::MONO_FONT_FAMILY);

        for (line_ix, line) in block.lines.iter().enumerate() {
            let is_last_line = selection_order as usize + 1 == line_count;
            block_el = block_el.child(guide_line(
                selected_tab,
                block_ix,
                line_ix,
                line,
                selection_order,
                is_last_line,
                cx,
            ));
            selection_order += 1;
        }

        guide_body = guide_body.child(block_el);
    }

    div()
        .w(px(theme::HOME_GUIDE_WIDTH))
        .flex()
        .flex_col()
        .gap(px(theme::SPACING_SM))
        .child(
            div()
                .id("home-guide-panel")
                .w_full()
                .max_h(px(184.0))
                .rounded(px(8.0))
                .bg(rgb(theme::BG_CARD))
                .border_1()
                .border_color(rgb(theme::BORDER_SUBTLE))
                .px(px(theme::SPACING_SM))
                .pb(px(theme::SPACING_SM))
                .child(tab_list)
                .child(selection_area(guide_body)),
        )
}

fn guide_tab_button(
    spec: &'static GuideTabSpec,
    selected: bool,
    cx: &mut Context<HomeView>,
) -> impl IntoElement {
    div()
        .id(SharedString::from(format!("home-guide-tab-{}", spec.label)))
        .w(px(48.0))
        .h(px(34.0))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .border_b_1()
        .border_color(rgb(if selected {
            theme::BORDER_HIGHLIGHT
        } else {
            theme::BG_CARD
        }))
        .hit_slop(px(10.0))
        .on_press(cx.listener(move |this, _event, _window, cx| {
            this.select_guide_tab(spec.tab, cx);
        }))
        .child(
            svg()
                .path(spec.icon)
                .size(px(spec.icon_size))
                .text_color(rgb(if selected {
                    theme::TEXT_SECONDARY
                } else {
                    theme::TEXT_MUTED
                })),
        )
}

fn guide_line(
    tab: GuideTab,
    block_ix: usize,
    line_ix: usize,
    line: &'static GuideLine,
    selection_order: u64,
    is_last_line: bool,
    _cx: &mut Context<HomeView>,
) -> AnyElement {
    let text = StyledText::new(line.text)
        .selectable()
        .selection_order(selection_order)
        .selection_separator_after(if is_last_line { "" } else { "\n" });

    let row = div()
        .id(SharedString::from(format!(
            "home-guide-line-{:?}-{block_ix}-{line_ix}",
            tab
        )))
        .w_full()
        .text_color(rgb(if line.comment {
            theme::TEXT_MUTED
        } else {
            theme::TEXT_SECONDARY
        }))
        .text_size(px(theme::FONT_DETAIL))
        .child(text);

    row.into_any_element()
}

fn app_version_text() -> String {
    static APP_VERSION_TEXT: OnceLock<String> = OnceLock::new();
    APP_VERSION_TEXT
        .get_or_init(|| {
            format!(
                "zedra v{}",
                platform_bridge::app_version_with_build_number()
            )
        })
        .clone()
}

/// Render a workspace card UI element.
fn workspace_card(
    index: usize,
    project_name: String,
    subtitle: String,
    connect_phase: Option<ConnectPhase>,
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
        .on_press(cx.listener(move |this, _event, window, cx| {
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
                .child(ConnectionStatusIndicator::from_phase(
                    ("home-connect-status", index),
                    connect_phase.as_ref(),
                ))
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
    size: f32,
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
        .on_press(cx.listener(move |_this, _event, _window, _cx| {
            platform_bridge::bridge().open_url(url);
        }))
        .child(
            svg()
                .path(icon)
                .size(px(size))
                .text_color(rgb(theme::TEXT_MUTED)),
        )
}
