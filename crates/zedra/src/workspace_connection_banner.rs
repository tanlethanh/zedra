//! Animated status banner shown over the workspace main view while the session
//! is not connected (idle, reconnecting, disconnected, or failed). It slides
//! down under the header on appearance and, once the session reconnects, lingers
//! briefly then slides back up and removes itself.
//!
//! Tapping the banner opens the connection detail; the refresh button restarts
//! the connection.

use std::time::Duration;

use gpui::*;
use zedra_session::{ConnectPhase, HolePunchStage, SessionState};

use crate::platform_bridge::{self, HapticFeedback};
use crate::theme;

/// Emitted to the owning workspace. Delivered as an entity event rather than a
/// dispatched action so it does not depend on focus being on a workspace
/// element — `window.dispatch_action` silently misses the workspace handlers
/// when focus is elsewhere (e.g. while idle), which made navigation flaky.
#[derive(Clone, Copy, Debug)]
pub enum BannerEvent {
    /// Open the connection detail view.
    OpenDetail,
    /// Restart the connection.
    Refresh,
}

const BANNER_HEIGHT: f32 = 32.0;
/// How long the banner lingers after reconnecting before it slides away.
const BANNER_LINGER: Duration = Duration::from_millis(1000);
/// Slide/fade duration for both the enter and exit transitions.
const BANNER_ANIM: Duration = Duration::from_millis(260);

/// Gentle decelerate-in for the slide-down entrance (settles into place).
fn ease_out_cubic(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

/// Gentle accelerate-out for the slide-up dismissal (eases away).
fn ease_in_cubic(t: f32) -> f32 {
    t.powi(3)
}

/// Phases that warrant the banner: anything that is neither connected nor the
/// pre-connection `Init` state (idle, connecting during a reconnect, reconnect
/// backoff, disconnect, failure).
fn is_bannerable(phase: &ConnectPhase) -> bool {
    !matches!(phase, ConnectPhase::Connected | ConnectPhase::Init)
}

pub struct ConnectionBanner {
    session_state: Entity<SessionState>,
    /// Latched once the session first connects. The banner is a "connection
    /// dropped" affordance over the workspace, so it stays hidden through the
    /// initial connect (which the full connecting screen already covers).
    has_connected: bool,
    visible: bool,
    /// True while playing the slide-up exit; kept in the tree until it removes.
    dismissing: bool,
    /// Bumped on each show so the enter animation re-fires (keyed ElementId).
    enter_id: u64,
    exit_id: u64,
    /// Linger-then-remove task; dropping it cancels a pending auto-dismiss.
    dismiss_task: Option<Task<()>>,
    _session_sub: Subscription,
}

impl EventEmitter<BannerEvent> for ConnectionBanner {}

impl ConnectionBanner {
    pub fn new(session_state: Entity<SessionState>, cx: &mut Context<Self>) -> Self {
        let session_sub = cx.observe(&session_state, |this, _, cx| this.sync(cx));
        let mut banner = Self {
            session_state,
            has_connected: false,
            visible: false,
            dismissing: false,
            enter_id: 0,
            exit_id: 0,
            dismiss_task: None,
            _session_sub: session_sub,
        };
        banner.sync(cx);
        banner
    }

    /// Reconcile visibility with the current phase.
    fn sync(&mut self, cx: &mut Context<Self>) {
        let phase = self.session_state.read(cx).phase.clone();

        if phase.is_connected() {
            self.has_connected = true;
            // Linger, then slide up — only if a drop had shown the banner.
            if self.visible && !self.dismissing && self.dismiss_task.is_none() {
                self.schedule_dismiss(cx);
                cx.notify(); // flip to the green "Connected" state during the linger
            }
        } else if self.has_connected && is_bannerable(&phase) {
            // A drop after the initial connect: cancel any pending dismiss and show.
            self.dismiss_task = None;
            if !self.visible || self.dismissing {
                self.visible = true;
                self.dismissing = false;
                self.enter_id = self.enter_id.wrapping_add(1);
            }
            cx.notify(); // refresh the message even when already visible
        } else if self.visible {
            // Pre-first-connect or non-bannerable state: hide without animating.
            self.visible = false;
            self.dismissing = false;
            self.dismiss_task = None;
            cx.notify();
        }
    }

    /// After a reconnect, linger `BANNER_LINGER`, then start the exit slide.
    fn schedule_dismiss(&mut self, cx: &mut Context<Self>) {
        self.dismiss_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(BANNER_LINGER).await;
            let _ = this.update(cx, |this, cx| this.begin_exit(cx));
        }));
    }

    /// Play the slide-up exit, then remove the banner once the animation ends.
    fn begin_exit(&mut self, cx: &mut Context<Self>) {
        if !self.visible {
            return;
        }
        self.dismissing = true;
        self.exit_id = self.exit_id.wrapping_add(1);
        cx.notify();
        self.dismiss_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(BANNER_ANIM).await;
            let _ = this.update(cx, |this, cx| {
                this.visible = false;
                this.dismissing = false;
                this.dismiss_task = None;
                cx.notify();
            });
        }));
    }

    fn banner_row(&self, message: String, accent: u32, cx: &mut Context<Self>) -> Stateful<Div> {
        div()
            .id("connection-banner")
            .absolute()
            .left_0()
            .right_0()
            .h(px(BANNER_HEIGHT))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(theme::SPACING_SM))
            .px(px(theme::SPACING_MD))
            .bg(rgb(theme::bg_card(cx)))
            .border_b_1()
            .border_color(rgb(theme::border_subtle(cx)))
            .cursor_pointer()
            .on_press(cx.listener(|_this, _event, _window, cx| {
                cx.emit(BannerEvent::OpenDetail);
            }))
            .child(
                svg()
                    .path("icons/dot.svg")
                    .size(px(theme::ICON_STATUS))
                    .flex_shrink_0()
                    .text_color(rgb(accent)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(theme::FONT_DETAIL))
                    .text_color(rgb(theme::text_secondary(cx)))
                    .child(message),
            )
            .child(render_refresh_button(cx))
    }
}

impl Render for ConnectionBanner {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.visible {
            return div().into_any_element();
        }

        let phase = self.session_state.read(cx).phase.clone();
        let stage = self.session_state.read(cx).snapshot.hole_punch_stage;
        let (message, accent) = banner_content(&phase, stage, cx);
        let row = self.banner_row(message, accent, cx);

        if self.dismissing {
            row.with_animation(
                ElementId::NamedInteger("connection-banner-exit".into(), self.exit_id),
                Animation::new(BANNER_ANIM).with_easing(ease_in_cubic),
                |row, delta| row.top(px(-BANNER_HEIGHT * delta)).opacity(1.0 - delta),
            )
            .into_any_element()
        } else {
            row.with_animation(
                ElementId::NamedInteger("connection-banner-enter".into(), self.enter_id),
                Animation::new(BANNER_ANIM).with_easing(ease_out_cubic),
                |row, delta| row.top(px(-BANNER_HEIGHT * (1.0 - delta))).opacity(delta),
            )
            .into_any_element()
        }
    }
}

/// Refresh button — stops propagation so tapping it restarts the connection
/// without also opening the connection detail.
fn render_refresh_button(cx: &mut Context<ConnectionBanner>) -> Stateful<Div> {
    div()
        .id("connection-banner-refresh")
        .w(px(28.0))
        .h(px(28.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .flex_shrink_0()
        .hit_slop(px(10.0))
        .on_pointer_down(|_, _, cx| cx.stop_propagation())
        .on_press(cx.listener(|_this, _event, _window, cx| {
            cx.stop_propagation();
            platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
            cx.emit(BannerEvent::Refresh);
        }))
        .child(
            svg()
                .path("icons/refresh-ccw.svg")
                .size(px(theme::ICON_SM))
                .text_color(rgb(theme::text_muted(cx))),
        )
}

/// User-facing message and accent color for the current phase. `stage` is the
/// live hole-punch sub-stage, surfaced as the connecting progress detail.
fn banner_content(phase: &ConnectPhase, stage: Option<HolePunchStage>, cx: &App) -> (String, u32) {
    let palette = theme::palette(cx);
    match phase {
        // Shown briefly during the linger after a reconnect, matching the header.
        ConnectPhase::Connected => ("Connected".into(), palette.accent_green),
        ConnectPhase::Reconnecting {
            next_retry_secs, ..
        } => {
            let message = if *next_retry_secs > 0 {
                format!("Reconnecting in {next_retry_secs}s")
            } else {
                "Reconnecting".into()
            };
            (message, palette.accent_yellow)
        }
        ConnectPhase::Idle { .. } => ("Connection idle".into(), palette.accent_yellow),
        ConnectPhase::Disconnected => ("Disconnected".into(), palette.accent_red),
        ConnectPhase::Failed(error) => (error.user_message(), palette.accent_red),
        // Remaining bannerable states are the connecting steps during a reconnect;
        // show the phase progress detail after a centered dot.
        _ => {
            let detail = match (phase, stage) {
                (ConnectPhase::HolePunching, Some(stage)) => stage.display_name(),
                _ => phase.display_name(),
            };
            (
                format!("Connecting \u{00b7} {detail}"),
                palette.accent_yellow,
            )
        }
    }
}
