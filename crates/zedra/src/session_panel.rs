/// Session info panel for the workspace drawer.
///
/// Displays host info, connection details, endpoints, and disconnect button.
use gpui::*;

use crate::theme;
use crate::transport_badge::{format_bytes, render_transport_badge, transport_badge_info_phase};
use crate::workspace_drawer::{WorkspaceDrawer, WorkspaceDrawerEvent};
use zedra_session::SessionState;

/// Render the session tab content for the workspace drawer.
pub fn render_session_tab(
    session_state: Option<&SessionState>,
    cx: &mut Context<WorkspaceDrawer>,
) -> Div {
    let inner = session_state.map(|s| s.get());

    let is_empty = inner.as_ref().map(|s| s.phase.is_init()).unwrap_or(true);
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

    let inner = inner.unwrap();
    let phase = &inner.phase;
    let snap = &inner.snapshot;

    let mut info = div().px(px(theme::DRAWER_PADDING)).flex().flex_col();

    if !snap.username.is_empty() && !snap.hostname.is_empty() {
        let host = format!("{}@{}", snap.username, snap.hostname);
        info = info.child(info_row("Host", host));
    }

    if let Some(os) = snap.os_version.as_deref()
        && let Some(arch) = snap.arch.as_deref()
    {
        let platform = format!("{} / {}", os, arch,);
        info = info.child(info_row("Platform", platform));
    }

    if !snap.strip_path.is_empty() {
        info = info.child(info_row("Directory", snap.strip_path.clone()));
    }

    if let Some(alpn) = snap.alpn.clone() {
        info = info.child(info_row("Protocol", alpn));
    }

    if let Some(version) = snap.host_version.as_deref() {
        let daemon_version = format!("v{}", version);
        info = info.child(info_row("Daemon version", daemon_version));
    }

    // --- Connection badge ---
    let (badge_label, badge_color) = transport_badge_info_phase(phase, snap.transport.as_ref());
    info = info.child(
        div()
            .py(px(4.0))
            .child(
                div()
                    .text_color(rgb(theme::TEXT_MUTED))
                    .text_size(px(theme::FONT_DETAIL))
                    .child("Connection"),
            )
            .child(render_transport_badge(badge_label, badge_color)),
    );

    // --- Transport details ---
    if let Some(t) = &snap.transport {
        let remote_addr_label = format!("{} ({})", t.remote_addr, t.num_paths);
        info = info.child(info_row("Remote Address", remote_addr_label));

        info = info.child(info_row(
            "Data",
            format!(
                "{} sent / {} recv",
                format_bytes(t.bytes_sent),
                format_bytes(t.bytes_recv),
            ),
        ));
    }

    // --- Session section ---
    if let Some(sid) = &snap.session_id {
        info = info.child(info_row("Session ID", sid.clone()));
    }

    // --- Phase timing section ---
    info = info.child(render_timing(snap));

    // --- Disconnect button ---
    let disconnect_button = div()
        .id("session-disconnect-btn")
        .mt(px(8.0))
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

    info.child(disconnect_button).child(div().h(px(16.0)))
}

/// Render phase timing summary row.
fn render_timing(snap: &zedra_session::ConnectSnapshot) -> Div {
    let mut timing_parts: Vec<String> = Vec::new();
    if let Some(ms) = snap.binding_ms {
        timing_parts.push(format!("Bind {ms}ms"));
    }
    if let Some(ms) = snap.hole_punch_ms {
        timing_parts.push(format!("HolePunch {ms}ms"));
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

    muted_info_row("Timing", timing_parts.join(" \u{00b7} "))
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

fn muted_info_row(label: &'static str, value: String) -> Div {
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
                .text_color(rgb(theme::TEXT_MUTED))
                .text_size(px(theme::FONT_BODY))
                .child(value),
        )
}
