use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::prelude::FluentBuilder;
use gpui::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DrawerSide {
    Left,
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

/// Once a gesture's axis is determined, lock it for the rest of the touch.
#[derive(Clone, Copy, PartialEq)]
enum GestureAxis {
    Undecided,
    Horizontal, // drawer swipe
    Vertical,   // content scroll — ignore in drawer handler
}

// ---------------------------------------------------------------------------
// DrawerHost
// ---------------------------------------------------------------------------

pub struct DrawerHost {
    content: AnyView,
    /// Persistent drawer view (set once, slides in/out)
    drawer_view: Option<AnyView>,
    side: DrawerSide,
    width: Pixels,
    backdrop_opacity: f32,
    focus_handle: FocusHandle,
    // Gesture + animation state
    drawer_state: Arc<Mutex<DrawerState>>,
    /// Animation start offset
    snap_from: f32,
    /// Animation target offset (None = no animation in progress)
    snap_target: Option<f32>,
    /// When the current snap animation was started
    snap_started_at: Option<std::time::Instant>,
    /// Incremented each snap to retrigger with_animation
    animation_id: u64,
    /// Locked gesture axis for current touch sequence
    gesture_axis: GestureAxis,
    /// Accumulated deltas to decide axis before locking
    gesture_accum: (f32, f32),
    /// Width of the left-edge trigger zone for swipe-to-open
    edge_zone_width: Pixels,
    /// Most recent horizontal drag delta — used to bias snap direction
    last_drag_dx: f32,
}

impl DrawerHost {
    pub fn new(content: AnyView, cx: &mut Context<Self>) -> Self {
        Self {
            content,
            drawer_view: None,
            side: DrawerSide::Left,
            width: px(293.0),
            backdrop_opacity: 0.4,
            focus_handle: cx.focus_handle(),
            drawer_state: Arc::new(Mutex::new(DrawerState::default())),
            snap_from: 0.0,
            snap_target: None,
            snap_started_at: None,
            animation_id: 0,
            gesture_axis: GestureAxis::Undecided,
            gesture_accum: (0.0, 0.0),
            edge_zone_width: px(40.0),
            last_drag_dx: 0.0,
        }
    }

    /// Pre-register the drawer view. It persists across open/close cycles.
    pub fn set_drawer(&mut self, view: AnyView) {
        self.drawer_view = Some(view);
    }

    /// Animate the drawer open (slide-in).
    pub fn open(&mut self, cx: &mut Context<Self>) {
        let w = f32::from(self.width);
        self.start_snap(w, cx);
        cx.emit(DrawerEvent::Opened);
    }

    /// Animate the drawer closed (slide-out).
    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.start_snap(0.0, cx);
        cx.emit(DrawerEvent::Closed);
    }

    pub fn is_open(&self) -> bool {
        let offset = self.drawer_state.lock().map(|s| s.offset).unwrap_or(0.0);
        offset > 0.0 || self.snap_target.map_or(false, |t| t > 0.0)
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

    pub fn set_edge_zone_width(&mut self, width: Pixels) {
        self.edge_zone_width = width;
    }

    /// Start a snap animation to the given target offset.
    fn start_snap(&mut self, target: f32, cx: &mut Context<Self>) {
        let current = self.drawer_state.lock().map(|s| s.offset).unwrap_or(0.0);

        // Skip animation when already at (or very near) target — avoids a ghost
        // overlay window where the backdrop stays alive but invisible.
        if (current - target).abs() < 1.0 {
            if let Ok(mut state) = self.drawer_state.lock() {
                state.offset = target;
                state.is_dragging = false;
            }
            self.snap_target = None;
            self.snap_started_at = None;
            cx.notify();
            return;
        }

        if let Ok(mut state) = self.drawer_state.lock() {
            state.offset = target;
            state.is_dragging = false;
        }
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
            if started.elapsed() >= Duration::from_millis(280) {
                self.snap_target = None;
                self.snap_started_at = None;
            }
        }

        let content = self.content.clone();
        let drawer_view = self.drawer_view.clone();
        let has_drawer = drawer_view.is_some();
        let drawer_width = f32::from(self.width);
        let max_opacity = self.backdrop_opacity;
        let edge_zone_width = self.edge_zone_width;

        // Read drawer state
        let (drawer_offset, is_dragging) = self
            .drawer_state
            .lock()
            .map(|s| (s.offset, s.is_dragging))
            .unwrap_or((0.0, false));

        let is_open = drawer_offset > 0.0;
        let snap_target = self.snap_target;
        let snap_from = self.snap_from;
        let animation_id = self.animation_id;
        let animating = snap_target.is_some() && !is_dragging;

        // Edge zone shows when drawer is closed and not animating open
        let show_edge = has_drawer && !is_open && !snap_target.map_or(false, |t| t > 0.0);
        // Overlay shows when drawer is visible or animating
        let show_overlay = has_drawer && (is_open || snap_target.is_some());

        div()
            .track_focus(&self.focus_handle)
            .size_full()
            // Mouse up handler: reset gesture axis and snap on release
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _event: &MouseUpEvent, _window, cx| {
                    this.gesture_axis = GestureAxis::Undecided;
                    this.gesture_accum = (0.0, 0.0);

                    let was_dragging = this
                        .drawer_state
                        .lock()
                        .map(|s| s.is_dragging)
                        .unwrap_or(false);

                    if was_dragging {
                        let current =
                            this.drawer_state.lock().map(|s| s.offset).unwrap_or(0.0);
                        let width = f32::from(this.width);
                        let last_dx = this.last_drag_dx;

                        // Use drag direction to bias the snap decision:
                        // - Swiping left (dx < -2): snap closed regardless of position
                        // - Swiping right (dx > 2): snap open regardless of position
                        // - Slow/stopped: fall back to midpoint threshold
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
            // Left-edge swipe trigger zone — invisible, sits on top of content
            .when(show_edge, |el| {
                el.child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .bottom_0()
                        .w(edge_zone_width)
                        .on_scroll_wheel(cx.listener(
                            |this, event: &ScrollWheelEvent, _window, cx| {
                                let (dx, dy) = match event.delta {
                                    ScrollDelta::Pixels(p) => {
                                        (f32::from(p.x), f32::from(p.y))
                                    }
                                    ScrollDelta::Lines(l) => (l.x * 20.0, l.y * 20.0),
                                };

                                if this.gesture_axis == GestureAxis::Vertical {
                                    return;
                                }

                                let width = f32::from(this.width);

                                if this.gesture_axis == GestureAxis::Horizontal {
                                    this.snap_target = None;
                                    this.snap_started_at = None;
                                    this.last_drag_dx = dx;
                                    if let Ok(mut state) = this.drawer_state.lock() {
                                        state.is_dragging = true;
                                        state.offset =
                                            (state.offset + dx).clamp(0.0, width);
                                    }
                                    cx.notify();
                                    return;
                                }

                                // Undecided — accumulate then lock axis
                                this.gesture_accum.0 += dx;
                                this.gesture_accum.1 += dy;
                                let (ax, ay) = (
                                    this.gesture_accum.0.abs(),
                                    this.gesture_accum.1.abs(),
                                );

                                if ax + ay > 6.0 {
                                    if ax > ay {
                                        this.gesture_axis = GestureAxis::Horizontal;
                                        this.snap_target = None;
                                        this.snap_started_at = None;
                                        if let Ok(mut state) = this.drawer_state.lock()
                                        {
                                            state.is_dragging = true;
                                            state.offset = (state.offset
                                                + this.gesture_accum.0)
                                                .clamp(0.0, width);
                                        }
                                        cx.notify();
                                    } else {
                                        this.gesture_axis = GestureAxis::Vertical;
                                    }
                                }
                            },
                        )),
                )
            })
            // Drawer overlay: backdrop + panel (when offset > 0 or animating)
            .when(show_overlay, |el| {
                let drawer_view = match drawer_view {
                    Some(v) => v,
                    None => return el,
                };

                // Backdrop — covers full area, tappable to close, swipeable
                let backdrop = div()
                    .absolute()
                    .inset_0()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            cx.emit(DrawerEvent::BackdropTapped);
                            this.close(cx);
                        }),
                    )
                    .on_scroll_wheel(cx.listener(
                        |this, event: &ScrollWheelEvent, _window, cx| {
                            let dx = match event.delta {
                                ScrollDelta::Pixels(p) => f32::from(p.x),
                                ScrollDelta::Lines(l) => l.x * 20.0,
                            };
                            if dx.abs() > 1.0 {
                                let width = f32::from(this.width);
                                this.snap_target = None;
                                this.snap_started_at = None;
                                this.last_drag_dx = dx;
                                if let Ok(mut state) = this.drawer_state.lock() {
                                    state.is_dragging = true;
                                    state.offset =
                                        (state.offset + dx).clamp(0.0, width);
                                    // If dragged all the way to 0, close immediately
                                    // so the overlay disappears and doesn't ghost-catch
                                    // the next touch.
                                    if state.offset <= 0.0 {
                                        state.is_dragging = false;
                                    }
                                }
                                cx.notify();
                            }
                        },
                    ));

                let backdrop: AnyElement = if animating {
                    let from = snap_from;
                    let target = snap_target.unwrap();
                    backdrop
                        .with_animation(
                            ElementId::NamedInteger(
                                "drawer-backdrop-snap".into(),
                                animation_id,
                            ),
                            Animation::new(Duration::from_millis(250))
                                .with_easing(ease_out_quint()),
                            move |elem, delta| {
                                let o = from + (target - from) * delta;
                                let opacity = (o / drawer_width * max_opacity)
                                    .clamp(0.0, max_opacity);
                                elem.bg(hsla(0.0, 0.0, 0.0, opacity))
                            },
                        )
                        .into_any_element()
                } else {
                    let opacity = (drawer_offset / drawer_width * max_opacity)
                        .clamp(0.0, max_opacity);
                    backdrop
                        .bg(hsla(0.0, 0.0, 0.0, opacity))
                        .into_any_element()
                };

                // Drawer panel
                let panel = div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .w(px(drawer_width))
                    .bg(rgb(0x0e0c0c))
                    .child(drawer_view);

                let panel: AnyElement = if animating {
                    let from = snap_from;
                    let target = snap_target.unwrap();
                    panel
                        .with_animation(
                            ElementId::NamedInteger(
                                "drawer-panel-snap".into(),
                                animation_id,
                            ),
                            Animation::new(Duration::from_millis(250))
                                .with_easing(ease_out_quint()),
                            move |elem, delta| {
                                let o = from + (target - from) * delta;
                                elem.left(px(o - drawer_width))
                            },
                        )
                        .into_any_element()
                } else {
                    panel
                        .left(px(drawer_offset - drawer_width))
                        .into_any_element()
                };

                el.child(
                    deferred(
                        div()
                            .absolute()
                            .inset_0()
                            .child(backdrop)
                            .child(panel),
                    )
                    .with_priority(998),
                )
            })
    }
}
