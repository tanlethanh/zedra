/// Transport badge: connection type indicator (P2P / Relay / Reconnecting).
use std::time::Duration;

use gpui::*;
use zedra_session::{ConnectPhase, TransportSnapshot};

use crate::theme;

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
        ConnectPhase::Disconnected => ("Tap refresh to reconnect.".into(), theme::ACCENT_RED),
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

fn phase_indicator_blinks(phase: &ConnectPhase) -> bool {
    phase_indicator_color(phase) == theme::ACCENT_YELLOW
}

const STATUS_PULSE_MS: u64 = 1800;
const STATUS_PULSE_MIN_OPACITY: f32 = 0.35;

#[derive(Clone, IntoElement)]
pub(crate) struct ConnectionStatusIndicator {
    id: ElementId,
    color: u32,
    size: f32,
    blink: bool,
}

impl ConnectionStatusIndicator {
    pub(crate) fn from_phase(id: impl Into<ElementId>, phase: Option<&ConnectPhase>) -> Self {
        let color = phase
            .map(phase_indicator_color)
            .unwrap_or(theme::ACCENT_DIM);
        let blink = phase.map(phase_indicator_blinks).unwrap_or(false);

        Self {
            id: id.into(),
            color,
            size: theme::ICON_STATUS,
            blink,
        }
    }

    pub(crate) fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }
}

impl RenderOnce for ConnectionStatusIndicator {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let dot = div()
            .w(px(self.size))
            .h(px(self.size))
            .rounded(px(self.size / 2.0))
            .flex_shrink_0()
            .bg(rgb(self.color));

        if self.blink {
            dot.with_animation(
                self.id,
                Animation::new(Duration::from_millis(STATUS_PULSE_MS)).repeat(),
                |dot, delta| {
                    let wave = 0.5 - 0.5 * (delta * std::f32::consts::TAU).cos();
                    let opacity =
                        STATUS_PULSE_MIN_OPACITY + wave * (1.0 - STATUS_PULSE_MIN_OPACITY);
                    dot.opacity(opacity)
                },
            )
            .into_any_element()
        } else {
            dot.into_any_element()
        }
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

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use crate::theme;
    use crate::transport_badge::{
        ConnectionStatusIndicator, phase_indicator_color, transport_badge,
    };
    use zedra_session::{ConnectError, ConnectPhase};

    #[test]
    fn disconnected_badge_uses_manual_reconnect_hint() {
        let (label, color) = transport_badge(&ConnectPhase::Disconnected, None);

        assert_eq!(label, "Tap refresh to reconnect.");
        assert_eq!(color, theme::ACCENT_RED);
    }

    #[test]
    fn reconnecting_badge_includes_retry_countdown() {
        let (label, color) = transport_badge(
            &ConnectPhase::Reconnecting {
                attempt: 2,
                reason: zedra_session::ReconnectReason::ConnectionLost,
                next_retry_secs: 3,
            },
            None,
        );

        assert_eq!(label, "Reconnecting (2) \u{00b7} 3s");
        assert_eq!(color, theme::ACCENT_RED);
    }

    #[test]
    fn warning_status_indicator_blinks() {
        let idle = ConnectPhase::Idle {
            idle_since: Instant::now(),
        };
        let reconnecting = ConnectPhase::Reconnecting {
            attempt: 1,
            reason: zedra_session::ReconnectReason::ConnectionLost,
            next_retry_secs: 2,
        };
        let connected = ConnectPhase::Connected;
        let failed = ConnectPhase::Failed(ConnectError::HostUnreachable);

        for phase in [&idle, &reconnecting] {
            let indicator = ConnectionStatusIndicator::from_phase("test-dot", Some(phase));
            assert_eq!(phase_indicator_color(phase), theme::ACCENT_YELLOW);
            assert!(indicator.blink);
        }

        for phase in [&connected, &failed] {
            let indicator = ConnectionStatusIndicator::from_phase("test-dot", Some(phase));
            assert_ne!(phase_indicator_color(phase), theme::ACCENT_YELLOW);
            assert!(!indicator.blink);
        }
    }

    #[test]
    fn failed_badge_uses_friendly_error_message() {
        let cases = [
            (
                ConnectError::AlpnMismatch,
                "Protocol mismatch, Update App or CLI",
            ),
            (
                ConnectError::ConnectionClosed,
                "Connection closed. Tap refresh to reconnect.",
            ),
            (
                ConnectError::HandshakeConsumed,
                "The QR code was used. Refresh it and scan again.",
            ),
            (
                ConnectError::SessionOccupied,
                "Host occupied. Disconnect other device and retry.",
            ),
            (
                ConnectError::HostUnreachable,
                "Host unreachable. Check network and host.",
            ),
        ];

        for (error, message) in cases {
            let (label, color) = transport_badge(&ConnectPhase::Failed(error), None);

            assert_eq!(label, message);
            assert_eq!(color, theme::ACCENT_RED);
        }
    }
}
