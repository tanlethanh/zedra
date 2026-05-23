use gpui::prelude::FluentBuilder;
use gpui::*;
use tracing::error;
use zedra_rpc::proto::{AgentSetupState, AgentSummary, HostEvent, ManagedAgentKind};
use zedra_session::{Session, SessionHandle};

use crate::agent_session_list::{
    AgentSessionListProps, agent_icon, kind_slug, render_agent_session_list,
};
use crate::fonts;
use crate::platform_bridge::{self, HapticFeedback};
use crate::theme;
use crate::ui::{
    chevron_back_button, subscreen_padded_body, subscreen_page, subscreen_refresh_button,
};
use crate::workspace_action;

#[derive(Clone, Debug, PartialEq)]
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
    pub fn new(session_handle: SessionHandle, session: Session, cx: &mut Context<Self>) -> Self {
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
        view.subscribe_agent_info(session, cx);
        view.load_agents(false, cx);
        view
    }

    fn subscribe_agent_info(&mut self, session: Session, cx: &mut Context<Self>) {
        let mut host_event_rx = session.subscribe_host_events();
        let task = cx.spawn(async move |this, cx| {
            loop {
                match host_event_rx.recv().await {
                    Ok(HostEvent::AgentInfoChanged { info }) => {
                        let should_break = this
                            .update(cx, |this, cx| {
                                if let Some(agent) =
                                    this.agents.iter_mut().find(|agent| agent.kind == info.kind)
                                {
                                    *agent = info;
                                } else {
                                    this.agents.push(info);
                                }
                                this.agent_state = LoadState::Ready;
                                cx.notify();
                            })
                            .is_err();
                        if should_break {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!("agent manage host event listener lagged by {}", skipped);
                        let should_break = this
                            .update(cx, |this, cx| this.load_agents(false, cx))
                            .is_err();
                        if should_break {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        self._tasks.push(task);
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

    fn render_list_header(cx: &mut Context<Self>) -> impl IntoElement {
        render_manage_header(
            cx,
            ManageHeaderConfig {
                id_prefix: "agent-manage-list",
                back: ManageHeaderBack::Workspace,
                title: "Manage agents".into(),
                description: "Agents installed on this host.".into(),
            },
        )
    }

    fn render_detail_header(agent: &AgentSummary, cx: &mut Context<Self>) -> impl IntoElement {
        render_manage_header(
            cx,
            ManageHeaderConfig {
                id_prefix: "agent-manage-detail",
                back: ManageHeaderBack::List,
                title: agent.display_name.clone().into(),
                description: cli_version_display(agent).into(),
            },
        )
    }

    fn render_list_body(agents: &[AgentSummary], cx: &mut Context<Self>) -> impl IntoElement {
        let mut list = div()
            .id("agent-manage-list")
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(theme::SPACING_SM));

        for agent in agents {
            let kind = agent.kind;
            let display_name = agent.display_name.clone();
            let version = cli_version_display(agent);
            let session_count = agent.sessions.resumable.max(agent.sessions.total);
            let sessions_label = format!("{session_count} sessions");
            list = list.child(
                div()
                    .id(SharedString::from(format!(
                        "agent-manage-row-{}",
                        kind_slug(kind)
                    )))
                    .w_full()
                    .min_w_0()
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
                            .gap(px(10.0))
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
                                    .child(
                                        div()
                                            .min_w_0()
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .text_size(px(theme::FONT_BODY))
                                            .text_color(rgb(theme::text_primary(cx)))
                                            .child(display_name),
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
                                    ),
                            ),
                    ),
            );
        }
        subscreen_padded_body(list)
    }

    fn render_detail_body(
        agent: &AgentSummary,
        sessions: &[zedra_rpc::proto::AgentSessionSummary],
        session_state: &LoadState,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        subscreen_padded_body(
            div()
                .w_full()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(theme::SPACING_MD))
                .child(render_detail_summary(agent, cx))
                .child(render_agent_session_list(
                    AgentSessionListProps {
                        sections: crate::agent_session_list::group_sessions_by_day(
                            sessions.to_vec(),
                        ),
                        loading: matches!(session_state, LoadState::Loading),
                        error: match session_state {
                            LoadState::Error(message) => Some(message.clone()),
                            _ => None,
                        },
                        empty_message: "No sessions found for this agent.",
                        resume_on_tap: true,
                        scroll_container: false,
                        horizontal_padding: false,
                    },
                    cx,
                )),
        )
    }
}

impl Render for AgentManageView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let screen = self.screen;
        let agent_state = self.agent_state.clone();
        let agents = self.agents.clone();
        let sessions = self.sessions.clone();
        let session_state = self.session_state.clone();
        let detail_agent = match screen {
            Screen::Detail(kind) => agents.iter().find(|agent| agent.kind == kind).cloned(),
            Screen::List => None,
        };

        let header: AnyElement = match (&agent_state, screen, detail_agent.as_ref()) {
            (LoadState::Ready, Screen::Detail(_), Some(agent)) => {
                Self::render_detail_header(agent, cx).into_any_element()
            }
            _ => Self::render_list_header(cx).into_any_element(),
        };

        let body: AnyElement = match agent_state {
            LoadState::Loading => {
                subscreen_padded_body(empty_text("Loading managed agents...", cx))
                    .into_any_element()
            }
            LoadState::Error(message) => subscreen_padded_body(empty_text(
                format!("Failed to load managed agents: {message}"),
                cx,
            ))
            .into_any_element(),
            LoadState::Ready => match screen {
                Screen::List => Self::render_list_body(&agents, cx).into_any_element(),
                Screen::Detail(_) => {
                    if let Some(agent) = detail_agent.as_ref() {
                        Self::render_detail_body(agent, &sessions, &session_state, cx)
                            .into_any_element()
                    } else {
                        subscreen_padded_body(empty_text("No agent selected.", cx))
                            .into_any_element()
                    }
                }
            },
        };

        subscreen_page(
            "agent-manage-view",
            rgb(theme::bg_primary(cx)),
            header,
            body,
        )
    }
}

#[derive(Clone, Copy)]
enum ManageHeaderBack {
    Workspace,
    List,
}

struct ManageHeaderConfig {
    id_prefix: &'static str,
    back: ManageHeaderBack,
    title: SharedString,
    description: SharedString,
}

fn render_manage_header(
    cx: &mut Context<AgentManageView>,
    config: ManageHeaderConfig,
) -> impl IntoElement {
    div()
        .id(SharedString::from(format!("{}-header", config.id_prefix)))
        .min_w_0()
        .px(px(theme::SUBSCREEN_PADDING_X))
        .pt(px(theme::SPACING_XS))
        .pb(px(theme::SPACING_SM))
        .child(
            div()
                .id(SharedString::from(format!(
                    "{}-header-inner",
                    config.id_prefix
                )))
                .relative()
                .min_w_0()
                .child(
                    div()
                        .min_w_0()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(theme::SPACING_MD))
                        .child(manage_back_button(cx, config.back))
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
                                        .child(config.title),
                                )
                                .child(
                                    div()
                                        .text_size(px(theme::FONT_BODY))
                                        .text_color(rgb(theme::text_muted(cx)))
                                        .child(config.description),
                                ),
                        ),
                )
                .child(subscreen_refresh_button(
                    "agent-manage-refresh-btn",
                    cx,
                    |this, _event, _window, cx| this.load_agents(true, cx),
                )),
        )
}

fn render_detail_summary(agent: &AgentSummary, cx: &App) -> impl IntoElement {
    let mut rows: Vec<(SharedString, String)> = vec![
        ("Version".into(), cli_version_display(agent)),
        ("Setup".into(), setup_label(agent.setup.state).to_string()),
        (
            "Sessions".into(),
            format!(
                "{} resumable / {} total",
                agent.sessions.resumable, agent.sessions.total
            ),
        ),
    ];
    for field in &agent.account.fields {
        rows.push((field.label.clone().into(), field.value.clone()));
    }
    if !agent.warnings.is_empty() {
        rows.push((
            "Warnings".into(),
            agent
                .warnings
                .iter()
                .map(|warning| warning.code.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        ));
    }

    let last = rows.len().saturating_sub(1);
    let mut summary = div()
        .id("agent-manage-detail-metadata")
        .w_full()
        .min_w_0()
        .pb(px(theme::SPACING_XS))
        .border_b_1()
        .border_color(rgb(theme::border_subtle(cx)))
        .flex()
        .flex_col();

    for (index, (label, value)) in rows.into_iter().enumerate() {
        summary = summary.child(summary_line(label, value, cx, index < last));
    }
    summary
}

fn manage_back_button(cx: &mut Context<AgentManageView>, back: ManageHeaderBack) -> Stateful<Div> {
    chevron_back_button(
        "agent-manage-back-btn",
        cx,
        move |this, _event, window, cx| {
            platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
            match back {
                ManageHeaderBack::Workspace => {
                    window.dispatch_action(workspace_action::NavigateBack.boxed_clone(), cx);
                }
                ManageHeaderBack::List => this.back_to_list(cx),
            }
        },
    )
}

fn summary_line(label: SharedString, value: String, cx: &App, show_divider: bool) -> Div {
    div()
        .min_w_0()
        .flex()
        .flex_row()
        .items_baseline()
        .gap(px(theme::SPACING_SM))
        .py(px(theme::AGENT_METADATA_ROW_PY))
        .when(show_divider, |row| {
            row.border_b_1().border_color(rgb(theme::border_subtle(cx)))
        })
        .child(
            div()
                .w(px(theme::AGENT_METADATA_LABEL_WIDTH))
                .flex_shrink_0()
                .text_size(px(theme::FONT_DETAIL))
                .text_color(rgb(theme::text_muted(cx)))
                .child(label),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(theme::FONT_DETAIL))
                .text_color(rgb(theme::text_secondary(cx)))
                .child(value),
        )
}

fn empty_text(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .w_full()
        .min_w_0()
        .py(px(theme::SPACING_LG))
        .text_size(px(theme::FONT_BODY))
        .text_color(rgb(theme::text_muted(cx)))
        .whitespace_normal()
        .child(text.into())
}

fn cli_version_display(agent: &AgentSummary) -> String {
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

fn setup_label(state: AgentSetupState) -> &'static str {
    match state {
        AgentSetupState::MissingCli => "Missing CLI",
        AgentSetupState::NotConfigured => "Not configured",
        AgentSetupState::SkillsOnly => "Skills only",
        AgentSetupState::HooksReady => "Hooks ready",
        AgentSetupState::Error => "Error",
    }
}
