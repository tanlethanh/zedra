use std::time::Duration;

use gpui::prelude::FluentBuilder;
use gpui::*;

use crate::theme;

const DRAWER_DRAG_START_THRESHOLD: f32 = 10.0;
const DRAWER_VERTICAL_CANCEL_RATIO: f32 = 1.25;

#[derive(Clone, Copy, Debug, PartialEq)]
enum DragOrigin {
    Panel,
    Edge,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum GestureState {
    Idle,
    Pending {
        pointer_id: PointerId,
        start: Point<Pixels>,
        origin: DragOrigin,
    },
    Dragging {
        pointer_id: PointerId,
    },
}

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

#[derive(Clone, Copy, Debug, PartialEq)]
enum DrawerState {
    Opened,
    Snapping,
    Closing,
    Closed,
}

pub struct DrawerHost {
    content: AnyView,
    drawer: AnyView,
    drawer_side: DrawerSide,
    drawer_state: DrawerState,
    drawer_width: Pixels,
    /// Current visual offset: 0.0 = fully closed, drawer_width px = fully open.
    drawer_offset: f32,
    backdrop_opacity: f32,
    focus_handle: FocusHandle,
    /// Width of the screen-edge strip that acts as the drag-to-open initiator.
    edge_inset: f32,
    /// Captured offset at animation start.
    snap_from: f32,
    /// Animation target. `Some` while an animation is in progress.
    snap_target: Option<f32>,
    /// When the current snap animation was started.
    snap_started_at: Option<std::time::Instant>,
    /// Incremented on each snap to give `with_animation` a fresh `ElementId`.
    animation_id: u64,
    /// Last absolute touch x position for the active direct manipulation.
    last_drag_x: f32,
    /// Last effective delta (sign-normalised to opening direction).
    last_drag_dx: f32,
    gesture_state: GestureState,
    /// Dropping this cancels the pending state commit task.
    _snap_task: Option<Task<()>>,
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
            drawer_side: side,
            drawer_width: px(theme::DRAWER_DEFAULT_WIDTH),
            backdrop_opacity: 0.4,
            focus_handle: cx.focus_handle(),
            drawer_state: DrawerState::Closed,
            drawer_offset: 0.0,
            edge_inset: theme::DRAWER_EDGE_ZONE,
            snap_from: 0.0,
            snap_target: None,
            snap_started_at: None,
            animation_id: 0,
            last_drag_x: 0.0,
            last_drag_dx: 0.0,
            gesture_state: GestureState::Idle,
            _snap_task: None,
        }
    }

    pub fn set_content(&mut self, content: AnyView) {
        self.content = content;
    }
    pub fn set_drawer(&mut self, drawer: AnyView) {
        self.drawer = drawer;
    }
    pub fn set_side(&mut self, side: DrawerSide) {
        self.drawer_side = side;
    }
    pub fn set_width(&mut self, width: Pixels) {
        self.drawer_width = width;
    }
    pub fn set_backdrop_opacity(&mut self, opacity: f32) {
        self.backdrop_opacity = opacity;
    }

    pub fn open(&mut self, cx: &mut Context<Self>) {
        self.open_impl(None, cx);
    }

    pub fn open_with_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_impl(Some(window), cx);
    }

    fn open_impl(&mut self, window: Option<&mut Window>, cx: &mut Context<Self>) {
        if self.is_snap_animating() {
            return;
        }
        let w = f32::from(self.drawer_width);
        self.start_snap(w, window, cx);
        cx.emit(DrawerEvent::Opened);
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.close_impl(None, cx);
    }

    pub fn close_with_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_impl(Some(window), cx);
    }

    fn close_impl(&mut self, window: Option<&mut Window>, cx: &mut Context<Self>) {
        if self.is_snap_animating() {
            return;
        }
        self.start_snap(0.0, window, cx);
        cx.emit(DrawerEvent::Closed);
    }

    pub fn is_open(&self) -> bool {
        matches!(
            self.drawer_state,
            DrawerState::Opened | DrawerState::Snapping
        )
    }

    pub fn is_dragging(&self) -> bool {
        matches!(self.gesture_state, GestureState::Dragging { .. })
    }

    fn is_snap_animating(&self) -> bool {
        self.snap_target.is_some()
    }

    fn snap_duration_ms(from: f32, target: f32) -> u64 {
        if target > from {
            theme::DRAWER_OPEN_ANIMATION_DURATION_MS
        } else {
            theme::DRAWER_CLOSE_ANIMATION_DURATION_MS
        }
    }

    fn snap_animation(&self, from: f32, target: f32) -> Animation {
        let animation = Animation::new(Duration::from_millis(Self::snap_duration_ms(from, target)));
        if target > from {
            animation.with_easing(ease_out_quint())
        } else {
            animation.with_easing(ease_in_out)
        }
    }

    fn hide_soft_keyboard(window: Option<&mut Window>) {
        if let Some(window) = window {
            window.hide_soft_keyboard();
        }
    }

    /// Dispatches a pointer move to the active gesture, if any.
    fn handle_pointer_move(
        &mut self,
        pointer_id: PointerId,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.gesture_state {
            GestureState::Pending {
                pointer_id: pid, ..
            } if pid == pointer_id => self.update_pending_drag(pointer_id, position, window, cx),
            GestureState::Dragging { pointer_id: pid } if pid == pointer_id => {
                self.update_direct_drag(f32::from(position.x), cx)
            }
            _ => {}
        }
    }

    /// Dispatches pointer up or cancel to the active gesture, if any.
    fn handle_pointer_release(
        &mut self,
        pointer_id: PointerId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.gesture_state {
            GestureState::Pending {
                pointer_id: pid, ..
            } if pid == pointer_id => {
                self.gesture_state = GestureState::Idle;
                cx.notify();
            }
            GestureState::Dragging { pointer_id: pid } if pid == pointer_id => {
                self.end_direct_drag(window, cx)
            }
            _ => {}
        }
    }

    fn update_pending_drag(
        &mut self,
        pointer_id: PointerId,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let GestureState::Pending {
            pointer_id: pending_id,
            start,
            origin,
        } = self.gesture_state
        else {
            return;
        };
        if pending_id != pointer_id {
            return;
        }

        let dx = f32::from(position.x - start.x);
        let dy = f32::from(position.y - start.y);
        let abs_dx = dx.abs();
        let abs_dy = dy.abs();
        let opening_dx = match self.drawer_side {
            DrawerSide::Left => dx,
            DrawerSide::Right => -dx,
        };

        if abs_dx < DRAWER_DRAG_START_THRESHOLD && abs_dy < DRAWER_DRAG_START_THRESHOLD {
            return;
        }

        // Cancel only when clearly vertical — early samples carry noise.
        if abs_dy > DRAWER_DRAG_START_THRESHOLD && abs_dy > abs_dx * DRAWER_VERTICAL_CANCEL_RATIO {
            self.gesture_state = GestureState::Idle;
            cx.notify();
            return;
        }

        let promote = match origin {
            DragOrigin::Panel => opening_dx < -DRAWER_DRAG_START_THRESHOLD && abs_dx >= abs_dy,
            DragOrigin::Edge => opening_dx > DRAWER_DRAG_START_THRESHOLD && abs_dx >= abs_dy,
        };

        if promote {
            self.begin_direct_drag(pointer_id, f32::from(start.x), window, cx);
            self.update_direct_drag(f32::from(position.x), cx);
        }
    }

    fn begin_direct_drag(
        &mut self,
        pointer_id: PointerId,
        position_x: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_handle.focus(window, cx);
        window.hide_soft_keyboard();
        self.gesture_state = GestureState::Dragging { pointer_id };
        self.last_drag_x = position_x;
        self.last_drag_dx = 0.0;
    }

    fn update_direct_drag(&mut self, position_x: f32, cx: &mut Context<Self>) {
        let raw_dx = position_x - self.last_drag_x;
        let eff_dx = match self.drawer_side {
            DrawerSide::Left => raw_dx,
            DrawerSide::Right => -raw_dx,
        };
        let width = f32::from(self.drawer_width);
        self.drawer_offset = (self.drawer_offset + eff_dx).clamp(0.0, width);
        self.last_drag_dx = eff_dx;
        self.last_drag_x = position_x;
        cx.notify();
    }

    fn end_direct_drag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.gesture_state = GestureState::Idle;
        let width = f32::from(self.drawer_width);
        let current_offset = self.drawer_offset;
        let last_dx = self.last_drag_dx;
        self.last_drag_dx = 0.0;
        let target = if last_dx < -2.0 {
            0.0
        } else if last_dx > 2.0 {
            width
        } else if current_offset > width / 2.0 {
            width
        } else {
            0.0
        };
        self.start_snap(target, Some(window), cx);
        if (current_offset - target).abs() >= 1.0 {
            cx.emit(if target == 0.0 {
                DrawerEvent::Closed
            } else {
                DrawerEvent::Opened
            });
        }
    }

    fn start_snap(&mut self, target: f32, window: Option<&mut Window>, cx: &mut Context<Self>) {
        if self.is_snap_animating() {
            return;
        }
        let current_offset = self.drawer_offset;
        let duration_ms = Self::snap_duration_ms(current_offset, target);

        if (current_offset - target).abs() < 1.0 {
            self.drawer_offset = target;
            self.drawer_state = if target > 0.0 {
                DrawerState::Opened
            } else {
                DrawerState::Closed
            };
            self.snap_target = None;
            self.snap_started_at = None;
            self._snap_task = None;
            cx.notify();
            return;
        }

        Self::hide_soft_keyboard(window);
        self.snap_from = current_offset;
        self.snap_target = Some(target);
        self.snap_started_at = Some(std::time::Instant::now());
        self.animation_id += 1;
        self.drawer_state = if target > 0.0 {
            DrawerState::Snapping
        } else {
            DrawerState::Closing
        };

        self._snap_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(duration_ms))
                .await;
            this.update(cx, |this, cx| {
                this.drawer_offset = target;
                this.drawer_state = if target > 0.0 {
                    DrawerState::Opened
                } else {
                    DrawerState::Closed
                };
                this.snap_target = None;
                this.snap_started_at = None;
                cx.notify();
            })
            .ok();
        }));

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
        let content = self.content.clone();
        let drawer = self.drawer.clone();
        let drawer_width = f32::from(self.drawer_width);
        let drawer_offset = self.drawer_offset;
        let max_opacity = self.backdrop_opacity;
        let is_dragging = matches!(self.gesture_state, GestureState::Dragging { .. });
        let snap_target = self.snap_target;
        let snap_from = self.snap_from;
        let animation_id = self.animation_id;
        let animating = self.is_snap_animating() && !is_dragging;
        let side = self.drawer_side;
        let edge_inset = self.edge_inset;

        let show_overlay = drawer_offset > 0.0 || snap_target.is_some() || is_dragging;

        // Gesture move/up/cancel must be attached to every occluding element.
        // .occlude() breaks GPUI's hit test chain before reaching the outer div,
        // so the backdrop and panel each carry their own copies of these handlers.
        macro_rules! gesture_handlers {
            ($el:expr) => {
                $el.on_pointer_move(cx.listener(|this, e: &PointerMoveEvent, window, cx| {
                    this.handle_pointer_move(e.pointer_id, e.position, window, cx);
                }))
                .on_pointer_up(cx.listener(|this, e: &PointerUpEvent, window, cx| {
                    this.handle_pointer_release(e.pointer_id, window, cx);
                }))
                .on_pointer_cancel(cx.listener(
                    |this, e: &PointerCancelEvent, window, cx| {
                        this.handle_pointer_release(e.pointer_id, window, cx);
                    },
                ))
            };
        }

        div()
            .id("drawer-host")
            .track_focus(&self.focus_handle)
            .size_full()
            .relative()
            .map(|el| gesture_handlers!(el)) // handles edge-zone drag before overlay renders
            .child(div().id("drawer-content").size_full().child(content))
            .when(show_overlay, |el| {
                let backdrop_base =
                    gesture_handlers!(div().absolute().inset_0().occlude().on_pointer_down(
                        cx.listener(|this, _, window, cx| {
                            if this.is_snap_animating() {
                                return;
                            }
                            cx.emit(DrawerEvent::BackdropTapped);
                            this.close_with_window(window, cx);
                        })
                    ));

                let panel_base = gesture_handlers!(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .w(px(drawer_width))
                        .bg(rgb(0x0e0c0c))
                        .flex()
                        .flex_col()
                        .overflow_hidden()
                        .occlude()
                        .id("drawer-panel")
                        .on_pointer_down(cx.listener(|this, event: &PointerDownEvent, _, _cx| {
                            if this.is_snap_animating() {
                                return;
                            }
                            if matches!(this.gesture_state, GestureState::Idle) {
                                this.gesture_state = GestureState::Pending {
                                    pointer_id: event.pointer_id,
                                    start: event.position,
                                    origin: DragOrigin::Panel,
                                };
                            }
                        }))
                )
                .child(drawer);

                let (backdrop, panel): (AnyElement, AnyElement) = if animating {
                    let from = snap_from;
                    let target = snap_target.unwrap();
                    let from_opacity = (from / drawer_width) * max_opacity;
                    let target_opacity = (target / drawer_width) * max_opacity;
                    let anim = self.snap_animation(from, target);
                    (
                        backdrop_base
                            .with_animation(
                                ElementId::NamedInteger(
                                    "drawer-backdrop-snap".into(),
                                    animation_id,
                                ),
                                anim.clone(),
                                move |el, delta| {
                                    el.bg(hsla(
                                        0.0,
                                        0.0,
                                        0.0,
                                        from_opacity + (target_opacity - from_opacity) * delta,
                                    ))
                                },
                            )
                            .into_any_element(),
                        panel_base
                            .with_animation(
                                ElementId::NamedInteger("drawer-panel-snap".into(), animation_id),
                                anim,
                                move |el, delta| {
                                    let o = from + (target - from) * delta;
                                    match side {
                                        DrawerSide::Left => el.left(px(o - drawer_width)),
                                        DrawerSide::Right => el.right(px(o - drawer_width)),
                                    }
                                },
                            )
                            .into_any_element(),
                    )
                } else {
                    let opacity = (drawer_offset / drawer_width).clamp(0.0, 1.0) * max_opacity;
                    (
                        backdrop_base
                            .bg(hsla(0.0, 0.0, 0.0, opacity))
                            .into_any_element(),
                        match side {
                            DrawerSide::Left => panel_base
                                .left(px(drawer_offset - drawer_width))
                                .into_any_element(),
                            DrawerSide::Right => panel_base
                                .right(px(drawer_offset - drawer_width))
                                .into_any_element(),
                        },
                    )
                };

                el.child(
                    deferred(
                        div()
                            .absolute()
                            .inset_0()
                            .child(backdrop)
                            .child(panel)
                            .when(animating, |el| {
                                el.child(
                                    div()
                                        .absolute()
                                        .inset_0()
                                        .occlude()
                                        .id("drawer-snap-shield"),
                                )
                            }),
                    )
                    .with_priority(998),
                )
            })
            .child(
                div()
                    .id("drawer-edge")
                    .absolute()
                    .top_0()
                    .h_full()
                    .w(px(edge_inset))
                    .when(side == DrawerSide::Left, |el| el.left_0())
                    .when(side == DrawerSide::Right, |el| el.right_0())
                    .on_pointer_down(cx.listener(|this, event: &PointerDownEvent, _, _cx| {
                        if this.is_snap_animating() {
                            return;
                        }
                        if matches!(this.gesture_state, GestureState::Idle) {
                            this.gesture_state = GestureState::Pending {
                                pointer_id: event.pointer_id,
                                start: event.position,
                                origin: DragOrigin::Edge,
                            };
                        }
                    })),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{DrawerHost, DrawerSide};
    use gpui::{AppContext as _, Empty, TestAppContext};

    #[test]
    fn direct_drag_claims_focus_and_hides_keyboard() {
        let mut cx = TestAppContext::single();
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |_, cx| {
                let content = cx.new(|_| Empty);
                let drawer = cx.new(|_| Empty);
                cx.new(|cx| DrawerHost::new(content.into(), drawer.into(), DrawerSide::Left, cx))
            })
            .unwrap()
        });

        window
            .update(&mut cx, |drawer_host, window, cx| {
                let previous_focus = cx.focus_handle();
                previous_focus.focus(window, cx);
                window.show_soft_keyboard();

                assert!(previous_focus.is_focused(window));
                assert!(window.is_soft_keyboard_visible());

                drawer_host.begin_direct_drag(1, 0.0, window, cx);

                assert!(drawer_host.focus_handle.is_focused(window));
                assert!(!previous_focus.is_focused(window));
                assert!(!window.is_soft_keyboard_visible());
            })
            .unwrap();
        cx.quit();
    }
}
