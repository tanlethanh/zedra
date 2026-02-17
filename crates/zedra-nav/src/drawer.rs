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

// ---------------------------------------------------------------------------
// Drawer gesture bridge — android_app.rs writes, DrawerHost reads
// ---------------------------------------------------------------------------
// DrawerPan events bypass GPUI scroll dispatch entirely. android_app.rs
// pushes horizontal deltas here; DrawerHost drains them each render frame.
// This avoids a full-screen overlay that would intercept content scroll.

struct DrawerGestureBridge {
    /// Accumulated horizontal delta since last drain
    pending_dx: f32,
    /// Most recent individual delta (for snap direction bias)
    last_dx: f32,
    /// Whether a drawer pan gesture is currently active
    dragging: bool,
}

static DRAWER_BRIDGE: Mutex<DrawerGestureBridge> = Mutex::new(DrawerGestureBridge {
    pending_dx: 0.0,
    last_dx: 0.0,
    dragging: false,
});

/// Push a horizontal delta from the gesture arena (called from android_app.rs).
pub fn push_drawer_pan_delta(dx: f32) {
    if let Ok(mut bridge) = DRAWER_BRIDGE.lock() {
        bridge.pending_dx += dx;
        bridge.last_dx = dx;
        bridge.dragging = true;
    }
}

/// Reset the bridge (called from android_app.rs on ACTION_DOWN).
pub fn reset_drawer_gesture() {
    if let Ok(mut bridge) = DRAWER_BRIDGE.lock() {
        bridge.pending_dx = 0.0;
        bridge.last_dx = 0.0;
        bridge.dragging = false;
    }
}

/// Check whether a drawer pan gesture is currently active.
pub fn is_drawer_pan_active() -> bool {
    DRAWER_BRIDGE
        .lock()
        .map(|b| b.dragging)
        .unwrap_or(false)
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

    /// Replace the main content view.
    pub fn set_content(&mut self, content: AnyView) {
        self.content = content;
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

        // Drain pending drawer gesture deltas pushed by android_app.rs.
        // This runs before reading drawer_state so show_overlay sees the
        // up-to-date is_dragging flag.
        //
        // Auto-snap: when the offset crosses a position threshold (30% of
        // width) or the per-frame velocity exceeds 6px, the drawer auto-
        // completes its open/close animation without waiting for finger lift.
        let mut auto_snap_target: Option<f32> = None;
        if let Ok(mut bridge) = DRAWER_BRIDGE.lock() {
            if bridge.pending_dx.abs() > 0.0 {
                // If a snap animation is already running (e.g. from a
                // previous auto-snap), discard incoming deltas so the
                // animation plays to completion undisturbed.
                if self.snap_target.is_some() {
                    bridge.pending_dx = 0.0;
                } else {
                    let width = f32::from(self.width);
                    // Skip no-op: drawer already closed and swiping further left
                    let current = self
                        .drawer_state
                        .lock()
                        .map(|s| s.offset)
                        .unwrap_or(0.0);
                    if !(current <= 0.0 && bridge.pending_dx <= 0.0) {
                        self.snap_target = None;
                        self.snap_started_at = None;
                        self.last_drag_dx = bridge.last_dx;
                        let new_offset;
                        if let Ok(mut state) = self.drawer_state.lock() {
                            state.is_dragging = true;
                            state.offset =
                                (state.offset + bridge.pending_dx).clamp(0.0, width);
                            new_offset = state.offset;
                        } else {
                            new_offset = current;
                        }

                        // Auto-snap thresholds
                        const VELOCITY_THRESHOLD: f32 = 6.0; // px/frame
                        let position_threshold = width * 0.3;
                        let velocity = bridge.last_dx;

                        if velocity > 0.0
                            && (new_offset > position_threshold
                                || velocity > VELOCITY_THRESHOLD)
                        {
                            // Swiping right — auto-open
                            auto_snap_target = Some(width);
                            bridge.dragging = false;
                        } else if velocity < 0.0
                            && (new_offset < width - position_threshold
                                || velocity.abs() > VELOCITY_THRESHOLD)
                        {
                            // Swiping left — auto-close
                            auto_snap_target = Some(0.0);
                            bridge.dragging = false;
                        }
                    }
                    bridge.pending_dx = 0.0;
                }
            }
        }

        // Trigger auto-snap outside the lock
        if let Some(target) = auto_snap_target {
            let width = f32::from(self.width);
            self.start_snap(target, cx);
            if target < width * 0.5 {
                cx.emit(DrawerEvent::Closed);
            } else {
                cx.emit(DrawerEvent::Opened);
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
            // Mouse up handler: snap drawer on release
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _event: &MouseUpEvent, _window, cx| {
                    // Check both sources: DrawerState (set during render) and
                    // the global bridge (set immediately by push_drawer_pan_delta).
                    let state_dragging = this
                        .drawer_state
                        .lock()
                        .map(|s| s.is_dragging)
                        .unwrap_or(false);
                    let was_dragging = state_dragging || is_drawer_pan_active();

                    if was_dragging {
                        // Drain any remaining bridge delta before snapping
                        if let Ok(mut bridge) = DRAWER_BRIDGE.lock() {
                            if bridge.pending_dx.abs() > 0.0 {
                                let width = f32::from(this.width);
                                this.last_drag_dx = bridge.last_dx;
                                if let Ok(mut state) = this.drawer_state.lock() {
                                    state.offset =
                                        (state.offset + bridge.pending_dx).clamp(0.0, width);
                                }
                                bridge.pending_dx = 0.0;
                            } else if bridge.last_dx.abs() > this.last_drag_dx.abs() {
                                this.last_drag_dx = bridge.last_dx;
                            }
                            bridge.dragging = false;
                        }
                        let current =
                            this.drawer_state.lock().map(|s| s.offset).unwrap_or(0.0);
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
            // Drawer overlay: backdrop + panel (when offset > 0 or animating)
            // .occlude() on container blocks ALL events from reaching main content.
            // DrawerPan gestures bypass GPUI scroll dispatch — android_app.rs
            // pushes deltas to the global DRAWER_BRIDGE, drained above in render.
            .when(show_overlay, |el| {
                let drawer_view = match drawer_view {
                    Some(v) => v,
                    None => return el,
                };

                // Backdrop — covers full area, tappable to close.
                // Scroll handling is done by the drawer bridge (not GPUI).
                let backdrop = div()
                    .absolute()
                    .inset_0()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _window, cx| {
                            let offset =
                                this.drawer_state.lock().map(|s| s.offset).unwrap_or(0.0);
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
                            .occlude()
                            .child(backdrop)
                            .child(panel),
                    )
                    .with_priority(998),
                )
            })
    }
}
