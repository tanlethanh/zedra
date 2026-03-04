use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::prelude::FluentBuilder;
use gpui::*;

/// Global flag: true when any drawer overlay is visible.
/// Used to suppress input (e.g. keyboard) behind the overlay.
static DRAWER_OVERLAY_VISIBLE: AtomicBool = AtomicBool::new(false);

pub fn is_drawer_overlay_visible() -> bool {
    DRAWER_OVERLAY_VISIBLE.load(Ordering::Relaxed)
}


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

        // Drawer overlay (backdrop + panel) shows when drawer is visible, being
        // dragged, or animating. Including is_dragging prevents the occluding
        // overlay from disappearing mid-gesture when the offset hits 0.
        let show_overlay = has_drawer && (is_open || is_dragging || snap_target.is_some());
        DRAWER_OVERLAY_VISIBLE.store(show_overlay, Ordering::Relaxed);

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
                if dx.abs() <= dy.abs() {
                    return;
                }
                const EDGE_ZONE: f32 = 44.0;
                let pos_x = f32::from(event.position.x);
                let drawer_open =
                    this.drawer_state.lock().map(|s| s.offset > 0.0).unwrap_or(false);
                if !drawer_open && pos_x >= EDGE_ZONE {
                    return;
                }
                let width = f32::from(this.width);
                let current = this.drawer_state.lock().map(|s| s.offset).unwrap_or(0.0);
                if current <= 0.0 && dx <= 0.0 {
                    return;
                }
                this.last_drag_dx = dx;
                let new_offset = if let Ok(mut state) = this.drawer_state.lock() {
                    state.is_dragging = true;
                    state.offset = (state.offset + dx).clamp(0.0, width);
                    state.offset
                } else {
                    (current + dx).clamp(0.0, width)
                };
                const VELOCITY_THRESHOLD: f32 = 6.0;
                let position_threshold = width * 0.3;
                if dx > 0.0 && (new_offset > position_threshold || dx > VELOCITY_THRESHOLD) {
                    this.start_snap(width, cx);
                    cx.emit(DrawerEvent::Opened);
                } else if dx < 0.0
                    && (new_offset < width - position_threshold
                        || dx.abs() > VELOCITY_THRESHOLD)
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
                    let was_dragging = this
                        .drawer_state
                        .lock()
                        .map(|s| s.is_dragging)
                        .unwrap_or(false);

                    if was_dragging {
                        let current = this.drawer_state.lock().map(|s| s.offset).unwrap_or(0.0);
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
                let drawer_view = match drawer_view {
                    Some(v) => v,
                    None => return el,
                };

                // Backdrop — covers full area, tappable to close.
                let backdrop = div().absolute().inset_0().on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, event: &MouseDownEvent, _window, cx| {
                        let offset = this.drawer_state.lock().map(|s| s.offset).unwrap_or(0.0);
                        if f32::from(event.position.x) < offset {
                            return;
                        }
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
                            Animation::new(Duration::from_millis(250))
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
                    .child(drawer_view);

                let panel: AnyElement = if animating {
                    let from = snap_from;
                    let target = snap_target.unwrap();
                    panel
                        .with_animation(
                            ElementId::NamedInteger("drawer-panel-snap".into(), animation_id),
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
                            .occlude()
                            .child(backdrop)
                            .child(panel),
                    )
                    .with_priority(998),
                )
            })
    }
}
