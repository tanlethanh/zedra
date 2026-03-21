/// Transport badge: connection type indicator (P2P / Relay / Reconnecting).
use gpui::*;
use zedra_session::{ConnectPhase, ConnectState};

use crate::theme;

/// Compute badge label and dot color from connect state.
/// Returns `(label, dot_color)` for rendering in the workspace header and
/// as the phase subtitle in the connecting view.
///
/// For connecting phases the label includes live discovery data (relay latency,
/// NAT type, elapsed time) so the user can follow progress.
pub(crate) fn transport_badge_info(state: &ConnectState) -> (String, u32) {
    let snap = &state.snapshot;
    let elapsed = state.elapsed_secs();

    match &state.phase {
        ConnectPhase::Connected => {
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
        ConnectPhase::BindingEndpoint => {
            let label = if elapsed > 0 {
                format!("Binding endpoint \u{00b7} {elapsed}s")
            } else {
                "Binding endpoint".into()
            };
            (label, theme::ACCENT_YELLOW)
        }
        ConnectPhase::HolePunching => {
            let mut parts: Vec<String> = Vec::new();
            // Relay status
            if snap.relay_connected {
                match snap.relay_latency_ms {
                    Some(ms) => parts.push(format!("relay {ms}ms")),
                    None => parts.push("relay ok".into()),
                }
            } else {
                parts.push("relay\u{2026}".into());
            }
            // NAT / IP hints
            if let Some(varies) = snap.mapping_varies {
                parts.push(if varies { "symmetric NAT".into() } else { "cone NAT".into() });
            } else {
                match (snap.has_ipv4, snap.has_ipv6) {
                    (true, true) => parts.push("v4+v6".into()),
                    (true, false) => parts.push("v4".into()),
                    (false, true) => parts.push("v6".into()),
                    _ => {}
                }
            }
            // Elapsed
            if elapsed > 0 {
                parts.push(format!("{elapsed}s"));
            }
            let label = if parts.is_empty() {
                "Connecting\u{2026}".into()
            } else {
                parts.join(" \u{00b7} ")
            };
            (label, theme::ACCENT_YELLOW)
        }
        ConnectPhase::EstablishingRpc => {
            let label = if elapsed > 0 {
                format!("RPC setup \u{00b7} {elapsed}s")
            } else {
                "RPC setup".into()
            };
            (label, theme::ACCENT_YELLOW)
        }
        ConnectPhase::Registering => ("Registering device".into(), theme::ACCENT_YELLOW),
        ConnectPhase::Authenticating | ConnectPhase::Proving => {
            ("PKI challenge".into(), theme::ACCENT_YELLOW)
        }
        ConnectPhase::FetchingInfo => ("Fetching workspace info".into(), theme::ACCENT_YELLOW),
        ConnectPhase::ResumingTerminals => ("Resuming terminals".into(), theme::ACCENT_YELLOW),
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
