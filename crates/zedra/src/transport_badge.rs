/// Transport badge: connection type indicator (P2P / Relay / Reconnecting).
use gpui::*;
use zedra_session::{ConnectPhase, TransportSnapshot};

use crate::theme;

/// Seconds after last received bytes before a path is considered stale.
pub const STALE_THRESHOLD_SECS: u64 = 3;

/// Compute badge label and dot color from phase and transport.
/// Returns `(label, dot_color)`.
pub(crate) fn transport_badge(
    phase: &ConnectPhase,
    transport: Option<&TransportSnapshot>,
) -> (String, u32) {
    match phase {
        ConnectPhase::Init => ("Initializing".into(), theme::ACCENT_YELLOW),
        ConnectPhase::Connected => {
            let (conn_type, relay): (String, Option<&str>) = match transport {
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
            let rtt = transport.map(|t| t.rtt_ms).unwrap_or(0);
            let label = match (relay, rtt) {
                (Some(r), ms) if ms > 0 => format!("{conn_type} \u{00b7} {ms}ms"),
                (None, ms) if ms > 0 => format!("{conn_type} \u{00b7} {ms}ms"),
                _ => conn_type.to_string(),
            };
            let color = match transport {
                Some(t) if t.is_direct => theme::ACCENT_GREEN,
                Some(_) => theme::ACCENT_GREEN,
                None => theme::ACCENT_GREEN,
            };
            (label, color)
        }
        ConnectPhase::Disconnected => ("Disconnected".into(), theme::ACCENT_RED),
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
        ConnectPhase::BindingEndpoint => ("Binding endpoint".into(), theme::ACCENT_YELLOW),
        ConnectPhase::HolePunching => ("Hole punching".into(), theme::ACCENT_YELLOW),
        ConnectPhase::Registering => ("Registering device".into(), theme::ACCENT_YELLOW),
        ConnectPhase::Authenticating => ("Authenticating".into(), theme::ACCENT_YELLOW),
        ConnectPhase::Proving => ("Auth challenge".into(), theme::ACCENT_YELLOW),
        ConnectPhase::Sync => ("Syncing state".into(), theme::ACCENT_YELLOW),
        ConnectPhase::Idle { idle_since } => (
            format!("Idle {}s", idle_since.elapsed().as_secs()),
            theme::ACCENT_YELLOW,
        ),
    }
}

pub(crate) fn phase_indicator_color(phase: &ConnectPhase) -> u32 {
    if phase.is_connected() {
        theme::ACCENT_GREEN
    } else if phase.is_idle() || phase.is_connecting() || phase.is_reconnecting() {
        theme::ACCENT_YELLOW
    } else if phase.is_failed() {
        theme::ACCENT_RED
    } else {
        theme::ACCENT_DIM
    }
}

/// Render an inline transport badge element (dot + label).
pub(crate) fn render_transport_badge(label: String, color: u32) -> Div {
    div()
        .text_size(px(theme::FONT_DETAIL))
        .text_color(rgb(color))
        .child(label)
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
