//! Shared agent cards, session list, and display helpers (not navigation-stack views).
use chrono::{DateTime, Utc};
use gpui::prelude::FluentBuilder;
use gpui::*;
use zedra_rpc::proto::{
    AgentSessionSummary, AgentSetupState, AgentSummary, AgentUsageSnapshot, ManagedAgentKind,
};

use crate::fonts;
use crate::platform_bridge::{self, HapticFeedback};
use crate::{theme, workspace_action};

// ---------------------------------------------------------------------------
// Display helpers
// ---------------------------------------------------------------------------

pub fn cli_version_display(agent: &AgentSummary) -> String {
    if agent.cli.available {
        agent
            .cli
            .version
            .clone()
            .unwrap_or_else(|| "Checking…".to_string())
    } else {
        agent
            .cli
            .error
            .clone()
            .unwrap_or_else(|| "Not installed".to_string())
    }
}

pub fn setup_label(state: AgentSetupState) -> &'static str {
    match state {
        AgentSetupState::MissingCli => "Missing CLI",
        AgentSetupState::NotConfigured => "Not configured",
        AgentSetupState::SkillsOnly => "Skills only",
        AgentSetupState::HooksReady => "Hooks ready",
        AgentSetupState::Error => "Error",
    }
}

pub fn managed_agent_icon(kind: ManagedAgentKind) -> &'static str {
    match kind {
        ManagedAgentKind::Claude => "icons/claude.svg",
        ManagedAgentKind::Codex => "icons/openai.svg",
        ManagedAgentKind::OpenCode => "icons/opencode.svg",
    }
}

pub fn kind_slug(kind: ManagedAgentKind) -> &'static str {
    match kind {
        ManagedAgentKind::Claude => "claude",
        ManagedAgentKind::Codex => "codex",
        ManagedAgentKind::OpenCode => "opencode",
    }
}

pub fn managed_agent_name(kind: ManagedAgentKind) -> &'static str {
    match kind {
        ManagedAgentKind::Claude => "Claude",
        ManagedAgentKind::Codex => "Codex",
        ManagedAgentKind::OpenCode => "OpenCode",
    }
}

pub fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

// ---------------------------------------------------------------------------
// Agent card
// ---------------------------------------------------------------------------

pub struct AgentCardProps<'a> {
    pub agent: &'a AgentSummary,
}

pub fn render_agent_card(cx: &App, props: AgentCardProps<'_>) -> Stateful<Div> {
    let agent = props.agent;
    let kind = agent.kind;
    let display_name = agent.display_name.clone();
    let version = cli_version_display(agent);
    let session_count = agent.sessions.resumable.max(agent.sessions.total);
    let sessions_label = format!("{session_count} sessions");
    let plan = agent
        .account
        .fields
        .iter()
        .find(|f| f.label == "Plan")
        .map(|f| f.value.clone());
    let usage = agent.usage.clone();

    div()
        .id(SharedString::from(format!(
            "agent-card-{}",
            kind_slug(kind)
        )))
        .w_full()
        .min_w_0()
        .px(px(theme::SPACING_MD))
        .py(px(theme::SPACING_SM))
        .rounded(px(6.0))
        .border_1()
        .border_color(rgb(theme::border_subtle(cx)))
        .bg(rgb(theme::bg_card_dim(cx)))
        .child(
            div()
                .w_full()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(theme::SPACING_SM))
                        .child(
                            svg()
                                .path(managed_agent_icon(kind))
                                .size(px(theme::ICON_MD))
                                .flex_shrink_0()
                                .text_color(rgb(theme::text_muted(cx))),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(theme::SPACING_SM))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .text_size(px(theme::FONT_AGENT_CARD_TITLE))
                                        .font_family(fonts::HEADING_FONT_FAMILY)
                                        .text_color(rgb(theme::text_primary(cx)))
                                        .child(display_name),
                                )
                                .when_some(plan, |row, plan_label| {
                                    row.child(
                                        div()
                                            .flex_shrink_0()
                                            .px(px(theme::BADGE_PX))
                                            .py(px(theme::BADGE_PY))
                                            .rounded(px(theme::BADGE_RADIUS))
                                            .bg(rgb(theme::bg_card(cx)))
                                            .border_1()
                                            .border_color(rgb(theme::border_subtle(cx)))
                                            .text_size(px(theme::FONT_DETAIL))
                                            .text_color(rgb(theme::text_muted(cx)))
                                            .whitespace_nowrap()
                                            .child(plan_label.to_ascii_lowercase()),
                                    )
                                }),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(theme::SPACING_SM))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_size(px(theme::FONT_DETAIL))
                                .text_color(rgb(theme::text_muted(cx)))
                                .child(version),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .whitespace_nowrap()
                                .text_size(px(theme::FONT_DETAIL))
                                .text_color(rgb(theme::text_muted(cx)))
                                .child(sessions_label),
                        ),
                )
                .when_some(usage, |el, snap| {
                    el.child(render_usage_row(kind, &snap, cx))
                }),
        )
}

/// Compact usage row when live usage data is available (agent card, agent detail).
/// Renders rate-limit gauges (5h / 7d), reset times, and optional credit spend.
pub fn render_agent_usage_row(
    kind: ManagedAgentKind,
    snap: &AgentUsageSnapshot,
    cx: &App,
) -> impl IntoElement {
    render_usage_row(kind, snap, cx)
}

fn render_usage_row(
    kind: ManagedAgentKind,
    snap: &AgentUsageSnapshot,
    cx: &App,
) -> impl IntoElement {
    let five_reset = snap
        .rate_limit_five_hour_resets_at
        .and_then(format_reset_duration_hm);
    let seven_reset = snap
        .rate_limit_seven_day_resets_at
        .and_then(format_reset_duration_dh);

    let gauges = render_usage_gauges(snap, five_reset.as_deref(), seven_reset.as_deref(), cx);

    div()
        .id(SharedString::from(format!("agent-usage-{}", kind_slug(kind))))
        .w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(3.0))
        .children(gauges)
        .when_some(snap.total_cost_usd, |el, usd| {
            let text = if let Some(pct) = snap.context_used_percent {
                format!("${usd:.2} spent · {pct:.0}% of limit")
            } else {
                format!("${usd:.2} extra spend")
            };
            el.child(
                div()
                    .text_size(px(theme::FONT_DETAIL))
                    .text_color(rgb(theme::text_muted(cx)))
                    .child(text),
            )
        })
        .when(
            snap.lines_added.is_some() || snap.lines_removed.is_some(),
            |el| {
                let added = snap.lines_added.unwrap_or(0);
                let removed = snap.lines_removed.unwrap_or(0);
                let text = format!("+{added} / -{removed} lines changed");
                el.child(
                    div()
                        .text_size(px(theme::FONT_DETAIL))
                        .text_color(rgb(theme::text_muted(cx)))
                        .child(text),
                )
            },
        )
}

// Fixed column widths so 5h / 7d value columns align within each gauge.
const GAUGE_LABEL_W: f32 = 18.0;
const GAUGE_LABEL_BAR_GAP: f32 = 2.0;
const GAUGE_BAR_W: f32 = 40.0;
const GAUGE_BAR_VALUES_GAP: f32 = 6.0;
const GAUGE_PCT_W: f32 = 30.0;
const GAUGE_RESET_W: f32 = 40.0;

fn render_usage_gauges(
    snap: &AgentUsageSnapshot,
    five_reset: Option<&str>,
    seven_reset: Option<&str>,
    cx: &App,
) -> Vec<AnyElement> {
    let has_five = snap.rate_limit_five_hour_used_percent.is_some();
    let has_seven = snap.rate_limit_seven_day_used_percent.is_some();
    if !has_five && !has_seven {
        return Vec::new();
    }

    let mut gauges: Vec<AnyElement> = Vec::new();
    if let Some(pct) = snap.rate_limit_five_hour_used_percent {
        gauges.push(usage_gauge("5h", pct, five_reset, cx).into_any_element());
    }
    if let Some(pct) = snap.rate_limit_seven_day_used_percent {
        gauges.push(usage_gauge("7d", pct, seven_reset, cx).into_any_element());
    }

    vec![div()
        .w_full()
        .min_w_0()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .children(gauges)
        .into_any_element()]
}

/// Label + bar + fixed `%` and reset columns: `5h [████] 45%  2h30m`
fn usage_gauge(
    label: &'static str,
    pct: f32,
    resets_in: Option<&str>,
    cx: &App,
) -> impl IntoElement {
    let pct_clamped = pct.clamp(0.0, 100.0);
    let bar_color = usage_bar_color(pct_clamped, cx);
    let pct_text = format!("{pct:.0}%");

    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(GAUGE_BAR_VALUES_GAP))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(GAUGE_LABEL_BAR_GAP))
                .child(
                    div()
                        .w(px(GAUGE_LABEL_W))
                        .flex_shrink_0()
                        .whitespace_nowrap()
                        .text_size(px(theme::FONT_DETAIL))
                        .text_color(rgb(theme::text_muted(cx)))
                        .child(label),
                )
                .child(
                    div()
                        .w(px(GAUGE_BAR_W))
                        .h(px(3.0))
                        .flex_shrink_0()
                        .rounded_full()
                        .bg(rgb(theme::border_subtle(cx)))
                        .child(
                            div()
                                .h_full()
                                .rounded_full()
                                .w(px(GAUGE_BAR_W * pct_clamped / 100.0))
                                .bg(rgb(bar_color)),
                        ),
                ),
        )
        .child(
            div()
                .w(px(GAUGE_PCT_W))
                .flex_shrink_0()
                .flex()
                .justify_end()
                .child(
                    div()
                        .whitespace_nowrap()
                        .text_size(px(theme::FONT_DETAIL))
                        .text_color(rgb(bar_color))
                        .child(pct_text),
                ),
        )
        .child(
            div()
                .w(px(GAUGE_RESET_W))
                .flex_shrink_0()
                .flex()
                .justify_end()
                .child(
                    div()
                        .whitespace_nowrap()
                        .text_size(px(theme::FONT_DETAIL))
                        .text_color(rgb(theme::text_muted(cx)))
                        .child(SharedString::from(
                            resets_in.unwrap_or("").to_string(),
                        )),
                ),
        )
}

fn usage_bar_color(pct: f32, cx: &App) -> u32 {
    if pct >= 80.0 {
        theme::accent_red(cx)
    } else if pct >= 50.0 {
        theme::accent_yellow(cx)
    } else {
        theme::accent_green(cx)
    }
}

fn seconds_until_reset(resets_at: i64) -> Option<i64> {
    let secs = resets_at - chrono::Utc::now().timestamp();
    (secs > 0).then_some(secs)
}

/// 5h window reset: `2h30m`, `45m`.
fn format_reset_duration_hm(resets_at: i64) -> Option<String> {
    let secs = seconds_until_reset(resets_at)?;
    let hours = secs / 3600;
    let mins = (secs % 3600) / 60;
    if hours > 0 {
        Some(format!("{hours}h{mins}m"))
    } else {
        Some(format!("{}m", mins.max(1)))
    }
}

/// 7d window reset: `3d5h`, `12h`.
fn format_reset_duration_dh(resets_at: i64) -> Option<String> {
    let secs = seconds_until_reset(resets_at)?;
    let hours = secs / 3600;
    let mins = (secs % 3600) / 60;
    if hours >= 24 {
        let days = hours / 24;
        let rem = hours % 24;
        if rem > 0 {
            Some(format!("{days}d{rem}h"))
        } else {
            Some(format!("{days}d"))
        }
    } else if hours > 0 {
        Some(format!("{hours}h"))
    } else {
        Some(format!("{}m", mins.max(1)))
    }
}

// ---------------------------------------------------------------------------
// Session card
// ---------------------------------------------------------------------------

pub struct SessionCardProps {
    pub session: AgentSessionSummary,
    pub resume_on_tap: bool,
}

pub fn render_session_card<C: 'static>(
    props: SessionCardProps,
    cx: &mut Context<C>,
) -> Stateful<Div> {
    let session = props.session;
    let can_resume = session.resume.available;
    let kind = session.kind;
    let session_id = session.session_id.clone();
    let item_id = SharedString::from(format!(
        "session-card-{}-{}",
        kind_slug(kind),
        short_id(&session.session_id)
    ));

    div()
        .w_full()
        .min_w_0()
        .px(px(theme::SPACING_MD))
        .py(px(theme::SPACING_SM))
        .rounded(px(6.0))
        .border_1()
        .border_color(rgb(theme::border_subtle(cx)))
        .bg(rgb(theme::bg_card_dim(cx)))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .when(props.resume_on_tap && can_resume, |el| {
            el.cursor_pointer().on_press(cx.listener({
                let session_id = session_id.clone();
                move |_this, _event, window, cx| {
                    platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
                    window.dispatch_action(
                        workspace_action::ResumeAgentSession {
                            kind,
                            session_id: session_id.clone(),
                        }
                        .boxed_clone(),
                        cx,
                    );
                }
            }))
        })
        .id(item_id)
        .child(
            svg()
                .path(managed_agent_icon(kind))
                .size(px(theme::ICON_MD))
                .flex_shrink_0()
                .text_color(rgb(theme::text_muted(cx))),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(session_title_row(&session, cx))
                .child(session_meta_row(&session, cx)),
        )
}

pub fn session_title(session: &AgentSessionSummary) -> String {
    session
        .title
        .clone()
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| "Unknown".to_string())
}

fn session_title_row(session: &AgentSessionSummary, cx: &App) -> Div {
    let mut row = div()
        .w_full()
        .min_w_0()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_size(px(theme::FONT_BODY))
                .text_color(rgb(theme::text_primary(cx)))
                .child(session_title(session)),
        );

    if let Some(at) = session.last_activity_at.or(session.created_at) {
        row = row.child(
            div()
                .flex_shrink_0()
                .text_size(px(theme::FONT_DETAIL))
                .text_color(rgb(theme::text_muted(cx)))
                .child(format_session_time(at)),
        );
    }

    row
}

fn session_meta_row(session: &AgentSessionSummary, cx: &App) -> impl IntoElement {
    let branch = session
        .git
        .as_ref()
        .and_then(|git| git.branch.clone())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    let right = session_meta_tail(session);

    div()
        .id(SharedString::from(format!(
            "session-card-meta-{}",
            short_id(&session.session_id)
        )))
        .w_full()
        .min_w_0()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .text_size(px(theme::FONT_DETAIL))
        .text_color(rgb(theme::text_muted(cx)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(4.0))
                .overflow_hidden()
                .child(
                    svg()
                        .path("icons/git-branch.svg")
                        .size(px(theme::ICON_XS))
                        .flex_shrink_0()
                        .text_color(rgb(theme::text_muted(cx))),
                )
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(branch),
                ),
        )
        .when(!right.is_empty(), |el| {
            el.child(
                div()
                    .flex_shrink_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(right),
            )
        })
}

fn session_meta_tail(session: &AgentSessionSummary) -> String {
    session
        .transcript_size_bytes
        .map(format_size)
        .unwrap_or_default()
}

fn format_session_time(at: DateTime<Utc>) -> String {
    at.format("%H:%M").to_string()
}

fn format_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

// ---------------------------------------------------------------------------
// Session list
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct AgentSessionSection {
    pub label: String,
    pub sessions: Vec<AgentSessionSummary>,
}

pub fn group_sessions_by_day(sessions: Vec<AgentSessionSummary>) -> Vec<AgentSessionSection> {
    let mut sorted = sessions;
    sorted.sort_by(|left, right| {
        right
            .last_activity_at
            .cmp(&left.last_activity_at)
            .then_with(|| right.created_at.cmp(&left.created_at))
    });

    let mut sections = Vec::new();
    for session in sorted {
        let label = day_label(session.last_activity_at.or(session.created_at));
        if sections
            .last()
            .is_some_and(|section: &AgentSessionSection| section.label == label)
        {
            sections
                .last_mut()
                .expect("section exists")
                .sessions
                .push(session);
        } else {
            sections.push(AgentSessionSection {
                label,
                sessions: vec![session],
            });
        }
    }
    sections
}

pub struct AgentSessionListProps {
    pub sections: Vec<AgentSessionSection>,
    pub loading: bool,
    pub error: Option<String>,
    pub empty_message: &'static str,
    pub resume_on_tap: bool,
    /// When true, the list fills remaining height and scrolls internally.
    pub scroll_container: bool,
    /// When true, applies horizontal padding on the list container.
    pub horizontal_padding: bool,
}

pub fn render_agent_session_list<C: 'static>(
    props: AgentSessionListProps,
    cx: &mut Context<C>,
) -> impl IntoElement {
    let mut list = div()
        .id("agent-session-list")
        .w_full()
        .min_w_0()
        .pb(px(theme::SPACING_MD));
    if props.horizontal_padding {
        list = list.px(px(theme::SUBSCREEN_PADDING_X));
    }
    if props.scroll_container {
        list = list.flex_1().min_h_0().overflow_y_scroll();
    }
    list = list.flex().flex_col().gap(px(theme::SPACING_SM));

    if props.loading {
        return list.child(list_empty_text("Loading sessions...", cx));
    }
    if let Some(error) = props.error {
        return list.child(list_empty_text(
            format!("Failed to load sessions: {error}"),
            cx,
        ));
    }
    if props.sections.is_empty() {
        return list.child(list_empty_text(props.empty_message, cx));
    }

    for section in props.sections {
        list = list.child(section_header(&section.label, cx));
        for session in section.sessions {
            list = list.child(render_session_card(
                SessionCardProps {
                    session,
                    resume_on_tap: props.resume_on_tap,
                },
                cx,
            ));
        }
    }
    list
}

fn section_header(label: &str, cx: &App) -> Div {
    div()
        .w_full()
        .min_w_0()
        .pt(px(theme::SPACING_SM))
        .pb(px(4.0))
        .text_size(px(theme::FONT_DETAIL))
        .text_color(rgb(theme::text_muted(cx)))
        .child(label.to_string())
}

fn list_empty_text(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .w_full()
        .min_w_0()
        .py(px(theme::SPACING_LG))
        .text_size(px(theme::FONT_BODY))
        .text_color(rgb(theme::text_muted(cx)))
        .whitespace_normal()
        .child(text.into())
}

fn day_label(at: Option<DateTime<Utc>>) -> String {
    let Some(at) = at else {
        return "Unknown date".to_string();
    };
    let today = Utc::now().date_naive();
    let date = at.date_naive();
    if date == today {
        "Today".to_string()
    } else if date == today.pred_opt().unwrap_or(today) {
        "Yesterday".to_string()
    } else {
        at.format("%A, %b %d").to_string()
    }
}
