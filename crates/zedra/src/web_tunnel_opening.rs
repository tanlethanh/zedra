//! Temporary overlay shown while a tunnelled page is being opened in the in-app
//! webview. Opening an agent web client spawns a server host-side, so seconds
//! pass with nothing on screen otherwise; this names the step in flight.
//!
//! State lives in [`crate::web_tunnel::progress`] because the tunnel steps run
//! on the session runtime. This view polls it and repaints.

use std::{f32::consts::TAU, time::Duration};

use gpui::{prelude::FluentBuilder as _, *};

use crate::pending::spawn_periodic_task;
use crate::theme;
use crate::web_tunnel::progress::{OpenProgress, Progress};
use crate::workspace_action;

/// Poll cadence: fast enough that a step change reads as immediate, and it also
/// ticks the elapsed counter.
const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Elapsed seconds past which the step gets an elapsed counter. Below this a
/// timer just adds noise to an open that is about to succeed.
const SLOW_OPEN_SECS: u64 = 2;

pub struct WebTunnelOpening {
    progress: Progress,
    version: u64,
    _poll: Task<()>,
}

impl WebTunnelOpening {
    pub fn new(progress: Progress, cx: &mut Context<Self>) -> Self {
        let poll_progress = progress.clone();
        let poll = spawn_periodic_task(cx, POLL_INTERVAL, move |this: &mut Self, cx| {
            let version = poll_progress.version();
            // Repaint on any change, and keep repainting while an open is live so
            // the spinner and elapsed counter advance.
            if version != this.version || poll_progress.current().is_some() {
                this.version = version;
                cx.notify();
            }
        });

        Self {
            version: progress.version(),
            progress,
            _poll: poll,
        }
    }
}

impl Render for WebTunnelOpening {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(progress) = self.progress.current() else {
            return div().into_any_element();
        };

        // The close button anchors to this layer, so the centred column below is
        // laid out on its own and stays centred whatever the corner holds.
        div()
            .id("web-tunnel-opening")
            .absolute()
            .inset_0()
            .bg(rgb(theme::bg_primary(cx)))
            // Swallow presses so the covered view can't be driven blind.
            .on_press(|_, _, _| {})
            .child(
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(theme::SPACING_XL))
                    .px(px(theme::SPACING_LG))
                    .child(render_subject(&progress, cx))
                    .child(render_step(&progress, cx))
                    .when_some(progress.error.clone(), |d, error| {
                        d.child(render_error(&error, cx))
                    }),
            )
            .child(render_close_button(cx))
            .into_any_element()
    }
}

fn render_subject(progress: &OpenProgress, cx: &App) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(theme::SPACING_SM))
        .child(
            svg()
                .path(progress.icon.clone())
                .size(px(theme::ICON_LG))
                .text_color(rgb(theme::text_secondary(cx))),
        )
        .child(
            div()
                .max_w_full()
                .min_w_0()
                .truncate()
                .text_color(rgb(theme::text_primary(cx)))
                .text_size(px(theme::FONT_HEADING))
                .font_weight(FontWeight::MEDIUM)
                .child(progress.subject.clone()),
        )
}

/// Only the step in flight: the earlier ones are done and the later ones say
/// nothing about whether this open is moving.
fn render_step(progress: &OpenProgress, cx: &App) -> Div {
    let failed = progress.error.is_some();
    let elapsed = progress.elapsed_secs();
    let mut label = progress.step.label(&progress.subject);
    if !failed && elapsed >= SLOW_OPEN_SECS {
        label = format!("{label}\u{2026} {elapsed}s");
    }
    let color = if failed {
        theme::accent_red(cx)
    } else {
        theme::text_secondary(cx)
    };

    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::SPACING_SM))
        .child(
            div()
                .w(px(theme::ICON_XS))
                .h(px(theme::ICON_XS))
                .flex()
                .items_center()
                .justify_center()
                .child(render_step_marker(failed, cx)),
        )
        .child(
            div()
                .text_color(rgb(color))
                .text_size(px(theme::FONT_BODY))
                .child(label),
        )
}

fn render_step_marker(failed: bool, cx: &App) -> AnyElement {
    if failed {
        return svg()
            .path("icons/x.svg")
            .size(px(theme::ICON_FILE))
            .text_color(rgb(theme::accent_red(cx)))
            .into_any_element();
    }

    svg()
        .path("icons/refresh-ccw.svg")
        .size(px(theme::ICON_FILE))
        .text_color(rgb(theme::text_secondary(cx)))
        .with_animation(
            ElementId::Name("web-tunnel-opening-spin".into()),
            Animation::new(Duration::from_millis(700)).repeat(),
            |icon, delta| icon.with_transformation(Transformation::rotate(radians(TAU * delta))),
        )
        .into_any_element()
}

fn render_error(error: &str, cx: &App) -> Div {
    div()
        .max_w(px(theme::CONNECT_DETAIL_WIDTH))
        .text_align(TextAlign::Center)
        .text_color(rgb(theme::text_muted(cx)))
        .text_size(px(theme::FONT_DETAIL))
        .child(error.to_string())
}

/// Same corner affordance as the connecting view; closing also abandons the
/// open, so a webview can't arrive after the user has left.
fn render_close_button(cx: &mut Context<WebTunnelOpening>) -> Stateful<Div> {
    div()
        .id("web-tunnel-opening-close")
        .absolute()
        .top(px((theme::HEADER_BUTTON_SIZE - theme::ICON_SM) / 2.0))
        .right(px((theme::HEADER_BUTTON_SIZE - theme::ICON_SM) / 2.0))
        .w(px(theme::ICON_SM))
        .h(px(theme::ICON_SM))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hit_slop(px(20.0))
        .on_press(cx.listener(|_this, _event, window, cx| {
            window.dispatch_action(workspace_action::CancelWebTunnelOpen.boxed_clone(), cx);
        }))
        .child(
            svg()
                .path("icons/x.svg")
                .size(px(16.0))
                .text_color(rgb(theme::text_muted(cx))),
        )
}
