use gpui::*;
use tracing::error;
use zedra_rpc::proto::ManagedAgentKind;
use zedra_session::SessionHandle;

use crate::agent_session_list::{
    AgentSessionListProps, group_sessions_by_day, render_agent_session_list,
};
use crate::theme;

#[derive(Clone, Debug)]
enum LoadState {
    Loading,
    Ready,
    Error(String),
}

pub struct AgentSessionView {
    session_handle: SessionHandle,
    sections: Vec<crate::agent_session_list::AgentSessionSection>,
    load_state: LoadState,
    loading_epoch: u64,
    _tasks: Vec<Task<()>>,
}

impl AgentSessionView {
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
            let kinds = [
                ManagedAgentKind::Claude,
                ManagedAgentKind::Codex,
                ManagedAgentKind::OpenCode,
            ];
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

impl Render for AgentSessionView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("agent-session-view")
            .size_full()
            .min_h_0()
            .bg(rgb(theme::BG_PRIMARY))
            .flex()
            .flex_col()
            .child(
                div()
                    .px(px(theme::SPACING_MD))
                    .pt(px(theme::SPACING_MD))
                    .pb(px(theme::SPACING_SM))
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(theme::FONT_BODY))
                            .text_color(rgb(theme::TEXT_SECONDARY))
                            .child("All managed agent sessions in this workspace."),
                    )
                    .child(refresh_button(cx)),
            )
            .child(render_agent_session_list(
                AgentSessionListProps {
                    sections: self.sections.clone(),
                    loading: matches!(self.load_state, LoadState::Loading),
                    error: match &self.load_state {
                        LoadState::Error(message) => Some(message.clone()),
                        _ => None,
                    },
                    empty_message: "No sessions found for this workspace.",
                    resume_on_tap: true,
                },
                cx,
            ))
    }
}

fn refresh_button(cx: &mut Context<AgentSessionView>) -> Stateful<Div> {
    div()
        .id("agent-session-refresh-btn")
        .px(px(10.0))
        .py(px(6.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(rgb(theme::BORDER_SUBTLE))
        .cursor_pointer()
        .on_press(cx.listener(|this, _event, _window, cx| {
            this.load(true, cx);
        }))
        .child(
            div()
                .text_size(px(theme::FONT_DETAIL))
                .text_color(rgb(theme::TEXT_SECONDARY))
                .child("Refresh"),
        )
}
