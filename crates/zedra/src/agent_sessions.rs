use gpui::*;
use std::rc::Rc;
use tracing::error;
use zedra_session::SessionHandle;

use crate::agent_ui::{
    AgentSessionRow, flatten_session_sections, group_sessions_by_day, new_session_list_state,
    render_virtualized_agent_session_list, reset_session_list_state,
};
use crate::fonts;
use crate::platform_bridge::{self, HapticFeedback};
use crate::theme;
use crate::ui::{
    chevron_back_button, subscreen_empty_text, subscreen_padded_body, subscreen_page_unscrolled,
    subscreen_refresh_button,
};
use crate::workspace_action;

#[derive(Clone, Debug)]
enum LoadState {
    Loading,
    Ready,
    Error(String),
}

pub struct AgentSessions {
    session_handle: SessionHandle,
    rows: Rc<Vec<AgentSessionRow>>,
    list_state: ListState,
    load_state: LoadState,
    loading_epoch: u64,
    _tasks: Vec<Task<()>>,
}

impl AgentSessions {
    pub fn new(session_handle: SessionHandle, cx: &mut Context<Self>) -> Self {
        let mut view = Self {
            session_handle,
            rows: Rc::new(Vec::new()),
            list_state: new_session_list_state(0),
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
        self.set_rows(Vec::new());
        self.load_state = LoadState::Loading;
        cx.notify();

        let handle = self.session_handle.clone();
        let task = cx.spawn(async move |this, cx| {
            let mut sessions = Vec::new();
            let mut errors = Vec::new();
            match handle.agent_list(refresh).await {
                Ok(agents) => {
                    // Fan out per-agent scans so one slow agent doesn't gate the rest.
                    // Only detail-bearing agents have session lists; skip the rest.
                    let agents = agents.into_iter().filter(|agent| agent.shows_detail);
                    let results = futures::future::join_all(agents.map(|agent| {
                        let handle = handle.clone();
                        async move {
                            let slug = agent.slug;
                            let result = handle.agent_sessions(slug.clone(), refresh, 0).await;
                            (slug, result)
                        }
                    }))
                    .await;
                    for (slug, result) in results {
                        match result {
                            Ok(mut rows) => sessions.append(&mut rows),
                            Err(err) => errors.push(format!("{slug}: {err}")),
                        }
                    }
                }
                Err(err) => errors.push(err.to_string()),
            }
            let _ = this.update(cx, |this, cx| {
                if this.loading_epoch != epoch {
                    return;
                }
                this.set_rows(flatten_session_sections(group_sessions_by_day(sessions)));
                this.load_state = if errors.is_empty() {
                    LoadState::Ready
                } else if this.rows.is_empty() {
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

    /// `ListState` caches row measurements, so it must be reset whenever the
    /// row set changes or heights carry over from the previous load.
    fn set_rows(&mut self, rows: Vec<AgentSessionRow>) {
        reset_session_list_state(&self.list_state, rows.len());
        self.rows = Rc::new(rows);
    }
}

impl Render for AgentSessions {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body: AnyElement = match &self.load_state {
            LoadState::Loading => {
                subscreen_padded_body(subscreen_empty_text("Loading…", cx)).into_any_element()
            }
            LoadState::Error(message) => {
                subscreen_padded_body(subscreen_empty_text(message.clone(), cx)).into_any_element()
            }
            LoadState::Ready if self.rows.is_empty() => subscreen_padded_body(
                subscreen_empty_text("No sessions found for this workspace.", cx),
            )
            .into_any_element(),
            LoadState::Ready => render_virtualized_agent_session_list(
                Rc::clone(&self.rows),
                self.list_state.clone(),
                true,
            )
            .into_any_element(),
        };
        let header = render_session_header(cx).into_any_element();
        subscreen_page_unscrolled("agent-sessions", rgb(theme::bg_primary(cx)), header, body)
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
