/// Transport badge: connection type indicator (P2P / Relay / Reconnecting).

use std::sync::Mutex;

use gpui::*;

use crate::theme;

/// Last logged transport state, for change-only logging.
static LAST_TRANSPORT_STATE: Mutex<Option<String>> = Mutex::new(None);

/// Compute badge label and dot color from session state.
pub(crate) fn transport_badge_info(
    latency_ms: u64,
    conn_info: Option<&zedra_session::ConnectionInfo>,
) -> (String, u32) {
    // Show reconnecting state if a reconnect attempt is in progress
    let attempt = zedra_session::reconnect_attempt();
    if attempt > 0 {
        let key = format!("reconnecting-{}", attempt);
        if let Ok(mut last) = LAST_TRANSPORT_STATE.lock() {
            if last.as_deref() != Some(&key) {
                log::info!("[PERF] transport: reconnecting attempt={}", attempt);
                *last = Some(key);
            }
        }
        return (format!("Reconnecting... ({})", attempt), theme::ACCENT_RED);
    }

    let conn_type = conn_info
        .map(|i| if i.is_direct { "P2P" } else { "Relay" })
        .unwrap_or("...");
    let label = if latency_ms > 0 {
        format!("{} \u{00b7} {}ms", conn_type, latency_ms)
    } else {
        conn_type.to_string()
    };
    let color = match conn_info {
        Some(i) if i.is_direct => theme::ACCENT_GREEN,
        Some(_) => theme::ACCENT_YELLOW,
        None => theme::ACCENT_GREEN,
    };

    // Log on state change only
    let key = format!("{}-{}", conn_type, latency_ms);
    if let Ok(mut last) = LAST_TRANSPORT_STATE.lock() {
        if last.as_deref() != Some(&key) {
            log::info!("[PERF] transport: {} latency={}ms", conn_type, latency_ms);
            *last = Some(key);
        }
    }

    (label, color)
}

/// Render the positioned transport badge element (absolute, top-right of header).
pub(crate) fn render_transport_badge(label: String, dot_color: u32) -> Div {
    let top_inset = crate::platform_bridge::status_bar_inset();
    let badge_top = top_inset + (48.0 - 18.0) / 2.0;

    div()
        .absolute()
        .top(px(badge_top))
        .right(px(8.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(4.0))
        .px_2()
        .py(px(2.0))
        .rounded(px(4.0))
        .bg(theme::badge_bg())
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
