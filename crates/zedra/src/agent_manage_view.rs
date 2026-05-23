use gpui::*;
use tracing::error;
use zedra_rpc::proto::{AgentSetupState, AgentSummary, ManagedAgentKind};
use zedra_session::SessionHandle;

use crate::agent_session_list::{
    AgentSessionListProps, agent_icon, kind_slug, managed_agent_name, render_agent_session_list,
};
use crate::platform_bridge::{self, HapticFeedback};
use crate::theme;

#[derive(Clone, Debug)]
enum LoadState {
    Loading,
    Ready,
    Error(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Screen {
    List,
    Detail(ManagedAgentKind),
}

pub struct AgentManageView {
    session_handle: SessionHandle,
    agents: Vec<AgentSummary>,
    sessions: Vec<zedra_rpc::proto::AgentSessionSummary>,
    screen: Screen,
    agent_state: LoadState,
    session_state: LoadState,
    loading_epoch: u64,
    _tasks: Vec<Task<()>>,
}

impl AgentManageView {
    pub fn new(session_handle: SessionHandle, cx: &mut Context<Self>) -> Self {
        let mut view = Self {
            session_handle,
            agents: Vec::new(),
            sessions: Vec::new(),
            screen: Screen::List,
            agent_state: LoadState::Loading,
            session_state: LoadState::Ready,
            loading_epoch: 0,
            _tasks: Vec::new(),
        };
        view.load_agents(false, cx);
        view
    }

    fn load_agents(&mut self, refresh: bool, cx: &mut Context<Self>) {
        self.loading_epoch = self.loading_epoch.wrapping_add(1);
        let epoch = self.loading_epoch;
        self.agent_state = LoadState::Loading;
        self.session_state = LoadState::Ready;
        cx.notify();

        let handle = self.session_handle.clone();
        let task = cx.spawn(
            async move |this, cx| match handle.agent_list(refresh).await {
                Ok(agents) => {
                    let _ = this.update(cx, |this, cx| {
                        if this.loading_epoch != epoch {
                            return;
                        }
                        this.agents = agents;
                        this.agent_state = LoadState::Ready;
                        if let Screen::Detail(kind) = this.screen {
                            if this.agents.iter().any(|agent| agent.kind == kind) {
                                this.fetch_sessions(kind, refresh, cx);
                            } else {
                                this.screen = Screen::List;
                                this.sessions.clear();
                            }
                        }
                        cx.notify();
                    });
                }
                Err(err) => {
                    error!("agent list failed: {}", err);
                    let _ = this.update(cx, |this, cx| {
                        if this.loading_epoch != epoch {
                            return;
                        }
                        this.agent_state = LoadState::Error(err.to_string());
                        this.sessions.clear();
                        cx.notify();
                    });
                }
            },
        );
        self._tasks.push(task);
    }

    fn open_agent(&mut self, kind: ManagedAgentKind, cx: &mut Context<Self>) {
        self.screen = Screen::Detail(kind);
        self.fetch_sessions(kind, false, cx);
        cx.notify();
    }

    fn back_to_list(&mut self, cx: &mut Context<Self>) {
        self.screen = Screen::List;
        self.sessions.clear();
        self.session_state = LoadState::Ready;
        cx.notify();
    }

    fn fetch_sessions(&mut self, kind: ManagedAgentKind, refresh: bool, cx: &mut Context<Self>) {
        self.loading_epoch = self.loading_epoch.wrapping_add(1);
        let epoch = self.loading_epoch;
        self.sessions.clear();
        self.session_state = LoadState::Loading;
        cx.notify();

        let handle = self.session_handle.clone();
        let task =
            cx.spawn(
                async move |this, cx| match handle.agent_sessions(kind, refresh, 0).await {
                    Ok(sessions) => {
                        let _ = this.update(cx, |this, cx| {
                            if this.loading_epoch != epoch {
                                return;
                            }
                            if this.screen != Screen::Detail(kind) {
                                return;
                            }
                            this.sessions = sessions;
                            this.session_state = LoadState::Ready;
                            cx.notify();
                        });
                    }
                    Err(err) => {
                        error!(?kind, "agent sessions failed: {}", err);
                        let _ = this.update(cx, |this, cx| {
                            if this.loading_epoch != epoch || this.screen != Screen::Detail(kind) {
                                return;
                            }
                            this.sessions.clear();
                            this.session_state = LoadState::Error(err.to_string());
                            cx.notify();
                        });
                    }
                },
            );
        self._tasks.push(task);
    }

    fn selected_agent(&self) -> Option<&AgentSummary> {
        let Screen::Detail(kind) = self.screen else {
            return None;
        };
        self.agents.iter().find(|agent| agent.kind == kind)
    }

    fn render_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list = div()
            .id("agent-manage-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(theme::SPACING_MD))
            .pb(px(theme::SPACING_MD))
            .flex()
            .flex_col()
            .gap(px(theme::SPACING_SM));

        for agent in self.agents.clone() {
            let kind = agent.kind;
            list = list.child(
                div()
                    .id(SharedString::from(format!(
                        "agent-manage-row-{}",
                        kind_slug(kind)
                    )))
                    .px(px(theme::SPACING_MD))
                    .py(px(theme::SPACING_SM))
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(rgb(theme::border_subtle(cx)))
                    .bg(rgb(theme::bg_card(cx)))
                    .cursor_pointer()
                    .on_press(cx.listener(move |this, _event, _window, cx| {
                        platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
                        this.open_agent(kind, cx);
                    }))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(8.0))
                            .child(
                                svg()
                                    .path(agent_icon(kind))
                                    .size(px(theme::ICON_SM))
                                    .text_color(rgb(theme::text_muted(cx))),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .truncate()
                                            .text_size(px(theme::FONT_BODY))
                                            .text_color(rgb(theme::text_primary(cx)))
                                            .child(agent.display_name.clone()),
                                    )
                                    .child(
                                        div()
                                            .truncate()
                                            .text_size(px(theme::FONT_DETAIL))
                                            .text_color(rgb(theme::text_muted(cx)))
                                            .child(format!(
                                                "{} · {} sessions",
                                                setup_label(agent.setup.state),
                                                agent.sessions.resumable.max(agent.sessions.total)
                                            )),
                                    ),
                            ),
                    ),
            );
        }
        list
    }

    fn render_detail(&self, cx: &mut Context<Self>) -> Div {
        let Some(agent) = self.selected_agent() else {
            return empty_text("No agent selected.", cx);
        };
        let kind = agent.kind;

        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .px(px(theme::SPACING_MD))
                    .pt(px(theme::SPACING_SM))
                    .pb(px(theme::SPACING_SM))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(theme::SPACING_SM))
                    .child(back_button(cx))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(theme::FONT_BODY))
                                    .text_color(rgb(theme::text_primary(cx)))
                                    .child(agent.display_name.clone()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(theme::FONT_DETAIL))
                                    .text_color(rgb(theme::text_muted(cx)))
                                    .child(managed_agent_name(kind)),
                            ),
                    )
                    .child(refresh_button(cx)),
            )
            .child(render_detail_summary(agent, cx))
            .child(section_header("Sessions", cx))
            .child(render_agent_session_list(
                AgentSessionListProps {
                    sections: crate::agent_session_list::group_sessions_by_day(
                        self.sessions.clone(),
                    ),
                    loading: matches!(self.session_state, LoadState::Loading),
                    error: match &self.session_state {
                        LoadState::Error(message) => Some(message.clone()),
                        _ => None,
                    },
                    empty_message: "No sessions found for this agent.",
                    resume_on_tap: true,
                },
                cx,
            ))
    }
}

impl Render for AgentManageView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut root = div()
            .id("agent-manage-view")
            .size_full()
            .min_h_0()
            .bg(rgb(theme::bg_primary(cx)))
            .flex()
            .flex_col();

        match &self.agent_state {
            LoadState::Loading => {
                root = root.child(empty_text("Loading managed agents...", cx));
            }
            LoadState::Error(message) => {
                root = root
                    .child(empty_text(format!(
                        "Failed to load managed agents: {message}"
                    ), cx))
                    .child(refresh_button(cx));
            }
            LoadState::Ready => {
                root = match self.screen {
                    Screen::List => root.child(self.render_list(cx)),
                    Screen::Detail(_) => root.child(self.render_detail(cx)),
                };
            }
        }
        root
    }
}

fn render_detail_summary(agent: &AgentSummary, cx: &App) -> Div {
    let mut summary = div()
        .mx(px(theme::SPACING_MD))
        .mb(px(theme::SPACING_SM))
        .px(px(theme::SPACING_MD))
        .py(px(theme::SPACING_SM))
        .rounded(px(6.0))
        .border_1()
        .border_color(rgb(theme::border_subtle(cx)))
        .bg(rgb(theme::bg_card(cx)))
        .flex()
        .flex_col()
        .gap(px(4.0));

    let cli = if agent.cli.available {
        agent
            .cli
            .version
            .clone()
            .unwrap_or_else(|| "available".to_string())
    } else {
        agent
            .cli
            .error
            .clone()
            .unwrap_or_else(|| "missing".to_string())
    };

    summary = summary
        .child(summary_line("CLI", cli, cx))
        .child(summary_line(
            "Setup",
            setup_label(agent.setup.state).to_string(),
            cx,
        ))
        .child(summary_line(
            "Sessions",
            format!(
                "{} resumable / {} total",
                agent.sessions.resumable, agent.sessions.total
            ),
            cx,
        ));

    for field in &agent.account.fields {
        summary = summary.child(summary_line(&field.label, field.value.clone(), cx));
    }

    if !agent.warnings.is_empty() {
        summary = summary.child(summary_line(
            "Warnings",
            agent
                .warnings
                .iter()
                .map(|warning| warning.code.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            cx,
        ));
    }
    summary
}

fn back_button(cx: &mut Context<AgentManageView>) -> Stateful<Div> {
    div()
        .id("agent-manage-back-btn")
        .px(px(8.0))
        .py(px(6.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(rgb(theme::border_subtle(cx)))
        .cursor_pointer()
        .on_press(cx.listener(|this, _event, _window, cx| {
            platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
            this.back_to_list(cx);
        }))
        .child(
            svg()
                .path("icons/chevron-left.svg")
                .size(px(theme::ICON_SM))
                .text_color(rgb(theme::text_secondary(cx))),
        )
}

fn refresh_button(cx: &mut Context<AgentManageView>) -> Stateful<Div> {
    div()
        .id("agent-manage-refresh-btn")
        .px(px(10.0))
        .py(px(6.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(rgb(theme::border_subtle(cx)))
        .cursor_pointer()
        .on_press(cx.listener(|this, _event, _window, cx| {
            this.load_agents(true, cx);
        }))
        .child(
            div()
                .text_size(px(theme::FONT_DETAIL))
                .text_color(rgb(theme::text_secondary(cx)))
                .child("Refresh"),
        )
}

fn section_header(label: &'static str, cx: &App) -> Div {
    div()
        .px(px(theme::SPACING_MD))
        .pt(px(theme::SPACING_SM))
        .pb(px(4.0))
        .text_size(px(theme::FONT_DETAIL))
        .text_color(rgb(theme::text_muted(cx)))
        .child(label)
}

fn summary_line(label: &str, value: String, cx: &App) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::SPACING_SM))
        .child(
            div()
                .w(px(120.0))
                .flex_shrink_0()
                .text_size(px(theme::FONT_DETAIL))
                .text_color(rgb(theme::text_muted(cx)))
                .child(label.to_string()),
        )
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(px(theme::FONT_DETAIL))
                .text_color(rgb(theme::text_secondary(cx)))
                .child(value),
        )
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

fn setup_label(state: AgentSetupState) -> &'static str {
    match state {
        AgentSetupState::MissingCli => "Missing CLI",
        AgentSetupState::NotConfigured => "Not configured",
        AgentSetupState::SkillsOnly => "Skills only",
        AgentSetupState::HooksReady => "Hooks ready",
        AgentSetupState::Error => "Error",
    }
}
