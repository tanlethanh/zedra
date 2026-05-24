use gpui::*;
use tracing::error;
use zedra_rpc::proto::ManagedAgentKind;
use zedra_session::SessionHandle;

use crate::agent_ui::{
    AgentSessionListProps, AgentSessionSection, group_sessions_by_day, render_agent_session_list,
};
use crate::fonts;
use crate::platform_bridge::{self, HapticFeedback};
use crate::theme;
use crate::ui::{chevron_back_button, subscreen_page, subscreen_refresh_button};
use crate::workspace_action;

#[derive(Clone, Debug)]
enum LoadState {
    Loading,
    Ready,
    Error(String),
}

pub struct AgentSessions {
    session_handle: SessionHandle,
    sections: Vec<AgentSessionSection>,
    load_state: LoadState,
    loading_epoch: u64,
    _tasks: Vec<Task<()>>,
}

impl AgentSessions {
    pub fn new(session_handle: SessionHandle, cx: &mut Context<Self>) -> Self {
        let mut view = Self {
            session_handle,
            sections: Vec::new(),
            load_state: LoadState::Loading,
            loading_epoch: 0,
            _tasks: Vec::new(),
        };
        view.load(false, cx);
        view
    }

    fn load(&mut self, refresh: bool, cx: &mut Context<Self>) {
        self.loading_epoch = self.loading_epoch.wrapping_add(1);
        let epoch = self.loading_epoch;
        self.load_state = LoadState::Loading;
        self.sections.clear();
        cx.notify();

        let handle = self.session_handle.clone();
        let task = cx.spawn(async move |this, cx| {
            let (claude, codex, opencode) = tokio::join!(
                handle.agent_sessions(ManagedAgentKind::Claude, refresh, 0),
                handle.agent_sessions(ManagedAgentKind::Codex, refresh, 0),
                handle.agent_sessions(ManagedAgentKind::OpenCode, refresh, 0),
            );
            let mut sessions = Vec::new();
            let mut errors = Vec::new();
            for (kind, result) in [
                (ManagedAgentKind::Claude, claude),
                (ManagedAgentKind::Codex, codex),
                (ManagedAgentKind::OpenCode, opencode),
            ] {
                match result {
                    Ok(mut rows) => sessions.append(&mut rows),
                    Err(err) => errors.push(format!("{kind:?}: {err}")),
                }
            }
            let _ = this.update(cx, |this, cx| {
                if this.loading_epoch != epoch {
                    return;
                }
                this.sections = group_sessions_by_day(sessions);
                this.load_state = if errors.is_empty() {
                    LoadState::Ready
                } else if this.sections.is_empty() {
                    LoadState::Error(errors.join("; "))
                } else {
                    error!("agent sessions partial failure: {}", errors.join("; "));
                    LoadState::Ready
                };
                cx.notify();
            });
        });
        self._tasks.push(task);
    }
}

impl Render for AgentSessions {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = render_agent_session_list(
            AgentSessionListProps {
                sections: self.sections.clone(),
                loading: matches!(self.load_state, LoadState::Loading),
                error: match &self.load_state {
                    LoadState::Error(message) => Some(message.clone()),
                    _ => None,
                },
                empty_message: "No sessions found for this workspace.",
                resume_on_tap: true,
                scroll_container: false,
                horizontal_padding: true,
            },
            cx,
        )
        .into_any_element();
        let header = render_session_header(cx).into_any_element();
        subscreen_page("agent-sessions", rgb(theme::bg_primary(cx)), header, body)
    }
}

fn render_session_header(cx: &mut Context<AgentSessions>) -> impl IntoElement {
    div()
        .id("agent-sessions-header")
        .min_w_0()
        .px(px(theme::SUBSCREEN_PADDING_X))
        .pt(px(theme::SPACING_XS))
        .pb(px(theme::SPACING_SM))
        .child(
            div()
                .id("agent-sessions-header-inner")
                .relative()
                .min_w_0()
                .child(
                    div()
                        .min_w_0()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(theme::SPACING_MD))
                        .child(back_button(cx))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(0.0))
                                .child(
                                    div()
                                        .text_size(px(theme::FONT_HEADING))
                                        .font_family(fonts::HEADING_FONT_FAMILY)
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(rgb(theme::text_primary(cx)))
                                        .child("Agent history"),
                                )
                                .child(
                                    div()
                                        .text_size(px(theme::FONT_BODY))
                                        .text_color(rgb(theme::text_muted(cx)))
                                        .child("Sessions across agents. Press to resume"),
                                ),
                        ),
                )
                .child(subscreen_refresh_button(
                    "agent-sessions-refresh-btn",
                    cx,
                    |this, _event, _window, cx| this.load(true, cx),
                )),
        )
}

fn back_button(cx: &mut Context<AgentSessions>) -> Stateful<Div> {
    chevron_back_button(
        "agent-sessions-back-btn",
        cx,
        |_this, _event, window, cx| {
            platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
            window.dispatch_action(workspace_action::NavigateBack.boxed_clone(), cx);
        },
    )
}
