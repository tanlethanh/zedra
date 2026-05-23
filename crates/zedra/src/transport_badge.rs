/// Transport badge: connection type indicator (P2P / Relay / Reconnecting).
use std::sync::Arc;
use std::time::Duration;

use gpui::*;
use zedra_session::{ConnectPhase, TransportSnapshot};

use crate::platform_bridge::{self, HapticFeedback};
use crate::theme::{self, ThemePalette};

/// Compute badge label and dot color from phase and transport.
/// Returns `(label, dot_color)`.
pub(crate) fn transport_badge(
    palette: &ThemePalette,
    phase: &ConnectPhase,
    transport: Option<&TransportSnapshot>,
) -> (String, u32) {
    match phase {
        ConnectPhase::Init => ("Initializing".into(), palette.accent_yellow),
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
                Some(t) if t.is_direct => palette.accent_green,
                Some(_) => palette.accent_green,
                None => palette.accent_green,
            };
            (label, color)
        }
        ConnectPhase::Disconnected => ("Tap refresh to reconnect.".into(), palette.accent_red),
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
            (label, palette.accent_red)
        }
        ConnectPhase::Failed(err) => (err.user_message(), palette.accent_red),
        ConnectPhase::BindingEndpoint => ("Binding endpoint".into(), palette.accent_yellow),
        ConnectPhase::HolePunching => ("Hole punching".into(), palette.accent_yellow),
        ConnectPhase::Registering => ("Registering device".into(), palette.accent_yellow),
        ConnectPhase::Authenticating => ("Authenticating".into(), palette.accent_yellow),
        ConnectPhase::Proving => ("Auth challenge".into(), palette.accent_yellow),
        ConnectPhase::Sync => ("Syncing state".into(), palette.accent_yellow),
        ConnectPhase::Idle { idle_since } => (
            format!("Idle {}s", idle_since.elapsed().as_secs()),
            palette.accent_yellow,
        ),
    }
}

pub(crate) fn phase_indicator_color(palette: &ThemePalette, phase: &ConnectPhase) -> u32 {
    if phase.is_connected() {
        palette.accent_green
    } else if phase.is_idle() || phase.is_connecting() || phase.is_reconnecting() {
        palette.accent_yellow
    } else if phase.is_failed() {
        palette.accent_red
    } else {
        palette.accent_dim
    }
}

fn phase_indicator_blinks(palette: &ThemePalette, phase: &ConnectPhase) -> bool {
    phase_indicator_color(palette, phase) == palette.accent_yellow
}

const STATUS_PULSE_MS: u64 = 1800;
const STATUS_PULSE_MIN_OPACITY: f32 = 0.35;
const STATUS_PULSE_MAX_SCALE: f32 = 1.3;
const STATUS_HIT_SLOP: f32 = 20.0;

type ConnectionStatusPressHandler = Arc<dyn Fn(&PressEvent, &mut Window, &mut App) + 'static>;

#[derive(Clone, IntoElement)]
pub(crate) struct ConnectionStatusIndicator {
    id: ElementId,
    color: u32,
    size: f32,
    blink: bool,
    on_press: Option<ConnectionStatusPressHandler>,
}

impl ConnectionStatusIndicator {
    pub(crate) fn from_phase(
        id: impl Into<ElementId>,
        phase: Option<&ConnectPhase>,
        palette: &ThemePalette,
    ) -> Self {
        let color = phase
            .map(|phase| phase_indicator_color(palette, phase))
            .unwrap_or(palette.accent_dim);
        let blink = phase
            .map(|phase| phase_indicator_blinks(palette, phase))
            .unwrap_or(false);

        Self {
            id: id.into(),
            color,
            size: theme::ICON_STATUS,
            blink,
            on_press: None,
        }
    }

    pub(crate) fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    /// Press handler compatible with [`Context::listener`].
    pub(crate) fn on_press(
        mut self,
        handler: impl Fn(&PressEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_press = Some(Arc::new(handler));
        self
    }
}

fn status_pulse_wave(delta: f32) -> f32 {
    0.5 - 0.5 * (delta * std::f32::consts::TAU).cos()
}

fn render_status_dot(id: ElementId, color: u32, dot_size: f32, blink: bool) -> AnyElement {
    let dot = svg()
        .path("icons/dot.svg")
        .size(px(dot_size))
        .flex_shrink_0()
        .text_color(rgb(color));

    if blink {
        dot.with_animation(
            id,
            Animation::new(Duration::from_millis(STATUS_PULSE_MS)).repeat(),
            move |dot, delta| {
                let wave = status_pulse_wave(delta);
                let opacity = STATUS_PULSE_MIN_OPACITY + wave * (1.0 - STATUS_PULSE_MIN_OPACITY);
                let scale = 1.0 + wave * (STATUS_PULSE_MAX_SCALE - 1.0);
                dot.opacity(opacity)
                    .with_transformation(Transformation::scale(gpui::size(scale, scale)))
            },
        )
        .into_any_element()
    } else {
        dot.into_any_element()
    }
}

impl RenderOnce for ConnectionStatusIndicator {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let indicator = render_status_dot(self.id.clone(), self.color, self.size, self.blink);

        if let Some(on_press) = self.on_press {
            let button_id = (self.id, "press");
            div()
                .id(button_id)
                .flex()
                .items_center()
                .justify_center()
                .flex_shrink_0()
                .cursor_pointer()
                .hit_slop(px(STATUS_HIT_SLOP))
                .on_pointer_down(|_, _, cx| cx.stop_propagation())
                .on_press(move |event, window, cx| {
                    cx.stop_propagation();
                    platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
                    on_press(event, window, cx);
                })
                .child(indicator)
                .into_any_element()
        } else {
            indicator
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

    use crate::theme::ThemePalette;
    use crate::transport_badge::{
        ConnectionStatusIndicator, phase_indicator_color, transport_badge,
    };
    use zedra_session::{ConnectError, ConnectPhase};

    #[test]
    fn disconnected_badge_uses_manual_reconnect_hint() {
        let palette = ThemePalette::dark();
        let (label, color) = transport_badge(&palette, &ConnectPhase::Disconnected, None);

        assert_eq!(label, "Tap refresh to reconnect.");
        assert_eq!(color, palette.accent_red);
    }

    #[test]
    fn reconnecting_badge_includes_retry_countdown() {
        let palette = ThemePalette::dark();
        let (label, color) = transport_badge(
            &palette,
            &ConnectPhase::Reconnecting {
                attempt: 2,
                reason: zedra_session::ReconnectReason::ConnectionLost,
                next_retry_secs: 3,
            },
            None,
        );

        assert_eq!(label, "Reconnecting (2) \u{00b7} 3s");
        assert_eq!(color, palette.accent_red);
    }

    #[test]
    fn warning_status_indicator_blinks() {
        let palette = ThemePalette::dark();
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
            let indicator =
                ConnectionStatusIndicator::from_phase("test-dot", Some(phase), &palette);
            assert_eq!(
                phase_indicator_color(&palette, phase),
                palette.accent_yellow
            );
            assert!(indicator.blink);
        }

        for phase in [&connected, &failed] {
            let indicator =
                ConnectionStatusIndicator::from_phase("test-dot", Some(phase), &palette);
            assert_ne!(
                phase_indicator_color(&palette, phase),
                palette.accent_yellow
            );
            assert!(!indicator.blink);
        }
    }

    #[test]
    fn failed_badge_uses_friendly_error_message() {
        let palette = ThemePalette::dark();
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
            let (label, color) = transport_badge(&palette, &ConnectPhase::Failed(error), None);

            assert_eq!(label, message);
            assert_eq!(color, palette.accent_red);
        }
    }
}
