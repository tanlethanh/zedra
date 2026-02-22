/// Touch event handler: tap detection, scroll/drawer disambiguation, fling momentum.
///
/// Extracted from `AndroidApp` to isolate the 6 touch-related fields and their
/// state machine logic into a focused, testable struct.

use gpui::*;
use gpui_android::AndroidPlatform;

use crate::mgpui::gesture::{GestureArena, GestureKind};

/// Active fling state for momentum scrolling
struct FlingState {
    velocity_x: f32,
    velocity_y: f32,
    last_time: std::time::Instant,
    /// Last touch position (logical pixels) for dispatching scroll events
    position: Point<Pixels>,
}

/// Tap slop threshold in logical pixels — movement beyond this means a drag, not a tap.
/// 4px logical = 12px physical at 3x density (close to Android's 8dp standard).
const TAP_SLOP: f32 = 4.0;

/// Minimum velocity (logical px/s) to start a fling.
const FLING_THRESHOLD: f32 = 50.0;

pub(crate) struct TouchHandler {
    last_touch_position: Option<(f32, f32)>,
    touch_down_position: Option<(f32, f32)>,
    touch_is_drag: bool,
    arena_winner_dispatched: bool,
    fling_state: Option<FlingState>,
    gesture_arena: GestureArena,
}

impl TouchHandler {
    pub fn new() -> Self {
        Self {
            last_touch_position: None,
            touch_down_position: None,
            touch_is_drag: false,
            arena_winner_dispatched: false,
            fling_state: None,
            gesture_arena: GestureArena::default_drawer_scroll(),
        }
    }

    /// Whether a fling animation is currently active.
    pub fn has_active_fling(&self) -> bool {
        self.fling_state.is_some()
    }

    /// Handle a touch event. Converts physical pixel coordinates to logical
    /// and dispatches GPUI input events via the platform.
    pub fn handle_touch(
        &mut self,
        action: i32,
        x: f32,
        y: f32,
        platform: &AndroidPlatform,
    ) -> anyhow::Result<()> {
        let scale = crate::platform_bridge::bridge().density();
        let logical_x = x / scale;
        let logical_y = y / scale;
        let position = point(px(logical_x), px(logical_y));

        log::trace!(
            "handle_touch: action={}, pos=({:.1}, {:.1})",
            action,
            logical_x,
            logical_y
        );

        match action {
            0 => {
                // ACTION_DOWN — cancel any active fling, record origin for tap detection
                self.fling_state = None;
                self.last_touch_position = Some((logical_x, logical_y));
                self.touch_down_position = Some((logical_x, logical_y));
                self.touch_is_drag = false;
                self.arena_winner_dispatched = false;
                self.gesture_arena.reset();
                crate::mgpui::reset_drawer_gesture();
            }
            1 => {
                // ACTION_UP — if the finger didn't move beyond TAP_SLOP, treat as tap
                if !self.touch_is_drag {
                    platform.dispatch_input(PlatformInput::MouseDown(MouseDownEvent {
                        button: MouseButton::Left,
                        position,
                        modifiers: Modifiers::default(),
                        click_count: 1,
                        first_mouse: false,
                    }));
                } else if matches!(self.gesture_arena.winner(), Some(GestureKind::Scroll)) {
                    platform.dispatch_input(PlatformInput::ScrollWheel(ScrollWheelEvent {
                        position,
                        delta: ScrollDelta::Pixels(point(px(0.0), px(0.0))),
                        modifiers: Modifiers::default(),
                        touch_phase: TouchPhase::Ended,
                    }));
                }
                // Always dispatch MouseUp so gesture-driven UI (drawer snap) works
                platform.dispatch_input(PlatformInput::MouseUp(MouseUpEvent {
                    button: MouseButton::Left,
                    position,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                }));
                self.last_touch_position = None;
                self.touch_down_position = None;
                self.touch_is_drag = false;
                self.arena_winner_dispatched = false;
                // Don't reset arena here — handle_fling() needs the winner.
            }
            3 => {
                // ACTION_CANCEL — clean up, no tap
                self.last_touch_position = None;
                self.touch_down_position = None;
                self.touch_is_drag = false;
                self.arena_winner_dispatched = false;
            }
            2 => {
                // ACTION_MOVE — check tap slop, then feed gesture arena
                if !self.touch_is_drag {
                    if let Some((down_x, down_y)) = self.touch_down_position {
                        let dx = logical_x - down_x;
                        let dy = logical_y - down_y;
                        if (dx * dx + dy * dy).sqrt() > TAP_SLOP {
                            self.touch_is_drag = true;
                        }
                    }
                }

                if self.touch_is_drag {
                    if let Some((last_x, last_y)) = self.last_touch_position {
                        let delta_x = logical_x - last_x;
                        let delta_y = logical_y - last_y;

                        self.gesture_arena.on_move(delta_x, delta_y);

                        match self.gesture_arena.winner() {
                            Some(GestureKind::DrawerPan) => {
                                let dx = if !self.arena_winner_dispatched {
                                    self.arena_winner_dispatched = true;
                                    let (accum_x, _) = self
                                        .gesture_arena
                                        .accumulated_delta(GestureKind::DrawerPan);
                                    accum_x
                                } else {
                                    delta_x
                                };
                                crate::mgpui::push_drawer_pan_delta(dx);
                                platform.request_frame_forced();
                            }
                            Some(GestureKind::Scroll) => {
                                let dy = if !self.arena_winner_dispatched {
                                    self.arena_winner_dispatched = true;
                                    let (_, accum_y) =
                                        self.gesture_arena.accumulated_delta(GestureKind::Scroll);
                                    accum_y
                                } else {
                                    delta_y
                                };
                                platform.dispatch_input(PlatformInput::ScrollWheel(
                                    ScrollWheelEvent {
                                        position,
                                        delta: ScrollDelta::Pixels(point(px(0.0), px(dy))),
                                        modifiers: Modifiers::default(),
                                        touch_phase: TouchPhase::Moved,
                                    },
                                ));
                            }
                            None => {
                                // Still undetermined — don't dispatch (buffer phase)
                            }
                        }
                    }
                }
                self.last_touch_position = Some((logical_x, logical_y));
            }
            _ => {}
        }

        Ok(())
    }

    /// Handle fling gesture — start momentum scrolling.
    pub fn handle_fling(&mut self, velocity_x: f32, velocity_y: f32) -> anyhow::Result<()> {
        let scale = crate::platform_bridge::bridge().density();
        let vx = velocity_x / scale;
        let vy = velocity_y / scale;

        // Filter fling velocity to the winning gesture's axis.
        let (fling_vx, fling_vy) = match self.gesture_arena.winner() {
            Some(GestureKind::DrawerPan) => return Ok(()),
            Some(GestureKind::Scroll) => (0.0, vy),
            None => (vx, vy),
        };

        let pos = self
            .last_touch_position
            .map(|(x, y)| point(px(x), px(y)))
            .unwrap_or_else(|| point(px(0.0), px(0.0)));

        if fling_vx.abs() > FLING_THRESHOLD || fling_vy.abs() > FLING_THRESHOLD {
            log::info!("[PERF] fling: start vel=({:.0}, {:.0})", fling_vx, fling_vy);
            self.fling_state = Some(FlingState {
                velocity_x: fling_vx,
                velocity_y: fling_vy,
                last_time: std::time::Instant::now(),
                position: pos,
            });
        }
        Ok(())
    }

    /// Process active fling — apply friction and dispatch scroll events.
    pub fn process_fling(&mut self, platform: &AndroidPlatform) {
        let fling = match &mut self.fling_state {
            Some(f) => f,
            None => return,
        };

        let now = std::time::Instant::now();
        let dt = now.duration_since(fling.last_time).as_secs_f32();
        fling.last_time = now;

        // Frame-rate independent friction
        let friction = 0.95_f32.powf(dt * 60.0);
        fling.velocity_x *= friction;
        fling.velocity_y *= friction;

        let vx = fling.velocity_x;
        let vy = fling.velocity_y;

        if vx.abs() < FLING_THRESHOLD && vy.abs() < FLING_THRESHOLD {
            log::info!("[PERF] fling: end");
            platform.dispatch_input(PlatformInput::ScrollWheel(ScrollWheelEvent {
                position: fling.position,
                delta: ScrollDelta::Pixels(point(px(0.0), px(0.0))),
                modifiers: Modifiers::default(),
                touch_phase: TouchPhase::Ended,
            }));
            self.fling_state = None;
            return;
        }

        let delta_x = vx * dt;
        let delta_y = vy * dt;

        platform.dispatch_input(PlatformInput::ScrollWheel(ScrollWheelEvent {
            position: fling.position,
            delta: ScrollDelta::Pixels(point(px(delta_x), px(delta_y))),
            modifiers: Modifiers::default(),
            touch_phase: TouchPhase::Moved,
        }));
    }
}
