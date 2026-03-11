/// Transport badge: connection type indicator (P2P / Relay / Reconnecting).
use gpui::*;
use zedra_session::SessionState;

use crate::theme;

/// Compute badge label and dot color from session state.
///
/// Reconnect fields (`reconnect_attempt`, `reconnect_reason`, `next_retry_secs`)
/// come from the `SessionHandle` atomics — they are the source of truth since
/// `SessionState::Reconnecting` is not written by the reconnect loop.
///
/// Returns `(label, dot_color)` for rendering in the workspace header.
pub(crate) fn transport_badge_info(
    state: &SessionState,
    reconnect_attempt: u32,
    reconnect_reason: &zedra_session::ReconnectReason,
    next_retry_secs: u64,
    latency_ms: u64,
    conn_info: Option<&zedra_session::ConnectionInfo>,
) -> (String, u32) {
    // Reconnect state from handle takes priority over session state
    if reconnect_attempt > 0 {
        let reason_str = match reconnect_reason {
            zedra_session::ReconnectReason::AppForegrounded => "app resumed",
            zedra_session::ReconnectReason::ConnectionLost => "lost connection",
        };
        let label = if next_retry_secs > 0 {
            format!(
                "Reconnecting ({}) \u{00b7} {}s \u{00b7} {}",
                reconnect_attempt, next_retry_secs, reason_str
            )
        } else {
            format!(
                "Reconnecting ({}) \u{00b7} {}",
                reconnect_attempt, reason_str
            )
        };
        return (label, theme::ACCENT_RED);
    }

    match state {
        SessionState::Disconnected | SessionState::HostUnreachable => {
            ("Disconnected".into(), theme::ACCENT_RED)
        }
        SessionState::Connecting { phase } => (
            format!("Connecting\u{2026} {}", phase),
            theme::ACCENT_YELLOW,
        ),
        SessionState::Reconnecting { attempt, .. } => {
            (format!("Reconnecting ({})", attempt), theme::ACCENT_RED)
        }
        SessionState::Error(msg) => {
            let label = if msg.len() > 24 {
                format!("Error: {}\u{2026}", &msg[..24])
            } else {
                format!("Error: {}", msg)
            };
            (label, theme::ACCENT_RED)
        }
        SessionState::Connected { .. } => {
            let (conn_type, relay_info): (&str, Option<&str>) = match conn_info {
                Some(i) if i.is_direct => ("P2P", None),
                Some(i) => ("Relay", Some(i.relay_url.as_deref().unwrap_or("relay"))),
                None => ("\u{2026}", None),
            };
            let label = if let Some(relay) = relay_info {
                if latency_ms > 0 {
                    format!("{} \u{00b7} {} \u{00b7} {}ms", conn_type, relay, latency_ms)
                } else {
                    format!("{} \u{00b7} {}", conn_type, relay)
                }
            } else if latency_ms > 0 {
                format!("{} \u{00b7} {}ms", conn_type, latency_ms)
            } else {
                conn_type.to_string()
            };
            let color = match conn_info {
                Some(i) if i.is_direct => theme::ACCENT_GREEN,
                Some(_) => theme::ACCENT_YELLOW,
                None => theme::ACCENT_GREEN,
            };
            (label, color)
        }
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
