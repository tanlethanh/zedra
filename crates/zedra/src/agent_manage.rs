use gpui::*;
use tracing::error;
use zedra_rpc::proto::{AgentSummary, HostEvent};
use zedra_session::{Session, SessionHandle};

use crate::agent_ui::{AgentCardProps, render_agent_card};
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

pub struct AgentManage {
    session_handle: SessionHandle,
    agents: Vec<AgentSummary>,
    agent_state: LoadState,
    loading_epoch: u64,
    _tasks: Vec<Task<()>>,
}

impl AgentManage {
    pub fn new(session_handle: SessionHandle, session: Session, cx: &mut Context<Self>) -> Self {
        let mut view = Self {
            session_handle,
            agents: Vec::new(),
            agent_state: LoadState::Loading,
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
                                    this.agents.iter_mut().find(|agent| agent.slug == info.slug)
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
                        cx.notify();
                    });
                }
            },
        );
        self._tasks.push(task);
    }
}

impl Render for AgentManage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let agent_state = self.agent_state.clone();
        let agents = self.agents.clone();

        let body: AnyElement = match agent_state {
            LoadState::Loading => {
                subscreen_padded_body(empty_text("Loading…", cx)).into_any_element()
            }
            LoadState::Error(message) => {
                subscreen_padded_body(empty_text(message, cx)).into_any_element()
            }
            LoadState::Ready => render_list_body(&agents, cx).into_any_element(),
        };

        subscreen_page(
            "agent-manage",
            rgb(theme::bg_primary(cx)),
            render_list_header(cx).into_any_element(),
            body,
        )
    }
}

fn render_list_header(cx: &mut Context<AgentManage>) -> impl IntoElement {
    div()
        .id("agent-list-header")
        .min_w_0()
        .px(px(theme::SUBSCREEN_PADDING_X))
        .pt(px(theme::SPACING_XS))
        .pb(px(theme::SPACING_SM))
        .child(
            div()
                .id("agent-list-header-inner")
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
                                        .child("Manage agents"),
                                )
                                .child(
                                    div()
                                        .text_size(px(theme::FONT_BODY))
                                        .text_color(rgb(theme::text_muted(cx)))
                                        .child("View agent data, usage and sessions"),
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

fn back_button(cx: &mut Context<AgentManage>) -> Stateful<Div> {
    chevron_back_button("agent-manage-back-btn", cx, |_this, _event, window, cx| {
        platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
        window.dispatch_action(workspace_action::NavigateBack.boxed_clone(), cx);
    })
}

fn render_list_body(agents: &[AgentSummary], cx: &mut Context<AgentManage>) -> impl IntoElement {
    let mut list = div()
        .id("agent-list")
        .w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(theme::SPACING_SM));

    for agent in agents {
        let slug = agent.slug.clone();
        list = list.child(
            render_agent_card(cx, AgentCardProps { agent })
                .cursor_pointer()
                .on_press(cx.listener(move |_this, _event, window, cx| {
                    platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
                    window.dispatch_action(
                        workspace_action::OpenAgentDetail { slug: slug.clone() }.boxed_clone(),
                        cx,
                    );
                })),
        );
    }
    subscreen_padded_body(list)
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
