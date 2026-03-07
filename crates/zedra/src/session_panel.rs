/// Session info panel for the workspace drawer.
///
/// Displays host info, connection details, endpoints, and disconnect button.
use gpui::*;

use crate::theme;
use crate::transport_badge::format_bytes;
use crate::workspace_drawer::WorkspaceDrawerEvent;
use zedra_session::SessionState;

/// Render the session tab content for the workspace drawer.
pub fn render_session_tab(cx: &mut Context<crate::workspace_drawer::WorkspaceDrawer>) -> Div {
    let session = zedra_session::active_session();

    let Some(session) = session else {
        return div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .text_color(rgb(theme::TEXT_MUTED))
            .text_size(px(theme::FONT_BODY))
            .child("No active session");
    };

    let state = session.state();
    let latency = session.latency_ms();
    let session_id = session
        .session_id()
        .unwrap_or_else(|| "\u{2014}".to_string());
    let conn_info = session.connection_info();

    let (status_label, status_color) = match &state {
        SessionState::Connected { .. } => ("Connected", theme::ACCENT_GREEN),
        SessionState::Connecting { .. } => ("Connecting...", theme::ACCENT_YELLOW),
        SessionState::Reconnecting { .. } => ("Reconnecting...", theme::ACCENT_YELLOW),
        SessionState::Disconnected => ("Disconnected", theme::ACCENT_RED),
        SessionState::Error(_) => ("Error", theme::ACCENT_RED),
    };

    let (hostname, username, workdir, os, arch, os_version, host_version) = match &state {
        SessionState::Connected {
            hostname,
            username,
            workdir,
            os,
            arch,
            os_version,
            host_version,
        } => (
            hostname.clone(),
            username.clone(),
            workdir.clone(),
            os.clone(),
            arch.clone(),
            os_version.clone(),
            host_version.clone(),
        ),
        _ => Default::default(),
    };

    // --- Status banner ---

    let mut content = div().px(px(16.0)).pt(px(12.0)).flex().flex_col().child(
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .pb(px(10.0))
            .border_b_1()
            .border_color(rgb(theme::BORDER_SUBTLE))
            .child(
                div()
                    .w(px(theme::ICON_STATUS))
                    .h(px(theme::ICON_STATUS))
                    .rounded(px(3.0))
                    .bg(rgb(status_color)),
            )
            .child(
                div()
                    .text_color(rgb(theme::TEXT_PRIMARY))
                    .text_size(px(theme::FONT_BODY))
                    .font_weight(FontWeight::MEDIUM)
                    .child(status_label),
            ),
    );

    // --- Host section ---

    content = content
        .child(section_header("HOST"))
        .child(info_row("Hostname", hostname));
    if !username.is_empty() {
        content = content.child(info_row("User", username));
    }
    if !os.is_empty() {
        let os_label = if !arch.is_empty() {
            format!("{} / {}", os, arch)
        } else {
            os
        };
        content = content.child(info_row("Platform", os_label));
    }
    if !os_version.is_empty() {
        content = content.child(info_row("OS Version", os_version));
    }
    if !host_version.is_empty() {
        content = content.child(info_row("Host Version", host_version));
    }
    content = content.child(info_row("Directory", workdir));

    // --- Connection section ---

    if let Some(ci) = &conn_info {
        let conn_type_label = if ci.is_direct {
            "Direct (P2P)"
        } else {
            "Relayed"
        };
        let conn_type_color = if ci.is_direct {
            theme::ACCENT_GREEN
        } else {
            theme::ACCENT_YELLOW
        };

        content = content.child(section_header("CONNECTION")).child(
            div()
                .py(px(4.0))
                .child(
                    div()
                        .text_color(rgb(theme::TEXT_MUTED))
                        .text_size(px(theme::FONT_DETAIL))
                        .child("Type"),
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

        content = content
            .child(info_row("Protocol", ci.protocol.clone()))
            .child(info_row("Remote Address", ci.remote_addr.clone()));

        let rtt_ms = if ci.path_rtt_ms > 0 {
            ci.path_rtt_ms
        } else {
            latency
        };
        if rtt_ms > 0 {
            content = content.child(info_row("RTT", format!("{}ms", rtt_ms)));
        }

        content = content.child(info_row("Paths", format!("{}", ci.num_paths)));

        if ci.bytes_sent > 0 || ci.bytes_recv > 0 {
            content = content.child(info_row(
                "Data",
                format!(
                    "{} sent / {} recv",
                    format_bytes(ci.bytes_sent),
                    format_bytes(ci.bytes_recv),
                ),
            ));
        }
    } else if latency > 0 {
        content = content
            .child(section_header("CONNECTION"))
            .child(info_row("RTT", format!("{}ms", latency)));
    }

    // --- Endpoints section ---

    if let Some(ci) = &conn_info {
        content = content
            .child(section_header("ENDPOINTS"))
            .child(info_row("Local", ci.local_endpoint_id.clone()))
            .child(info_row("Remote", ci.endpoint_id.clone()));
    }

    // --- Session section ---

    content = content
        .child(section_header("SESSION"))
        .child(info_row("Session ID", session_id));

    if let SessionState::Error(msg) = &state {
        content = content.child(
            div()
                .pt(px(6.0))
                .text_color(rgb(theme::ACCENT_RED))
                .text_size(px(theme::FONT_BODY))
                .child(msg.clone()),
        );
    }

    let disconnect_button = div()
        .id("session-disconnect-btn")
        .mx(px(16.0))
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
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|_this, _event, _window, cx| {
                cx.emit(WorkspaceDrawerEvent::DisconnectRequested);
            }),
        )
        .child(div().flex().justify_center().child("Disconnect"));

    content = content.child(disconnect_button).child(div().h(px(16.0)));

    content
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
                .text_color(rgb(theme::TEXT_PRIMARY))
                .text_size(px(theme::FONT_BODY))
                .child(value),
        )
}

fn section_header(label: &'static str) -> Div {
    div()
        .pt(px(10.0))
        .pb(px(4.0))
        .border_b_1()
        .border_color(rgb(theme::BORDER_SUBTLE))
        .child(
            div()
                .text_color(rgb(theme::TEXT_SECONDARY))
                .text_size(px(theme::FONT_DETAIL))
                .font_weight(FontWeight::MEDIUM)
                .child(label),
        )
}
