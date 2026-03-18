/// Session info panel for the workspace drawer.
///
/// Displays host info, connection details, endpoints, and disconnect button.
use gpui::*;

use crate::theme;
use crate::transport_badge::format_bytes;
use crate::workspace_drawer::{WorkspaceDrawer, WorkspaceDrawerEvent};
use zedra_session::{ConnectPhase, ConnectState};

/// Render the session tab content for the workspace drawer.
pub fn render_session_tab(
    handle: Option<&zedra_session::SessionHandle>,
    cx: &mut Context<WorkspaceDrawer>,
) -> Div {
    let cs: Option<ConnectState> = handle.map(|h| h.connect_state());

    let is_empty = cs
        .as_ref()
        .map(|s| s.phase.is_idle())
        .unwrap_or(true);
    if is_empty {
        return div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .text_color(rgb(theme::TEXT_MUTED))
            .text_size(px(theme::FONT_BODY))
            .child("No active session");
    }

    let cs = cs.unwrap();
    let snap = &cs.snapshot;

    let mut content = div()
        .px(px(theme::DRAWER_PADDING))
        .flex()
        .flex_col();

    // --- Host section ---
    if let Some(hostname) = &snap.hostname {
        content = content.child(info_row("Hostname", hostname.clone()));
    }
    if let Some(username) = &snap.username {
        if !username.is_empty() {
            content = content.child(info_row("User", username.clone()));
        }
    }
    if let Some(os) = &snap.os {
        if !os.is_empty() {
            let os_label = match &snap.arch {
                Some(arch) if !arch.is_empty() => format!("{os} / {arch}"),
                _ => os.clone(),
            };
            content = content.child(info_row("Platform", os_label));
        }
    }
    if let Some(v) = &snap.os_version {
        if !v.is_empty() {
            content = content.child(info_row("OS Version", v.clone()));
        }
    }
    if let Some(v) = &snap.host_version {
        if !v.is_empty() {
            content = content.child(info_row("Host Version", v.clone()));
        }
    }
    if let Some(wd) = &snap.workdir {
        content = content.child(info_row("Directory", wd.clone()));
    }

    // --- Connection section ---
    if let Some(t) = &snap.transport {
        let conn_type_label = if t.is_direct { "Direct (P2P)" } else { "Relayed" };
        let conn_type_color = if t.is_direct {
            theme::ACCENT_GREEN
        } else {
            theme::ACCENT_YELLOW
        };

        content = content.child(
            div()
                .py(px(4.0))
                .child(
                    div()
                        .text_color(rgb(theme::TEXT_MUTED))
                        .text_size(px(theme::FONT_DETAIL))
                        .child("Connection"),
                )
                .child(
                    div()
                        .mt(px(1.0))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(4.0))
                        .child(
                            div()
                                .w(px(theme::ICON_STATUS))
                                .h(px(theme::ICON_STATUS))
                                .rounded(px(3.0))
                                .bg(rgb(conn_type_color)),
                        )
                        .child(
                            div()
                                .text_color(rgb(conn_type_color))
                                .text_size(px(theme::FONT_BODY))
                                .child(conn_type_label),
                        ),
                ),
        );

        content = content.child(info_row("Remote Address", t.remote_addr.clone()));

        if let Some(relay) = &t.relay_url {
            content = content.child(info_row("Relay", relay.clone()));
        }

        if t.rtt_ms > 0 {
            content = content.child(info_row("RTT", format!("{}ms", t.rtt_ms)));
        }

        content = content.child(info_row("Paths", format!("{}", t.num_paths)));

        if t.bytes_sent > 0 || t.bytes_recv > 0 {
            content = content.child(info_row(
                "Data",
                format!(
                    "{} sent / {} recv",
                    format_bytes(t.bytes_sent),
                    format_bytes(t.bytes_recv),
                ),
            ));
        }
    }

    // --- Endpoints section ---
    if let Some(id) = &snap.local_node_id {
        content = content.child(info_row("Local Node", id.clone()));
    }
    if let Some(id) = &snap.remote_node_id {
        content = content.child(info_row("Remote Node", id.clone()));
    }
    if let Some(alpn) = &snap.alpn {
        content = content.child(info_row("Protocol", alpn.clone()));
    }

    // --- Session section ---
    if let Some(sid) = &snap.session_id {
        content = content.child(info_row("Session ID", sid.clone()));
    }

    // --- Phase timing section ---
    content = content.child(render_timing(snap));

    // --- Error banner ---
    if let ConnectPhase::Failed(e) = &cs.phase {
        content = content.child(
            div()
                .mt(px(8.0))
                .p(px(8.0))
                .rounded(px(6.0))
                .border_1()
                .border_color(rgb(theme::ACCENT_RED))
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(
                    div()
                        .text_color(rgb(theme::ACCENT_RED))
                        .text_size(px(theme::FONT_BODY))
                        .child(e.user_message()),
                )
                .children(e.action_hint().map(|hint| {
                    div()
                        .text_color(rgb(theme::TEXT_MUTED))
                        .text_size(px(theme::FONT_DETAIL))
                        .child(hint)
                })),
        );
    }

    // --- Reconnecting banner ---
    if let ConnectPhase::Reconnecting {
        attempt,
        next_retry_secs,
        ..
    } = &cs.phase
    {
        let msg = if *next_retry_secs > 0 {
            format!("Reconnecting (attempt {attempt}) — {next_retry_secs}s until next retry")
        } else {
            format!("Reconnecting (attempt {attempt})\u{2026}")
        };
        content = content.child(
            div()
                .mt(px(8.0))
                .text_color(rgb(theme::ACCENT_YELLOW))
                .text_size(px(theme::FONT_DETAIL))
                .child(msg),
        );
    }

    // --- Disconnect button ---
    let disconnect_button = div()
        .id("session-disconnect-btn")
        .mt(px(16.0))
        .px(px(12.0))
        .py(px(8.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(rgb(theme::ACCENT_RED))
        .text_color(rgb(theme::ACCENT_RED))
        .text_size(px(theme::FONT_BODY))
        .cursor_pointer()
        .hover(|s| s.bg(gpui::hsla(0.0, 0.6, 0.5, 0.1)))
        .on_click(cx.listener(|_this, _event, _window, cx| {
            cx.emit(WorkspaceDrawerEvent::DisconnectRequested);
        }))
        .child(div().flex().justify_center().child("Disconnect"));

    content.child(disconnect_button).child(div().h(px(16.0)))
}

/// Render phase timing summary row.
fn render_timing(snap: &zedra_session::ConnectSnapshot) -> Div {
    let mut timing_parts: Vec<String> = Vec::new();
    if let Some(ms) = snap.binding_ms {
        timing_parts.push(format!("Bind {ms}ms"));
    }
    if let Some(ms) = snap.hole_punch_ms {
        timing_parts.push(format!("P2P {ms}ms"));
    }
    if let Some(ms) = snap.rpc_ms {
        timing_parts.push(format!("RPC {ms}ms"));
    }
    if let Some(ms) = snap.register_ms {
        timing_parts.push(format!("Reg {ms}ms"));
    }
    if let Some(ms) = snap.auth_ms {
        timing_parts.push(format!("Auth {ms}ms"));
    }
    if let Some(ms) = snap.fetch_ms {
        timing_parts.push(format!("Info {ms}ms"));
    }
    if let Some(ms) = snap.resume_ms {
        timing_parts.push(format!("Resume {ms}ms"));
    }
    if timing_parts.is_empty() {
        return div();
    }
    div()
        .py(px(4.0))
        .child(
            div()
                .text_color(rgb(theme::TEXT_MUTED))
                .text_size(px(theme::FONT_DETAIL))
                .child("Timing"),
        )
        .child(
            div()
                .mt(px(1.0))
                .text_color(rgb(theme::TEXT_SECONDARY))
                .text_size(px(theme::FONT_DETAIL))
                .child(timing_parts.join(" \u{00b7} ")),
        )
}

fn info_row(label: &'static str, value: String) -> Div {
    div()
        .py(px(4.0))
        .child(
            div()
                .text_color(rgb(theme::TEXT_MUTED))
                .text_size(px(theme::FONT_DETAIL))
                .child(label),
        )
        .child(
            div()
                .mt(px(1.0))
                .text_color(rgb(theme::TEXT_SECONDARY))
                .text_size(px(theme::FONT_BODY))
                .child(value),
        )
}
