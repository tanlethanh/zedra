use gpui::prelude::FluentBuilder;
use gpui::*;
use tracing::error;
use zedra_rpc::proto::{AgentFile, AgentSummary, HostEvent, ManagedAgentKind};
use zedra_session::{Session, SessionHandle};

use crate::agent_ui::{
    AgentSessionListProps, cli_version_display, group_sessions_by_day, managed_agent_name,
    render_agent_session_list, render_agent_usage_row, setup_label,
};
use crate::file_preview_view::FilePreviewView;
use crate::fonts;
use crate::platform_bridge::{self, CustomSheetDetent, CustomSheetOptions, HapticFeedback};
use crate::telemetry::view_telemetry;
use crate::theme;
use crate::ui::{
    chevron_back_button, subscreen_padded_body, subscreen_page, subscreen_refresh_button,
};
use crate::workspace_action;
use crate::workspace_state::WorkspaceState;

#[derive(Clone, Debug, PartialEq)]
enum LoadState {
    Loading,
    Ready,
    Error(String),
}

pub struct AgentDetail {
    session_handle: SessionHandle,
    kind: ManagedAgentKind,
    agent: Option<AgentSummary>,
    sessions: Vec<zedra_rpc::proto::AgentSessionSummary>,
    /// Read-only config/memory files (Hermes). Empty for agents without a set.
    files: Vec<AgentFile>,
    /// Persistent preview for the native file sheet; its content swaps per tap.
    file_preview: Entity<FilePreviewView>,
    agent_state: LoadState,
    session_state: LoadState,
    loading_epoch: u64,
    _tasks: Vec<Task<()>>,
}

impl AgentDetail {
    pub fn new(
        session_handle: SessionHandle,
        session: Session,
        kind: ManagedAgentKind,
        workspace_state: Entity<WorkspaceState>,
        cx: &mut Context<Self>,
    ) -> Self {
        let file_preview =
            cx.new(|cx| FilePreviewView::new(session_handle.clone(), workspace_state, cx));
        let mut view = Self {
            session_handle,
            kind,
            agent: None,
            sessions: Vec::new(),
            files: Vec::new(),
            file_preview,
            agent_state: LoadState::Loading,
            session_state: LoadState::Loading,
            loading_epoch: 0,
            _tasks: Vec::new(),
        };
        view.subscribe_agent_info(session, cx);
        view.reload(false, cx);
        view
    }

    fn subscribe_agent_info(&mut self, session: Session, cx: &mut Context<Self>) {
        let kind = self.kind;
        let mut host_event_rx = session.subscribe_host_events();
        let task = cx.spawn(async move |this, cx| {
            loop {
                match host_event_rx.recv().await {
                    Ok(HostEvent::AgentInfoChanged { info }) if info.kind == kind => {
                        let should_break = this
                            .update(cx, |this, cx| {
                                this.agent = Some(info);
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
                        tracing::warn!("agent detail host event listener lagged by {}", skipped);
                        let should_break =
                            this.update(cx, |this, cx| this.reload(false, cx)).is_err();
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

    fn reload(&mut self, refresh: bool, cx: &mut Context<Self>) {
        self.loading_epoch = self.loading_epoch.wrapping_add(1);
        let epoch = self.loading_epoch;
        self.agent_state = LoadState::Loading;
        self.session_state = LoadState::Loading;
        cx.notify();

        let handle = self.session_handle.clone();
        let kind = self.kind;
        let task = cx.spawn(async move |this, cx| {
            let (agents, sessions, files) = tokio::join!(
                handle.agent_list(refresh),
                handle.agent_sessions(kind, refresh, 0),
                handle.agent_files(kind),
            );
            let _ = this.update(cx, |this, cx| {
                if this.loading_epoch != epoch {
                    return;
                }
                // Files are a best-effort extra; on RPC/protocol failure keep
                // the prior list rather than blanking the section (absent files
                // already come back as explicit `missing` rows, so an error is
                // distinct from "no files").
                if let Ok(files) = files {
                    this.files = files;
                }
                match agents {
                    Ok(agents) => {
                        this.agent = agents.into_iter().find(|agent| agent.kind == kind);
                        this.agent_state = if this.agent.is_some() {
                            LoadState::Ready
                        } else {
                            LoadState::Error("Agent not found on host.".into())
                        };
                    }
                    Err(err) => {
                        error!(?kind, "agent detail agent list failed: {}", err);
                        this.agent = None;
                        this.agent_state = LoadState::Error(err.to_string());
                    }
                }
                match sessions {
                    Ok(sessions) => {
                        this.sessions = sessions;
                        this.session_state = LoadState::Ready;
                    }
                    Err(err) => {
                        error!(?kind, "agent detail sessions failed: {}", err);
                        this.sessions.clear();
                        this.session_state = LoadState::Error(err.to_string());
                    }
                }
                cx.notify();
            });
        });
        self._tasks.push(task);
    }

    fn header_title(&self) -> String {
        self.agent
            .as_ref()
            .map(|agent| agent.display_name.clone())
            .unwrap_or_else(|| managed_agent_name(self.kind).to_string())
    }

    fn header_description(&self) -> String {
        match (&self.agent_state, self.agent.as_ref()) {
            (LoadState::Ready, Some(agent)) => cli_version_display(agent),
            (LoadState::Loading, _) => "Loading…".into(),
            (LoadState::Error(message), _) => message.clone(),
            _ => String::new(),
        }
    }

    /// Read-only config/memory file list. Each present file is a tappable row
    /// that opens its content in a native sheet (content is not rendered inline).
    fn render_files_section(
        &self,
        files: &[AgentFile],
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut section = div()
            .id("agent-detail-files")
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(theme::SPACING_XS))
            .child(
                div()
                    .text_size(px(theme::FONT_DETAIL))
                    .text_color(rgb(theme::text_muted(cx)))
                    .child("Config & memory (read-only)"),
            );
        for (index, file) in files.iter().enumerate() {
            section = section.child(self.file_row(index, file, cx));
        }
        section
    }

    fn file_row(&self, index: usize, file: &AgentFile, cx: &mut Context<Self>) -> Stateful<Div> {
        let subtitle = if file.missing {
            "not created yet".to_string()
        } else if file.truncated {
            format!("{} · truncated", file.path)
        } else {
            file.path.clone()
        };
        let mut row = div()
            .id(SharedString::from(format!("agent-file-row-{}", file.label)))
            .min_w_0()
            .flex()
            .flex_row()
            .items_baseline()
            .gap(px(theme::SPACING_SM))
            .py(px(theme::AGENT_METADATA_ROW_PY))
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(theme::FONT_DETAIL))
                    .font_family(fonts::MONO_FONT_FAMILY)
                    .text_color(rgb(theme::text_secondary(cx)))
                    .child(file.label.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(theme::FONT_DETAIL))
                    .text_color(rgb(theme::text_muted(cx)))
                    .child(subtitle),
            );
        // Only present files open a sheet; missing ones are inert. Clone the
        // (potentially large) file content only on tap, not every render.
        if !file.missing {
            row = row
                .cursor_pointer()
                .on_press(cx.listener(move |this, _event, _window, cx| {
                    platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
                    if let Some(file) = this.files.get(index).cloned() {
                        this.open_file_preview(file, cx);
                    }
                }));
        }
        row
    }

    fn open_file_preview(&self, file: AgentFile, cx: &mut Context<Self>) {
        let subtitle = if file.truncated {
            format!("{} · truncated", file.path)
        } else {
            file.path.clone()
        };
        view_telemetry::record(view_telemetry::custom_sheet_file(&file.path));
        self.file_preview.update(cx, |preview, cx| {
            preview.open_content(file.label.clone(), subtitle, &file.path, file.content, cx);
        });
        platform_bridge::show_custom_sheet(
            CustomSheetOptions {
                detents: vec![CustomSheetDetent::Large],
                initial_detent: CustomSheetDetent::Large,
                shows_grabber: true,
                expands_on_scroll_edge: true,
                edge_attached_in_compact_height: false,
                width_follows_preferred_content_size_when_edge_attached: false,
                corner_radius: None,
                modal_in_presentation: false,
            },
            self.file_preview.clone(),
        );
    }
}

impl Render for AgentDetail {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let agent = self.agent.clone();
        let sessions = self.sessions.clone();
        let agent_state = self.agent_state.clone();
        let session_state = self.session_state.clone();
        let title = self.header_title();
        let description = self.header_description();

        let body: AnyElement = match (&agent_state, agent.as_ref()) {
            (LoadState::Loading, _) => {
                subscreen_padded_body(empty_text("Loading…", cx)).into_any_element()
            }
            (LoadState::Error(message), _) => {
                subscreen_padded_body(empty_text(message.clone(), cx)).into_any_element()
            }
            (LoadState::Ready, Some(agent)) => {
                let (sessions_loading, sessions_error) = match &session_state {
                    LoadState::Loading => (true, None),
                    LoadState::Error(message) => (false, Some(message.clone())),
                    LoadState::Ready => (false, None),
                };
                subscreen_padded_body(
                    div()
                        .w_full()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(theme::SPACING_MD))
                        .child(render_detail_metadata(agent, cx))
                        .when_some(agent.usage.as_ref(), |col, usage| {
                            col.child(render_agent_usage_row(agent.kind, usage, cx))
                        })
                        .when(!self.files.is_empty(), |col| {
                            col.child(self.render_files_section(&self.files, cx))
                        })
                        .child(render_agent_session_list(
                            AgentSessionListProps {
                                sections: group_sessions_by_day(sessions),
                                loading: sessions_loading,
                                error: sessions_error,
                                empty_message: "No sessions found for this agent.",
                                resume_on_tap: true,
                                scroll_container: false,
                                horizontal_padding: false,
                            },
                            cx,
                        )),
                )
                .into_any_element()
            }
            (LoadState::Ready, None) => {
                subscreen_padded_body(empty_text("Agent not found on host.", cx)).into_any_element()
            }
        };

        subscreen_page(
            "agent-detail",
            rgb(theme::bg_primary(cx)),
            render_detail_header(title, description, cx).into_any_element(),
            body,
        )
    }
}

fn render_detail_header(
    title: String,
    description: String,
    cx: &mut Context<AgentDetail>,
) -> impl IntoElement {
    div()
        .id("agent-detail-header")
        .min_w_0()
        .px(px(theme::SUBSCREEN_PADDING_X))
        .pt(px(theme::SPACING_XS))
        .pb(px(theme::SPACING_SM))
        .child(
            div()
                .id("agent-detail-header-inner")
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
                                        .child(title),
                                )
                                .child(
                                    div()
                                        .text_size(px(theme::FONT_BODY))
                                        .text_color(rgb(theme::text_muted(cx)))
                                        .child(description),
                                ),
                        ),
                )
                .child(subscreen_refresh_button(
                    "agent-detail-refresh-btn",
                    cx,
                    |this, _event, _window, cx| this.reload(true, cx),
                )),
        )
}

fn back_button(cx: &mut Context<AgentDetail>) -> Stateful<Div> {
    chevron_back_button("agent-detail-back-btn", cx, |_this, _event, window, cx| {
        platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
        window.dispatch_action(workspace_action::NavigateBack.boxed_clone(), cx);
    })
}

fn render_detail_metadata(agent: &AgentSummary, cx: &App) -> impl IntoElement {
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
        .id("agent-detail-metadata")
        .w_full()
        .min_w_0()
        .pb(px(theme::SPACING_XS))
        .border_b_1()
        .border_color(rgb(theme::border_subtle(cx)))
        .flex()
        .flex_col();

    for (index, (label, value)) in rows.into_iter().enumerate() {
        summary = summary.child(metadata_line(label, value, cx, index < last));
    }
    summary
}

fn metadata_line(label: SharedString, value: String, cx: &App, show_divider: bool) -> Div {
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

/// Read-only viewer for an agent's config/memory files (Hermes:
/// SOUL.md/USER.md/MEMORY.md/config.yaml/.env). Content is host-capped; long
/// files scroll within a bounded box.
/// Body of the native file sheet: a mono, horizontally- and vertically-
/// scrollable view of one file's content (GPUI text runs don't break on `\n`,
/// so each line is its own element).
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
