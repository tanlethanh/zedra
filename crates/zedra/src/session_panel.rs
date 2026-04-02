/// Session info panel for the workspace drawer.
///
/// Displays host info, connection details, endpoints, and disconnect button.
use gpui::*;

use crate::theme;
use crate::transport_badge::{
    STALE_THRESHOLD_SECS, format_bytes, render_transport_badge, transport_badge_info_phase,
};
use crate::workspace_drawer::{WorkspaceDrawer, WorkspaceDrawerEvent};
use zedra_session::{ConnectPhase, SessionState};

/// Render the session tab content for the workspace drawer.
pub fn render_session_tab(
    session_state: Option<&SessionState>,
    cx: &mut Context<WorkspaceDrawer>,
) -> Div {
    let inner = session_state.map(|s| s.get());

    let is_empty = inner.as_ref().map(|s| s.phase.is_idle()).unwrap_or(true);
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

    // --- Host section ---
    if !snap.hostname.is_empty() {
        info = info.child(info_row("Hostname", snap.hostname.clone()));
    }
    if !snap.username.is_empty() {
        info = info.child(info_row("User", snap.username.clone()));
    }
    if let Some(os) = &snap.os {
        if !os.is_empty() {
            let os_label = match &snap.arch {
                Some(arch) if !arch.is_empty() => format!("{os} / {arch}"),
                _ => os.clone(),
            };
            info = info.child(info_row("Platform", os_label));
        }
    }
    if let Some(v) = &snap.os_version {
        if !v.is_empty() {
            info = info.child(info_row("OS Version", v.clone()));
        }
    }
    if let Some(v) = &snap.host_version {
        if !v.is_empty() {
            info = info.child(info_row("Host Version", v.clone()));
        }
    }
    if !snap.workdir.is_empty() {
        info = info.child(info_row("Directory", snap.workdir.clone()));
    }

    // --- Connection badge ---
    {
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
                .child(render_transport_badge(badge_label, badge_color, false)),
        );
    }

    // --- Transport details ---
    if let Some(t) = &snap.transport {
        info = info.child(info_row("Remote Address", t.remote_addr.clone()));

        if let Some(relay) = &t.relay_url {
            info = info.child(info_row("Relay", relay.clone()));
        }

        let stale_secs = t.last_alive_at.map(|at| at.elapsed().as_secs());
        let is_stale = stale_secs.map_or(false, |s| s >= STALE_THRESHOLD_SECS);
        let rtt_label = match stale_secs {
            Some(secs) if secs < STALE_THRESHOLD_SECS => format!("{}ms", t.rtt_ms),
            Some(secs) => format!("stale {}s", secs),
            None => "\u{2014}".into(),
        };
        let rtt_color = if is_stale {
            theme::ACCENT_YELLOW
        } else {
            theme::TEXT_SECONDARY
        };
        info = info.child(
            div()
                .py(px(4.0))
                .child(
                    div()
                        .text_color(rgb(theme::TEXT_MUTED))
                        .text_size(px(theme::FONT_DETAIL))
                        .child("RTT"),
                )
                .child(
                    div()
                        .mt(px(1.0))
                        .text_color(rgb(rtt_color))
                        .text_size(px(theme::FONT_BODY))
                        .child(rtt_label),
                ),
        );

        info = info.child(info_row("Paths", format!("{}", t.num_paths)));

        if t.bytes_sent > 0 || t.bytes_recv > 0 {
            info = info.child(info_row(
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
        info = info.child(info_row("Local Node", id.clone()));
    }
    if let Some(id) = &snap.remote_node_id {
        info = info.child(info_row("Remote Node", id.clone()));
    }
    if let Some(alpn) = &snap.alpn {
        info = info.child(info_row("Protocol", alpn.clone()));
    }

    // --- Session section ---
    if let Some(sid) = &snap.session_id {
        info = info.child(info_row("Session ID", sid.clone()));
    }

    // --- Phase timing section ---
    info = info.child(render_timing(snap));

    // --- Error banner (failed phase only) ---
    if let ConnectPhase::Failed(e) = phase {
        info = info.child(
            div()
                .mt(px(8.0))
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

    // --- Reconnecting status row (only when not connected) ---
    if !phase.is_connected() {
        info = info.child(render_reconnecting_row(phase, snap, cx));
    }

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

/// "Connecting" label row shown when not connected.
fn render_reconnecting_row(
    phase: &ConnectPhase,
    snap: &zedra_session::ConnectSnapshot,
    cx: &mut Context<WorkspaceDrawer>,
) -> Div {
    let (label, color) = transport_badge_info_phase(phase, snap.transport.as_ref());
    div()
        .py(px(4.0))
        .cursor_pointer()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|_this, _event, _window, cx| {
                cx.emit(WorkspaceDrawerEvent::ShowConnectingOverlay);
            }),
        )
        .child(
            div()
                .text_color(rgb(theme::TEXT_MUTED))
                .text_size(px(theme::FONT_DETAIL))
                .child("Connecting"),
        )
        .child(
            div()
                .mt(px(1.0))
                .text_color(rgb(color))
                .text_size(px(theme::FONT_BODY))
                .child(label),
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
