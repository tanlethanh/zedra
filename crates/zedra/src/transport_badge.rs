/// Transport badge: connection type indicator (P2P / Relay / Reconnecting).
use gpui::*;
use zedra_session::{ConnectPhase, ConnectState};

use crate::theme;

/// Compute badge label and dot color from connect state.
/// Returns `(label, dot_color)` for rendering in the workspace header.
pub(crate) fn transport_badge_info(state: &ConnectState) -> (String, u32) {
    match &state.phase {
        ConnectPhase::Connected => {
            let snap = &state.snapshot;
            let (conn_type, relay): (String, Option<&str>) = match &snap.transport {
                Some(t) if t.is_direct => {
                    let hint = t
                        .network_hint
                        .as_ref()
                        .map(|h| format!("P2P \u{00b7} {}", h.label()))
                        .unwrap_or_else(|| "P2P".into());
                    (hint, None)
                }
                Some(t) => ("Relay".into(), Some(t.remote_addr.as_str())),
                None => ("\u{2026}".into(), None),
            };
            let rtt = snap.transport.as_ref().map(|t| t.rtt_ms).unwrap_or(0);
            let label = match (relay, rtt) {
                (Some(r), ms) if ms > 0 => format!("{conn_type} \u{00b7} {r} \u{00b7} {ms}ms"),
                (Some(r), _) => format!("{conn_type} \u{00b7} {r}"),
                (None, ms) if ms > 0 => format!("{conn_type} \u{00b7} {ms}ms"),
                _ => conn_type.to_string(),
            };
            let color = match &snap.transport {
                Some(t) if t.is_direct => theme::ACCENT_GREEN,
                Some(_) => theme::ACCENT_YELLOW,
                None => theme::ACCENT_GREEN,
            };
            (label, color)
        }
        ConnectPhase::Reconnecting {
            attempt,
            next_retry_secs,
            ..
        } => {
            let label = if *next_retry_secs > 0 {
                format!("Reconnecting ({attempt}) \u{00b7} {next_retry_secs}s")
            } else {
                format!("Reconnecting ({attempt})")
            };
            (label, theme::ACCENT_RED)
        }
        ConnectPhase::Failed(err) => (err.user_message(), theme::ACCENT_RED),
        p if p.is_connecting() => (
            format!("Connecting\u{2026} {}", p.display_name()),
            theme::ACCENT_YELLOW,
        ),
        _ => ("Disconnected".into(), theme::ACCENT_RED),
    }
}

/// Render an inline transport badge element (dot + label).
pub(crate) fn render_transport_badge(label: String, dot_color: u32) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(3.0))
        .child(
            div()
                .w(px(theme::ICON_STATUS))
                .h(px(theme::ICON_STATUS))
                .rounded(px(3.0))
                .bg(rgb(dot_color)),
        )
        .child(
            div()
                .text_size(px(theme::FONT_DETAIL))
                .text_color(rgb(dot_color))
                .child(label),
        )
}

/// Human-friendly byte count formatting.
pub fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}
