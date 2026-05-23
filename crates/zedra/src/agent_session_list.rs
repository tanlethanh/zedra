use chrono::{DateTime, Utc};
use gpui::prelude::FluentBuilder;
use gpui::*;
use zedra_rpc::proto::{AgentSessionSummary, ManagedAgentKind};

use crate::platform_bridge::{self, HapticFeedback};
use crate::{theme, workspace_action};

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
    let mut list = div().id("agent-session-list").pb(px(theme::SPACING_MD));
    if props.horizontal_padding {
        list = list.px(px(theme::SPACING_MD));
    }
    if props.scroll_container {
        list = list.flex_1().min_h_0().overflow_y_scroll();
    }
    list = list.flex().flex_col().gap(px(theme::SPACING_SM));

    if props.loading {
        return list.child(empty_text("Loading sessions...", cx));
    }
    if let Some(error) = props.error {
        return list.child(empty_text(format!("Failed to load sessions: {error}"), cx));
    }
    if props.sections.is_empty() {
        return list.child(empty_text(props.empty_message, cx));
    }

    for section in props.sections {
        list = list.child(section_header(&section.label, cx));
        for session in section.sessions {
            list = list.child(render_agent_session_item(session, props.resume_on_tap, cx));
        }
    }
    list
}

pub fn render_agent_session_item<C: 'static>(
    session: AgentSessionSummary,
    resume_on_tap: bool,
    cx: &mut Context<C>,
) -> Stateful<Div> {
    let can_resume = session.resume.available;
    let kind = session.kind;
    let session_id = session.session_id.clone();
    let item_id = SharedString::from(format!(
        "agent-session-{}-{}",
        kind_slug(kind),
        short_id(&session.session_id)
    ));

    div()
        .px(px(theme::SPACING_MD))
        .py(px(theme::SPACING_SM))
        .rounded(px(6.0))
        .border_1()
        .border_color(rgb(theme::border_subtle(cx)))
        .bg(rgb(theme::bg_card(cx)))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .when(resume_on_tap && can_resume, |el| {
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
                .path(agent_icon(kind))
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

fn section_header(label: &str, cx: &App) -> Div {
    div()
        .pt(px(theme::SPACING_SM))
        .pb(px(4.0))
        .text_size(px(theme::FONT_DETAIL))
        .text_color(rgb(theme::text_muted(cx)))
        .child(label.to_string())
}

fn empty_text(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .flex()
        .flex_1()
        .items_center()
        .justify_center()
        .px(px(theme::SPACING_MD))
        .text_size(px(theme::FONT_BODY))
        .text_color(rgb(theme::text_muted(cx)))
        .child(text.into())
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
            "agent-session-meta-{}",
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

pub fn session_title(session: &AgentSessionSummary) -> String {
    session
        .title
        .clone()
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| "Unknown".to_string())
}

fn format_session_time(at: DateTime<Utc>) -> String {
    at.format("%H:%M").to_string()
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

fn format_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

pub fn agent_icon(kind: ManagedAgentKind) -> &'static str {
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
