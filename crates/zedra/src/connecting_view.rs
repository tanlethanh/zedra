/// Connection loading screen — shown while a workspace is connecting/reconnecting/failed.
///
/// Layout:
///   1. Horizontal 5-step progress stepper
///   2. Vertical current-phase detail (transport, auth, host, timing, error/reconnect banners)
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use gpui::{Animation, AnimationExt as _, Transformation, prelude::FluentBuilder as _, *};

// Incremented each time the retry button is pressed.
// Each new value produces a unique animation element ID, causing GPUI to start
// a fresh oneshot rotation for exactly that press.
static RETRY_GENERATION: AtomicU64 = AtomicU64::new(0);
use zedra_session::{ConnectSnapshot, ConnectState, STEPPER_STEP_NAMES, TransportSnapshot};

use crate::platform_bridge::{self, AlertButton};
use crate::theme;
use crate::transport_badge::{format_bytes, render_transport_badge, transport_badge_info};

// ─── Public view ─────────────────────────────────────────────────────────────

pub struct ConnectingView {
    session_handle: zedra_session::SessionHandle,
    details_expanded: bool,
}

impl ConnectingView {
    pub fn new(handle: zedra_session::SessionHandle) -> Self {
        Self {
            session_handle: handle,
            details_expanded: false,
        }
    }
}

impl Render for ConnectingView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let cs = self.session_handle.connect_state();
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
            .child(render_phase_title(&cs, &self.session_handle))
            .child(render_stepper(&cs))
            .child(render_details_toggle(expanded, cx))
            .when(expanded, |d| d.child(render_detail(&cs)))
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

// ─── Retry button (reusable) ─────────────────────────────────────────────────

/// A 18×18 refresh icon button that spins once for 1 s when pressed.
/// Returns `None` when the current phase is not retryable.
/// The icon is dimmed normally; brightens to `TEXT_SECONDARY` after 30 s stuck.
pub fn render_retry_button(handle: &zedra_session::SessionHandle) -> Option<Div> {
    let cs = handle.connect_state();
    if !cs.phase.is_connecting() && !cs.phase.is_reconnecting() && !cs.phase.is_failed() {
        return None;
    }
    let stuck = cs.elapsed_secs() >= 30;
    let retry_color = if stuck {
        rgb(theme::TEXT_SECONDARY)
    } else {
        rgb(theme::TEXT_MUTED)
    };
    let handle_retry = handle.clone();
    let generation = RETRY_GENERATION.load(Ordering::Acquire);

    // A new generation ID causes GPUI to start a fresh oneshot animation.
    let retry_icon: AnyElement = if generation > 0 {
        svg()
            .path("icons/refresh-ccw.svg")
            .size_full()
            .text_color(retry_color)
            .with_animation(
                SharedString::from(format!("retry-spin-{generation}")),
                Animation::new(Duration::from_secs(1)),
                |svg, delta| svg.with_transformation(Transformation::rotate(percentage(delta))),
            )
            .into_any_element()
    } else {
        svg()
            .path("icons/refresh-ccw.svg")
            .size_full()
            .text_color(retry_color)
            .into_any_element()
    };

    Some(
        div()
            .cursor_pointer()
            .w(px(14.0))
            .h(px(14.0))
            .hit_slop(px(10.0))
            .on_mouse_down(MouseButton::Left, move |_, _, _| {
                RETRY_GENERATION.fetch_add(1, Ordering::Release);
                handle_retry.retry_connect();
            })
            .child(retry_icon),
    )
}

// ─── Phase title ─────────────────────────────────────────────────────────────

fn render_phase_title(cs: &ConnectState, handle: &zedra_session::SessionHandle) -> Div {
    let (label, dot_color) = transport_badge_info(cs);

    div()
        .mb(px(theme::SPACING_LG))
        .flex()
        .flex_col()
        .items_center()
        .gap(px(8.0))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(
                    // Fixed width so the retry icon always appears at the same position
                    // regardless of phase name length (longest: "Resuming terminals").
                    div()
                        .w(px(160.0))
                        .min_w_0()
                        .truncate()
                        .text_align(TextAlign::Center)
                        .text_color(rgb(theme::TEXT_PRIMARY))
                        .text_size(px(theme::FONT_HEADING))
                        .font_weight(FontWeight::MEDIUM)
                        .child(cs.phase.display_name()),
                )
                .children(render_retry_button(handle)),
        )
        .child(render_transport_badge(label, dot_color))
}

// ─── Horizontal stepper ──────────────────────────────────────────────────────

fn render_stepper(cs: &ConnectState) -> Div {
    let active_step = cs.phase.step_index().unwrap_or_else(|| {
        if cs.phase.is_idle() {
            0
        } else {
            // Reconnecting / Failed: use snapshot's failed_at_step or last completed step
            cs.snapshot
                .failed_at_step
                .unwrap_or_else(|| completed_step_count(cs).saturating_sub(1).min(2))
        }
    });

    let completed = completed_step_count(cs);

    let mut row = div()
        .w(px(180.0))
        .flex()
        .flex_row()
        .items_center()
        .mb(px(theme::SPACING_LG));

    for (i, name) in STEPPER_STEP_NAMES.iter().enumerate() {
        let is_done = i < completed && !cs.phase.is_failed();
        let is_active = i == active_step;
        let is_failed = cs.phase.is_failed() && i == active_step;

        // Dot
        let dot_color = if is_failed {
            rgb(theme::ACCENT_RED)
        } else if is_done {
            rgb(theme::ACCENT_GREEN)
        } else if is_active {
            rgb(theme::ACCENT_YELLOW)
        } else {
            rgb(theme::TEXT_MUTED)
        };

        let dot_border_color = dot_color;
        let dot_size = 10.0_f32;

        let dot = div()
            .w(px(dot_size))
            .h(px(dot_size))
            .rounded(px(dot_size / 2.0))
            .border_1()
            .border_color(dot_border_color)
            .when(is_done || is_active || is_failed, |d: Div| d.bg(dot_color));

        // Soft scale pulse on the active (in-progress) dot.
        // Uses an SVG circle so Transformation::scale() applies visually without affecting layout.
        let dot_element: AnyElement = if is_active && !is_failed {
            svg()
                .path("icons/dot.svg")
                .size(px(dot_size))
                .text_color(dot_color)
                .with_animation(
                    SharedString::from(format!("stepper-pulse-{i}")),
                    Animation::new(Duration::from_millis(1200)).repeat(),
                    move |s, delta| {
                        let t = (delta * std::f32::consts::PI).sin();
                        let scale = 1.0 + 0.35 * t;
                        s.with_transformation(Transformation::scale(size(scale, scale)))
                    },
                )
                .into_any_element()
        } else {
            dot.into_any_element()
        };

        let step_col = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(4.0))
            .child(dot_element)
            .child(
                div()
                    .text_size(px(9.0))
                    .text_color(if is_done {
                        rgb(theme::ACCENT_GREEN)
                    } else if is_active && !is_failed {
                        rgb(theme::ACCENT_YELLOW)
                    } else if is_failed {
                        rgb(theme::ACCENT_RED)
                    } else {
                        rgb(theme::TEXT_MUTED)
                    })
                    .child(*name),
            );

        row = row.child(step_col);

        // Connector line between steps
        if i < STEPPER_STEP_NAMES.len() - 1 {
            let line_color = if i + 1 <= completed && !cs.phase.is_failed() {
                rgb(theme::ACCENT_GREEN)
            } else {
                rgb(theme::BORDER_SUBTLE)
            };
            row = row.child(
                div()
                    .flex_1()
                    .h(px(1.0))
                    .mb(px(14.0)) // align with dots (above label)
                    .bg(line_color),
            );
        }
    }

    row
}

/// Number of steps that have been fully completed.
/// Step mapping: 0=Connect, 1=Auth, 2=Sync
fn completed_step_count(cs: &ConnectState) -> usize {
    if cs.phase.is_connected() {
        return 3;
    }
    let snap = &cs.snapshot;
    let mut n = 0;
    if snap.rpc_ms.is_some() {
        n = 1; // Connect done
    }
    if snap.auth_ms.is_some() {
        n = 2; // Auth done
    }
    if snap.resume_ms.is_some() {
        n = 3; // Sync done
    }
    n
}

// ─── Phase status helpers ────────────────────────────────────────────────────

/// Returns true if the snapshot has any discovery data worth showing.
fn has_discovery_data(snap: &ConnectSnapshot) -> bool {
    snap.relay_connected
        || !snap.direct_addrs.is_empty()
        || snap.has_ipv4
        || snap.has_ipv6
        || snap.relay_latency_ms.is_some()
}

/// Render the Discovery section rows (network probing results).
fn render_discovery_rows(snap: &ConnectSnapshot) -> Div {
    let mut d = div().flex().flex_col().gap(px(2.0));

    // Relay status
    let relay_status = if snap.relay_connected {
        match snap.relay_latency_ms {
            Some(ms) => format!("Connected ({ms}ms)"),
            None => "Connected".into(),
        }
    } else {
        "Connecting\u{2026}".into()
    };
    d = d.child(kv_row("Relay", &relay_status));

    // Direct addresses discovered
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

    // IPv4 / IPv6 reachability
    let ip_status = match (snap.has_ipv4, snap.has_ipv6) {
        (true, true) => "IPv4 + IPv6",
        (true, false) => "IPv4 only",
        (false, true) => "IPv6 only",
        (false, false) => "probing\u{2026}",
    };
    d = d.child(kv_row("UDP", ip_status));

    // NAT type
    if let Some(varies) = snap.mapping_varies {
        let nat = if varies {
            "Symmetric (hard NAT)"
        } else {
            "Cone / direct"
        };
        d = d.child(kv_row("NAT", nat));
    }

    // Captive portal
    if snap.captive_portal == Some(true) {
        d = d.child(kv_row("Portal", "Captive portal detected"));
    }

    d
}

// ─── Vertical detail panel ───────────────────────────────────────────────────

fn render_detail(cs: &ConnectState) -> Div {
    let snap = &cs.snapshot;
    let mut col = div()
        .w(px(theme::CONNECT_DETAIL_WIDTH))
        .flex()
        .flex_col()
        .gap(px(theme::SPACING_SM));

    // Endpoint section
    if snap.local_node_id.is_some() || snap.remote_node_id.is_some() || snap.relay_url.is_some() {
        col = col.child(render_section("Endpoint", render_endpoint_rows(snap)));
    }

    // Discovery section (live during HolePunching)
    if has_discovery_data(snap) {
        col = col.child(render_section("Discovery", render_discovery_rows(snap)));
    }

    // Transport section
    if let Some(t) = &snap.transport {
        col = col.child(render_section("Transport", render_transport_rows(t)));
    }

    // Auth section
    if snap.session_id.is_some() || snap.auth_outcome.is_some() {
        col = col.child(render_section("Auth", render_auth_rows(snap)));
    }

    // Host section
    if snap.hostname.is_some() {
        col = col.child(render_section("Host", render_host_rows(snap)));
    }

    // Timing row
    let timing = build_timing_string(snap);
    if !timing.is_empty() {
        col = col.child(
            div()
                .text_color(rgb(theme::TEXT_MUTED))
                .text_size(px(theme::FONT_DETAIL))
                .child(timing),
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
    if let Some(v) = &snap.hostname {
        d = d.child(kv_row("Host", v));
    }
    if let Some(v) = &snap.username {
        if !v.is_empty() {
            d = d.child(kv_row("User", v));
        }
    }
    if let Some(v) = &snap.workdir {
        d = d.child(kv_row("Workdir", v));
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
