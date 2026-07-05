use std::time::Instant;

use gpui::prelude::FluentBuilder;
use gpui::*;

use super::{SharedDropletState, TRAIL_LEN};
use crate::platform_bridge::{self, HapticFeedback};
use crate::settings::{ThemeStateEvent, theme_state};
use crate::theme::ThemePreference;

pub const DROPLET_RADIUS: f32 = 26.0;
const HIT_RADIUS: f32 = 34.0;
const SPRING_STIFFNESS: f32 = 170.0;
const SPRING_DAMPING: f32 = 14.0;
const SETTLE_SPEED: f32 = 3.0;
const SETTLE_DISTANCE: f32 = 0.5;
const MAX_TICK_SECONDS: f32 = 1.0 / 30.0;
// First-follower chase rate; falls off down the chain so the trail follows the drag path.
const TRAIL_CHASE_RATE: f32 = 26.0;
const TRAIL_CHASE_FALLOFF: f32 = 0.72;
// Press feedback: the droplet swells toward this scale while touched.
const PRESSED_SCALE: f32 = 1.25;
const SCALE_RATE: f32 = 18.0;

/// Invisible drag surface; `DropletEffect` draws the visual, this runs the spring.
pub struct DropletOverlay {
    state: SharedDropletState,
    enabled: bool,
    position: Point<f32>,
    velocity: Point<f32>,
    target: Point<f32>,
    trail: [Point<f32>; TRAIL_LEN],
    drag_pointer: Option<PointerId>,
    animating: bool,
    last_tick: Option<Instant>,
    scale_factor: f32,
    radius_scale: f32,
    base_color: (f32, f32, f32),
    _theme_observe: Option<Subscription>,
}

fn theme_base_color(cx: &App) -> (f32, f32, f32) {
    let preference = theme_state(cx)
        .map(|theme| theme.read(cx).preference())
        .unwrap_or_default();
    // Near-neutral, low-contrast tints: a slight lift on dark, a slight shade on light.
    match preference {
        ThemePreference::Dark => (0.22, 0.23, 0.25),
        ThemePreference::Light => (0.86, 0.86, 0.87),
    }
}

impl DropletOverlay {
    /// Owns the effect lifecycle: installs/removes it and publishes on `enabled` changes.
    pub fn new(
        state: SharedDropletState,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // Recolor on theme switch so a resting droplet picks up the new base.
        let theme_observe = theme_state(cx).map(|theme| {
            cx.subscribe(&theme, |this: &mut Self, _, _: &ThemeStateEvent, cx| {
                this.base_color = theme_base_color(cx);
                this.publish_state();
                cx.notify();
            })
        });
        let start = point(80.0, 160.0);
        let overlay = Self {
            state,
            enabled,
            position: start,
            velocity: point(0.0, 0.0),
            target: start,
            trail: [start; TRAIL_LEN],
            drag_pointer: None,
            animating: false,
            last_tick: None,
            scale_factor: window.scale_factor(),
            radius_scale: 1.0,
            base_color: theme_base_color(cx),
            _theme_observe: theme_observe,
        };
        super::apply_droplet_effect(window, enabled, &overlay.state);
        overlay.publish_state();
        overlay
    }

    pub fn set_enabled(&mut self, enabled: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.enabled == enabled {
            return;
        }
        self.enabled = enabled;
        self.drag_pointer = None;
        self.velocity = point(0.0, 0.0);
        self.target = self.position;
        self.trail = [self.position; TRAIL_LEN];
        self.radius_scale = 1.0;
        self.scale_factor = window.scale_factor();
        super::apply_droplet_effect(window, enabled, &self.state);
        self.publish_state();
        cx.notify();
    }

    fn publish_state(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.active = self.enabled;
            state.center = (self.position.x, self.position.y);
            state.radius = DROPLET_RADIUS * self.radius_scale;
            for (slot, follower) in state.trail.iter_mut().zip(self.trail.iter()) {
                *slot = (follower.x, follower.y);
            }
            state.scale_factor = self.scale_factor;
            state.base_color = self.base_color;
        }
    }

    fn start_animation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.animating {
            return;
        }
        self.animating = true;
        self.last_tick = Some(Instant::now());
        self.schedule_tick(window, cx);
        // The tick chain only advances when frames render; request the first one.
        cx.notify();
    }

    fn schedule_tick(&self, window: &mut Window, cx: &mut Context<Self>) {
        cx.on_next_frame(window, |overlay, window, cx| overlay.tick(window, cx));
    }

    fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let now = Instant::now();
        let dt = self
            .last_tick
            .map(|last| ((now - last).as_secs_f32()).min(MAX_TICK_SECONDS))
            .unwrap_or(MAX_TICK_SECONDS);
        self.last_tick = Some(now);

        let displacement = self.target - self.position;
        let acceleration = displacement * SPRING_STIFFNESS - self.velocity * SPRING_DAMPING;
        self.velocity += acceleration * dt;
        self.position += self.velocity * dt;

        // Swell while touched, relax on release.
        let target_scale = if self.drag_pointer.is_some() {
            PRESSED_SCALE
        } else {
            1.0
        };
        self.radius_scale += (target_scale - self.radius_scale) * (1.0 - (-SCALE_RATE * dt).exp());

        // Each follower chases the one ahead, so the trail traces the drag path.
        let mut leader = self.position;
        let mut chase_rate = TRAIL_CHASE_RATE;
        for follower in self.trail.iter_mut() {
            let blend = 1.0 - (-chase_rate * dt).exp();
            *follower += (leader - *follower) * blend;
            leader = *follower;
            chase_rate *= TRAIL_CHASE_FALLOFF;
        }

        let length = |p: Point<f32>| (p.x * p.x + p.y * p.y).sqrt();
        let settled = self.drag_pointer.is_none()
            && length(self.velocity) < SETTLE_SPEED
            && length(displacement) < SETTLE_DISTANCE
            && length(self.trail[TRAIL_LEN - 1] - self.position) < SETTLE_DISTANCE
            && (self.radius_scale - 1.0).abs() < 0.005;
        if settled {
            self.position = self.target;
            self.velocity = point(0.0, 0.0);
            self.trail = [self.position; TRAIL_LEN];
            self.radius_scale = 1.0;
            self.animating = false;
            self.last_tick = None;
        }

        self.scale_factor = window.scale_factor();
        self.publish_state();
        cx.notify();
        if self.animating {
            self.schedule_tick(window, cx);
        }
    }

    fn handle_pointer_down(
        &mut self,
        event: &PointerDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.drag_pointer.is_some() {
            return;
        }
        // The droplet claims this touch; nothing beneath should react to it.
        cx.stop_propagation();
        self.drag_pointer = Some(event.pointer_id);
        self.target = point(f32::from(event.position.x), f32::from(event.position.y));
        platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
        self.start_animation(window, cx);
    }

    fn handle_pointer_move(&mut self, event: &PointerMoveEvent) {
        if self.drag_pointer == Some(event.pointer_id) {
            self.target = point(f32::from(event.position.x), f32::from(event.position.y));
        }
    }

    fn handle_pointer_release(&mut self, pointer_id: PointerId) {
        if self.drag_pointer == Some(pointer_id) {
            self.drag_pointer = None;
        }
    }
}

impl Render for DropletOverlay {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let position = self.position;
        div()
            .absolute()
            .inset_0()
            .when(self.enabled, |el| {
                el.child(
                    div()
                        .absolute()
                        .left(px(position.x - HIT_RADIUS))
                        .top(px(position.y - HIT_RADIUS))
                        .size(px(HIT_RADIUS * 2.0))
                        .rounded_full()
                        // Suppress hover/press/scroll for UI under the hit circle.
                        .occlude()
                        .on_pointer_down(cx.listener(|this, event, window, cx| {
                            this.handle_pointer_down(event, window, cx);
                        })),
                )
            })
            .when(self.enabled && self.drag_pointer.is_some(), |el| {
                el.child(
                    div()
                        .absolute()
                        .inset_0()
                        .occlude()
                        .on_pointer_move(cx.listener(|this, event: &PointerMoveEvent, _, _cx| {
                            this.handle_pointer_move(event);
                        }))
                        .on_pointer_up(cx.listener(|this, event: &PointerUpEvent, _, _cx| {
                            this.handle_pointer_release(event.pointer_id);
                        }))
                        .on_pointer_cancel(cx.listener(
                            |this, event: &PointerCancelEvent, _, _cx| {
                                this.handle_pointer_release(event.pointer_id);
                            },
                        )),
                )
            })
    }
}
