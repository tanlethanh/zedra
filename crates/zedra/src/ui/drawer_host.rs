use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui::prelude::FluentBuilder;
use gpui::*;

use crate::{platform_bridge, theme};

/// Global flag: true when any drawer overlay is visible.
/// Used to suppress input (e.g. keyboard) behind the overlay.
static DRAWER_OVERLAY_VISIBLE: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DrawerSide {
    Left,
    Right,
}

impl Default for DrawerSide {
    fn default() -> Self {
        Self::Left
    }
}

#[derive(Clone, Debug)]
pub enum DrawerEvent {
    Opened,
    Closed,
    BackdropTapped,
}

// ---------------------------------------------------------------------------
// Gesture-driven drawer state
// ---------------------------------------------------------------------------

struct DrawerState {
    /// Current drawer offset (0 = closed, width = fully open)
    offset: f32,
    /// Whether a drag gesture is in progress
    is_dragging: bool,
}

impl Default for DrawerState {
    fn default() -> Self {
        Self {
            offset: 0.0,
            is_dragging: false,
        }
    }
}

pub struct DrawerHost {
    content: AnyView,
    drawer: AnyView,
    side: DrawerSide,
    width: Pixels,
    backdrop_opacity: f32,
    focus_handle: FocusHandle,
    // Gesture + animation state
    drawer_state: DrawerState,
    /// Animation start offset
    snap_from: f32,
    /// Animation target offset (None = no animation in progress)
    snap_target: Option<f32>,
    /// When the current snap animation was started
    snap_started_at: Option<std::time::Instant>,
    /// Incremented each snap to retrigger with_animation
    animation_id: u64,
    /// Most recent horizontal drag delta — used to bias snap direction
    last_drag_dx: f32,
}

impl DrawerHost {
    pub fn new(
        content: AnyView,
        drawer: AnyView,
        side: DrawerSide,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            content,
            drawer,
            side,
            width: px(theme::DRAWER_DEFAULT_WIDTH),
            backdrop_opacity: 0.4,
            focus_handle: cx.focus_handle(),
            drawer_state: DrawerState::default(),
            snap_from: 0.0,
            snap_target: None,
            snap_started_at: None,
            animation_id: 0,
            last_drag_dx: 0.0,
        }
    }

    pub fn set_content(&mut self, content: AnyView) {
        self.content = content;
    }

    /// Pre-register the drawer view. It persists across open/close cycles.
    pub fn set_drawer(&mut self, drawer: AnyView) {
        self.drawer = drawer;
    }

    /// Animate the drawer open (slide-in).
    ///
    /// Emits `DrawerEvent::Opened` immediately (before the animation completes)
    /// so callers can update state (e.g. load git status) without waiting for
    /// the visual transition to finish.
    pub fn open(&mut self, cx: &mut Context<Self>) {
        let w = f32::from(self.width);
        DRAWER_OVERLAY_VISIBLE.store(true, Ordering::Relaxed);
        self.start_snap(w, cx);
        cx.emit(DrawerEvent::Opened);
    }

    /// Animate the drawer closed (slide-out).
    ///
    /// Emits `DrawerEvent::Closed` immediately (before the animation completes).
    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.start_snap(0.0, cx);
        cx.emit(DrawerEvent::Closed);
    }

    pub fn is_open(&self) -> bool {
        self.drawer_state.offset > 0.0 || self.snap_target.map_or(false, |t| t > 0.0)
    }

    pub fn set_side(&mut self, side: DrawerSide) {
        self.side = side;
    }

    pub fn set_width(&mut self, width: Pixels) {
        self.width = width;
    }

    pub fn set_backdrop_opacity(&mut self, opacity: f32) {
        self.backdrop_opacity = opacity;
    }

    /// Start a snap animation to the given target offset.
    fn start_snap(&mut self, target: f32, cx: &mut Context<Self>) {
        let current = self.drawer_state.offset;

        // Skip animation when already at (or very near) target — avoids a ghost
        // overlay window where the backdrop stays alive but invisible.
        if (current - target).abs() < 1.0 {
            self.drawer_state.offset = target;
            self.drawer_state.is_dragging = false;
            self.snap_target = None;
            self.snap_started_at = None;
            cx.notify();
            return;
        }

        self.drawer_state.offset = target;
        self.drawer_state.is_dragging = false;
        self.snap_from = current;
        self.snap_target = Some(target);
        self.snap_started_at = Some(std::time::Instant::now());
        self.animation_id += 1;
        cx.notify();
    }
}

impl EventEmitter<DrawerEvent> for DrawerHost {}

impl Focusable for DrawerHost {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for DrawerHost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Clear completed snap animations (animation duration is 250ms, give 280ms margin)
        if let Some(started) = self.snap_started_at {
            if started.elapsed() >= Duration::from_millis(theme::DRAWER_ANIMATION_DURATION_MS + 30)
            {
                self.snap_target = None;
                self.snap_started_at = None;
                // Drawer is now fully closed — update global overlay flag
                if self.drawer_state.offset <= 0.0 {
                    DRAWER_OVERLAY_VISIBLE.store(false, Ordering::Relaxed);
                }
            }
        }

        let content = self.content.clone();
        let drawer = self.drawer.clone();
        let drawer_width = f32::from(self.width);
        let max_opacity = self.backdrop_opacity;

        let drawer_offset = self.drawer_state.offset;
        let is_dragging = self.drawer_state.is_dragging;

        let is_open = drawer_offset > 0.0;
        let snap_target = self.snap_target;
        let snap_from = self.snap_from;
        let animation_id = self.animation_id;
        let animating = snap_target.is_some() && !is_dragging;

        // Drawer overlay (backdrop + panel) shows when drawer is visible, being
        // dragged, or animating. Including is_dragging prevents the occluding
        // overlay from disappearing mid-gesture when the offset hits 0.
        let show_overlay = is_open || is_dragging || snap_target.is_some();

        div()
            .track_focus(&self.focus_handle)
            .size_full()
            // Horizontal scroll events drive drawer open/close on both platforms.
            // iOS pan gestures and Android touch drags both arrive as ScrollWheelEvent;
            // dx.abs() > dy.abs() selects the drawer path over content scroll.
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, window, cx| {
                if this.snap_target.is_some() {
                    return;
                }
                let delta = event.delta.pixel_delta(window.line_height());
                let dx = f32::from(delta.x);
                let dy = f32::from(delta.y);
                let pos_x = f32::from(event.position.x);
                if dx.abs() <= dy.abs() {
                    return;
                }
                let vw = f32::from(window.viewport_size().width);
                let (eff_dx, edge_ok) = match this.side {
                    DrawerSide::Left => (
                        dx,
                        pos_x < theme::DRAWER_EDGE_ZONE || this.drawer_state.offset > 0.0,
                    ),
                    DrawerSide::Right => (
                        -dx,
                        pos_x > vw - theme::DRAWER_EDGE_ZONE || this.drawer_state.offset > 0.0,
                    ),
                };
                if !edge_ok {
                    return;
                }
                // Horizontal swipe is driving the drawer — dismiss keyboard.
                platform_bridge::bridge().hide_keyboard();
                let width = f32::from(this.width);
                let current = this.drawer_state.offset;
                if current <= 0.0 && eff_dx <= 0.0 {
                    return;
                }
                this.last_drag_dx = eff_dx;
                this.drawer_state.is_dragging = true;
                this.drawer_state.offset = (this.drawer_state.offset + eff_dx).clamp(0.0, width);
                let new_offset = this.drawer_state.offset;
                const VELOCITY_THRESHOLD: f32 = theme::DRAWER_VELOCITY_THRESHOLD;
                let position_threshold = width * 0.3;
                if eff_dx > 0.0 && (new_offset > position_threshold || eff_dx > VELOCITY_THRESHOLD)
                {
                    this.start_snap(width, cx);
                    cx.emit(DrawerEvent::Opened);
                } else if eff_dx < 0.0
                    && (new_offset < width - position_threshold
                        || eff_dx.abs() > VELOCITY_THRESHOLD)
                {
                    this.start_snap(0.0, cx);
                    cx.emit(DrawerEvent::Closed);
                } else {
                    cx.notify();
                }
            }))
            // Mouse up handler: snap drawer on release
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _event: &MouseUpEvent, _window, cx| {
                    if this.drawer_state.is_dragging {
                        let current = this.drawer_state.offset;
                        let width = f32::from(this.width);
                        let last_dx = this.last_drag_dx;

                        let target = if last_dx < -2.0 {
                            0.0
                        } else if last_dx > 2.0 {
                            width
                        } else if current > width / 2.0 {
                            width
                        } else {
                            0.0
                        };
                        this.last_drag_dx = 0.0;
                        this.start_snap(target, cx);
                        if target == 0.0 {
                            cx.emit(DrawerEvent::Closed);
                        } else {
                            cx.emit(DrawerEvent::Opened);
                        }
                    }
                }),
            )
            // Content — always rendered full width
            .child(content)
            // Drawer overlay: backdrop + panel (when offset > 0 or animating).
            // .occlude() on the container blocks events from reaching main content.
            .when(show_overlay, |el| {
                // Backdrop — covers full area, tappable to close.
                // When the drawer is closed/closing (offset=0), we return early so
                // events fall through to the content behind the overlay.
                // When the drawer is open, stop_propagation() blocks content buttons.
                let backdrop = div().absolute().inset_0().on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, event: &MouseDownEvent, window, cx| {
                        let offset = this.drawer_state.offset;
                        // Drawer closed or closing — let events through to content
                        if offset <= 0.0 {
                            return;
                        }
                        // Tap inside the panel area — panel's own .occlude() handles it
                        let inside_panel = match this.side {
                            DrawerSide::Left => f32::from(event.position.x) < offset,
                            DrawerSide::Right => {
                                let vw = f32::from(window.viewport_size().width);
                                f32::from(event.position.x) > vw - offset
                            }
                        };
                        if inside_panel {
                            return;
                        }
                        // Backdrop tap: block content behind from firing, close drawer
                        cx.stop_propagation();
                        platform_bridge::bridge().hide_keyboard();
                        cx.emit(DrawerEvent::BackdropTapped);
                        this.close(cx);
                    }),
                );

                let backdrop: AnyElement = if animating {
                    let from = snap_from;
                    let target = snap_target.unwrap();
                    backdrop
                        .with_animation(
                            ElementId::NamedInteger("drawer-backdrop-snap".into(), animation_id),
                            Animation::new(Duration::from_millis(
                                theme::DRAWER_ANIMATION_DURATION_MS,
                            ))
                            .with_easing(ease_out_quint()),
                            move |elem, delta| {
                                let o = from + (target - from) * delta;
                                let opacity =
                                    (o / drawer_width * max_opacity).clamp(0.0, max_opacity);
                                elem.bg(hsla(0.0, 0.0, 0.0, opacity))
                            },
                        )
                        .into_any_element()
                } else {
                    let opacity =
                        (drawer_offset / drawer_width * max_opacity).clamp(0.0, max_opacity);
                    backdrop.bg(hsla(0.0, 0.0, 0.0, opacity)).into_any_element()
                };

                // Drawer panel — .occlude() prevents events inside the panel
                // from leaking to the backdrop's tap handler.
                let panel = div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .w(px(drawer_width))
                    .bg(rgb(0x0e0c0c))
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .occlude()
                    .child(drawer);

                let side = self.side;
                let panel: AnyElement = if animating {
                    let from = snap_from;
                    let target = snap_target.unwrap();
                    panel
                        .with_animation(
                            ElementId::NamedInteger("drawer-panel-snap".into(), animation_id),
                            Animation::new(Duration::from_millis(
                                theme::DRAWER_ANIMATION_DURATION_MS,
                            ))
                            .with_easing(ease_out_quint()),
                            move |elem, delta| {
                                let o = from + (target - from) * delta;
                                match side {
                                    DrawerSide::Left => elem.left(px(o - drawer_width)),
                                    DrawerSide::Right => elem.right(px(o - drawer_width)),
                                }
                            },
                        )
                        .into_any_element()
                } else {
                    match side {
                        DrawerSide::Left => panel
                            .left(px(drawer_offset - drawer_width))
                            .into_any_element(),
                        DrawerSide::Right => panel
                            .right(px(drawer_offset - drawer_width))
                            .into_any_element(),
                    }
                };

                el.child(
                    deferred(
                        div()
                            .absolute()
                            .inset_0()
                            // No .occlude() here: during close animation (offset=0, snap_target live)
                            // we want events to reach content. The backdrop's on_mouse_down calls
                            // stop_propagation() when the drawer is actually open (offset > 0).
                            .child(backdrop)
                            .child(panel),
                    )
                    .with_priority(998),
                )
            })
    }
}
