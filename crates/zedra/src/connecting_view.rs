/// Connection loading screen — shown while a workspace is connecting/reconnecting/failed.
///
/// Layout:
///   1. Horizontal 5-step progress stepper
///   2. Vertical current-phase detail (transport, auth, host, timing, error/reconnect banners)
use gpui::{prelude::FluentBuilder as _, *};
use zedra_session::{ConnectPhase, ConnectSnapshot, SessionState, TransportSnapshot};

use crate::platform_bridge::{self, AlertButton};
use crate::theme;
use crate::transport_badge::{format_bytes, render_transport_badge, transport_badge_info_phase};

// ─── Public view ─────────────────────────────────────────────────────────────

pub struct ConnectingView {
    session_state: SessionState,
    details_expanded: bool,
}

impl ConnectingView {
    pub fn new(session_state: SessionState) -> Self {
        Self {
            session_state,
            details_expanded: false,
        }
    }
}

impl Render for ConnectingView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let inner = self.session_state.get();
        let expanded = self.details_expanded;
        div()
            .id("connecting-view")
            .size_full()
            .bg(rgb(theme::BG_PRIMARY))
            .flex()
            .flex_col()
            .items_center()
            .justify_start()
            .pt(px(32.0))
            .child(render_phase_title(&inner.phase, &inner.snapshot))
            .child(render_details_toggle(expanded, cx))
            .when(expanded, |d| {
                d.child(render_detail(&inner.phase, &inner.snapshot))
            })
    }
}

// ─── Details toggle ─────────────────────────────────────────────────────────

fn render_details_toggle(expanded: bool, cx: &mut Context<ConnectingView>) -> Stateful<Div> {
    let label = if expanded {
        "Hide Details"
    } else {
        "View Details"
    };
    let chevron: SharedString = if expanded {
        "icons/chevron-up.svg".into()
    } else {
        "icons/chevron-down.svg".into()
    };

    div()
        .id("details-toggle")
        .cursor_pointer()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(4.0))
        .mb(px(theme::SPACING_SM))
        .on_click(cx.listener(|this, _event, _window, _cx| {
            this.details_expanded = !this.details_expanded;
        }))
        .child(
            div()
                .text_size(px(theme::FONT_DETAIL))
                .text_color(rgb(theme::TEXT_MUTED))
                .child(label),
        )
        .child(
            svg()
                .path(chevron)
                .size(px(12.0))
                .text_color(rgb(theme::TEXT_MUTED)),
        )
}

// ─── Phase title ─────────────────────────────────────────────────────────────

fn render_phase_title(phase: &ConnectPhase, snap: &ConnectSnapshot) -> Div {
    let (label, color) = transport_badge_info_phase(phase, snap.transport.as_ref());

    let title = match phase {
        ConnectPhase::BindingEndpoint | ConnectPhase::HolePunching => "Connect",
        ConnectPhase::Authenticating | ConnectPhase::Proving => "Authorize",
        ConnectPhase::Sync => "Sync",
        p => p.display_name(),
    };

    div()
        .mb(px(theme::SPACING_LG))
        .flex()
        .flex_col()
        .items_center()
        .gap(px(8.0))
        .child(
            div().flex().flex_row().items_center().gap(px(8.0)).child(
                div()
                    .w(px(160.0))
                    .min_w_0()
                    .truncate()
                    .text_align(TextAlign::Center)
                    .text_color(rgb(theme::TEXT_PRIMARY))
                    .text_size(px(theme::FONT_HEADING))
                    .font_weight(FontWeight::MEDIUM)
                    .child(title),
            ),
        )
        .child(render_transport_badge(label, color))
}

// ─── Phase status helpers ────────────────────────────────────────────────────

fn has_discovery_data(snap: &ConnectSnapshot) -> bool {
    snap.relay_connected
        || !snap.direct_addrs.is_empty()
        || snap.has_ipv4
        || snap.has_ipv6
        || snap.relay_latency_ms.is_some()
}

fn render_discovery_rows(snap: &ConnectSnapshot) -> Div {
    let mut d = div().flex().flex_col().gap(px(2.0));

    let relay_status = if snap.relay_connected {
        match snap.relay_latency_ms {
            Some(ms) => format!("Connected ({ms}ms)"),
            None => "Connected".into(),
        }
    } else {
        "Connecting\u{2026}".into()
    };
    d = d.child(kv_row("Relay", &relay_status));

    if !snap.direct_addrs.is_empty() {
        let count = snap.direct_addrs.len();
        let direct_addrs = snap.direct_addrs.clone();
        let direct_label = format!(
            "{count} addr{} - tap to view",
            if count == 1 { "" } else { "s" }
        );
        d = d.child(
            div()
                .flex()
                .flex_row()
                .gap(px(6.0))
                .cursor_pointer()
                .hover(|style| style.bg(theme::hover_bg()))
                .on_mouse_down(MouseButton::Left, move |_, _, _| {
                    if direct_addrs.is_empty() {
                        return;
                    }

                    let message = direct_addrs.join("\n");
                    platform_bridge::show_alert(
                        "Direct addresses",
                        &message,
                        vec![AlertButton::default("OK")],
                        |_| {},
                    );
                })
                .child(
                    div()
                        .w(px(60.0))
                        .flex_shrink_0()
                        .text_color(rgb(theme::TEXT_MUTED))
                        .text_size(px(theme::FONT_DETAIL))
                        .child("Direct"),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(rgb(theme::TEXT_SECONDARY))
                        .text_size(px(theme::FONT_DETAIL))
                        .child(direct_label),
                ),
        );
    }

    let ip_status = match (snap.has_ipv4, snap.has_ipv6) {
        (true, true) => "IPv4 + IPv6",
        (true, false) => "IPv4 only",
        (false, true) => "IPv6 only",
        (false, false) => "probing\u{2026}",
    };
    d = d.child(kv_row("UDP", ip_status));

    if let Some(varies) = snap.mapping_varies {
        let nat = if varies {
            "Symmetric (hard NAT)"
        } else {
            "Cone / direct"
        };
        d = d.child(kv_row("NAT", nat));
    }

    if snap.captive_portal == Some(true) {
        d = d.child(kv_row("Portal", "Captive portal detected"));
    }

    d
}

// ─── Vertical detail panel ───────────────────────────────────────────────────

fn render_detail(phase: &ConnectPhase, snap: &ConnectSnapshot) -> Div {
    let mut col = div()
        .w(px(theme::CONNECT_DETAIL_WIDTH))
        .flex()
        .flex_col()
        .gap(px(theme::SPACING_SM));

    if snap.local_node_id.is_some() || snap.remote_node_id.is_some() || snap.relay_url.is_some() {
        col = col.child(render_section("Endpoint", render_endpoint_rows(snap)));
    }

    if has_discovery_data(snap) {
        col = col.child(render_section("Discovery", render_discovery_rows(snap)));
    }

    if let Some(t) = &snap.transport {
        col = col.child(render_section("Transport", render_transport_rows(t)));
    }

    if snap.session_id.is_some() || snap.auth_outcome.is_some() {
        col = col.child(render_section("Auth", render_auth_rows(snap)));
    }

    if !snap.hostname.is_empty() {
        col = col.child(render_section("Host", render_host_rows(snap)));
    }

    let timing = build_timing_string(snap);
    if !timing.is_empty() {
        col = col.child(
            div()
                .text_color(rgb(theme::TEXT_MUTED))
                .text_size(px(theme::FONT_DETAIL))
                .child(timing),
        );
    }

    // Show phase-specific info
    if let ConnectPhase::Reconnecting {
        attempt,
        next_retry_secs,
        ..
    } = phase
    {
        col = col.child(
            div()
                .text_color(rgb(theme::TEXT_MUTED))
                .text_size(px(theme::FONT_DETAIL))
                .child(format!(
                    "Attempt {} · retry in {}s",
                    attempt, next_retry_secs
                )),
        );
    }

    col
}

fn render_section(title: &'static str, rows: Div) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(
            div()
                .text_color(rgb(theme::TEXT_MUTED))
                .text_size(px(theme::FONT_DETAIL))
                .mb(px(2.0))
                .child(title),
        )
        .child(rows)
}

fn render_endpoint_rows(snap: &ConnectSnapshot) -> Div {
    let mut d = div().flex().flex_col().gap(px(2.0));
    if let Some(id) = &snap.local_node_id {
        d = d.child(kv_row("Local", id));
    }
    if let Some(id) = &snap.remote_node_id {
        d = d.child(kv_row("Remote", id));
    }
    if let Some(relay) = &snap.relay_url {
        d = d.child(kv_row("Relay", relay));
    }
    if let Some(alpn) = &snap.alpn {
        d = d.child(kv_row("ALPN", alpn));
    }
    d
}

fn render_transport_rows(t: &TransportSnapshot) -> Div {
    let conn_type = if t.is_direct {
        match &t.network_hint {
            Some(h) => format!("P2P \u{00b7} {}", h.label()),
            None => "P2P".into(),
        }
    } else {
        "Relayed".into()
    };

    let alive = match t.last_alive_at {
        Some(at) => {
            let secs = at.elapsed().as_secs();
            if secs == 0 {
                "now".into()
            } else {
                format!("{secs}s ago")
            }
        }
        None => "\u{2014}".into(),
    };

    let mut d = div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(kv_row("Type", &conn_type))
        .child(kv_row(
            "Address",
            &format!("{} ({})", t.remote_addr, t.num_paths),
        ));

    if let Some(relay) = &t.relay_url {
        d = d.child(kv_row("Relay", relay));
    }
    let net = format!(
        "{}ms - {} \u{2191} / {} \u{2193}",
        t.rtt_ms,
        format_bytes(t.bytes_sent),
        format_bytes(t.bytes_recv)
    );
    d = d.child(kv_row("Net", &net));
    d = d.child(kv_row("Alive", &alive));
    d
}

fn render_auth_rows(snap: &ConnectSnapshot) -> Div {
    let mut d = div().flex().flex_col().gap(px(2.0));
    if let Some(sid) = &snap.session_id {
        d = d.child(kv_row("Session", sid));
    }
    if let Some(outcome) = &snap.auth_outcome {
        let label = match outcome {
            zedra_session::AuthOutcome::Registered => "Registered (first pairing)",
            zedra_session::AuthOutcome::Authenticated => "Authorized",
        };
        d = d.child(kv_row("Status", label));
    }
    d
}

fn render_host_rows(snap: &ConnectSnapshot) -> Div {
    let mut d = div().flex().flex_col().gap(px(2.0));
    if !snap.hostname.is_empty() {
        d = d.child(kv_row("Host", &snap.hostname));
    }
    if !snap.username.is_empty() {
        d = d.child(kv_row("User", &snap.username));
    }
    if !snap.workdir.is_empty() {
        d = d.child(kv_row("Workdir", &snap.workdir));
    }
    if let Some(os) = &snap.os {
        let label = match &snap.arch {
            Some(arch) if !arch.is_empty() => format!("{os} / {arch}"),
            _ => os.clone(),
        };
        d = d.child(kv_row("OS", &label));
    }
    if let Some(v) = &snap.host_version {
        if !v.is_empty() {
            d = d.child(kv_row("Version", v));
        }
    }
    d
}

fn build_timing_string(snap: &ConnectSnapshot) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(ms) = snap.binding_ms {
        parts.push(format!("Bind {ms}ms"));
    }
    if let Some(ms) = snap.hole_punch_ms {
        parts.push(format!("HolePunch {ms}ms"));
    }
    if let Some(ms) = snap.rpc_ms {
        parts.push(format!("RPC {ms}ms"));
    }
    if let Some(ms) = snap.register_ms {
        parts.push(format!("Reg {ms}ms"));
    }
    if let Some(ms) = snap.auth_ms {
        parts.push(format!("Auth {ms}ms"));
    }
    match (snap.fetch_ms, snap.resume_ms) {
        (Some(fetch), Some(resume)) => parts.push(format!("Sync {}ms", fetch + resume)),
        (Some(ms), None) | (None, Some(ms)) => parts.push(format!("Sync {ms}ms")),
        _ => {}
    }
    parts.join(" \u{00b7} ")
}

fn kv_row(key: &'static str, value: &str) -> Div {
    div()
        .flex()
        .flex_row()
        .gap(px(6.0))
        .child(
            div()
                .w(px(60.0))
                .flex_shrink_0()
                .text_color(rgb(theme::TEXT_MUTED))
                .text_size(px(theme::FONT_DETAIL))
                .child(key),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(rgb(theme::TEXT_SECONDARY))
                .text_size(px(theme::FONT_DETAIL))
                .child(value.to_string()),
        )
}
