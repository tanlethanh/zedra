# Touch Event System

## Overview

Zedra runs GPUI on mobile platforms (Android and iOS) that have no mouse. Touch input must be translated into GPUI's event model, which uses `PlatformInput` variants:

- `MouseDown` / `MouseUp` — tap (finger lift without drag)
- `ScrollWheelEvent` with `TouchPhase` — drag and fling
- `KeyDown` / `KeyUp` — hardware and IME keyboard

There are no dedicated touch event types in GPUI's `PlatformInput` enum. Touch scroll is modeled as `ScrollWheelEvent` with `touch_phase: TouchPhase::Moved/Ended`.

## Platform Layers

### Android: `gpui_android` (`vendor/zed/crates/gpui_android/src/android/platform.rs`)

`AndroidPlatform` owns all touch semantics. The JNI bridge (`crates/zedra/src/android/jni.rs`) receives raw Android `MotionEvent` integers and forwards them via the command queue to the main thread, which calls:

```
platform.handle_touch(action, x_physical, y_physical)
platform.handle_fling(velocity_x_physical, velocity_y_physical)
platform.process_fling()   // called every frame from handle_frame_request
```

**Physical → logical conversion**: `logical = physical / display_scale`. Android delivers coordinates in physical pixels; GPUI works in logical pixels.

**Tap detection**: `TAP_SLOP = 4.0` logical pixels. If the finger lifts within this radius of the down position, a `MouseDown + MouseUp` pair is dispatched. Otherwise the touch was a drag.

**Drag → ScrollWheel**: On `ACTION_MOVE` past TAP_SLOP, each frame's `(delta_x, delta_y)` in logical pixels is dispatched as `ScrollWheelEvent` with `touch_phase: TouchPhase::Moved`. On `ACTION_UP` after a drag, a zero-delta event with `TouchPhase::Ended` is dispatched, followed by `MouseUp`.

**Fling gating on tap**: `GpuiSurfaceView.java` always forwards velocity to `nativeFlingEvent` when it exceeds 150 px/s. Tap vs drag classification is authoritative in Rust: if `ACTION_UP` is processed as a tap (`is_drag = false`), `handle_touch` clears any stored fling before dispatching `MouseDown + MouseUp`. This prevents spurious scrolling after a fast tap gesture.

**Fling**: When Android's `VelocityTracker` fires (via `nativeFlingEvent`), `handle_fling()` stores the velocity if it exceeds `FLING_THRESHOLD = 50.0` logical px/s. Each frame, `process_fling()` applies frame-rate-independent friction (`0.95^(dt×60)`) and dispatches scroll events until velocity decays below threshold.

### iOS: `gpui_ios` (`vendor/zed/crates/gpui_ios/src/ios/window.rs`)

`IosWindow.handle_touch()` receives `UITouch` objects from UIKit, already in logical points (no pixel conversion needed — UIKit coordinates are points, not physical pixels).

**Tap detection**: UIKit tracks tap count (`numberOfTapsRequired`). A single `UITouchPhase::Began` dispatches `MouseDown`; `UITouchPhase::Ended` dispatches `MouseUp`. This matches GPUI's tap model.

**Drag → ScrollWheel**: `UITouchPhase::Moved` dispatches `ScrollWheelEvent` via `pan_gesture_to_scroll()`. The delta is `current_position - previous_position`, provided by UIKit.

**Velocity tracking**: During `Moved` events, instantaneous velocity `v = delta / dt` is smoothed with an exponential moving average (`0.7 × v_old + 0.3 × v_instant`). On `Ended`, if the smoothed velocity exceeds `FLING_THRESHOLD`, a `TouchFling` state is stored.

**Fling**: `process_fling()` is called by `gpui_ios_request_frame` (the `CADisplayLink` callback) before invoking the render callback. This ensures fling scroll events reach GPUI in the same frame they are generated.

## Gesture Disambiguation: `DrawerHost`

`DrawerHost` (`crates/zedra/src/mgpui/drawer_host.rs`) receives all scroll events via its `on_scroll_wheel` GPUI handler. It discriminates:

- **Horizontal dominant** (`dx.abs() > dy.abs()`): drawer pan
  - Left-edge zone (44px): only opens drawer when closed
  - Auto-snaps at 30% width threshold or 6px/frame velocity
- **Vertical dominant**: passes through to content scroll (`uniform_list`)

Both platforms send touch drags as `ScrollWheelEvent`, so `DrawerHost.on_scroll_wheel` handles gesture disambiguation uniformly — no platform-specific routing. GPUI's scroll handler does not call `stop_propagation()`, so the drawer and content scrollers both receive every event and each applies its own filter.

## Why No Gesture Arena

Earlier versions used a `GestureArena` that buffered touch events until it could classify the gesture as drawer-pan or content-scroll. This was removed because:

1. `DrawerHost.on_scroll_wheel` can discriminate in-place using `dx.abs() > dy.abs()`
2. The same handler works identically on iOS (which never had the arena)
3. No "buffer leak" in practice: when `dx` dominates, the `dy` component is small enough that the content scroller doesn't visibly move; and vice versa

## Mouse and Stylus Routing (Future)

Android's `MotionEvent.getToolType()` distinguishes `TOOL_TYPE_FINGER`, `TOOL_TYPE_MOUSE`, and `TOOL_TYPE_STYLUS`. Currently all tool types go through the same drag-to-scroll path. Future work:

- **`TOOL_TYPE_MOUSE`** with `ACTION_SCROLL`: route directly to `ScrollWheelEvent` without drag conversion or fling (mouse wheel is discrete)
- **`TOOL_TYPE_MOUSE`** with `ACTION_MOVE` while button held: route to `MouseMove` events (mouse drag)
- **`TOOL_TYPE_FINGER`**: current behavior (drag → scroll + fling)

iOS UIKit maps hardware trackpad/mouse to `UITouchTypeIndirectPointer`. The current `handle_touch` implementation treats indirect pointer touches the same as finger touches; a future improvement could skip fling for `UITouchTypeIndirectPointer`.

## Fling Physics

Both platforms use the same friction model:

```
friction = 0.95 ^ (dt × 60)          // frame-rate independent
v_new = v_old × friction
delta = v_new × dt                    // logical pixels this frame
```

- `dt`: time in seconds since last fling frame (from `Instant::now()`)
- Threshold: `50.0` logical px/s — below this, fling ends with a `TouchPhase::Ended` event
- Typical decay: ~800ms to stop from 500 px/s starting velocity

## Bug Investigations

### TAP_SLOP mismatch — Fixed

Previously `GpuiSurfaceView.java` maintained a duplicate `TAP_SLOP = 12f` (physical px) that gated `nativeFlingEvent` dispatch. On devices with non-integer density (e.g. Mali-G68 at 2.75×), Rust's `TAP_SLOP = 4.0` logical px equates to 11 physical px — below Java's 12px threshold. This caused a gap where Rust classified the gesture as a drag (scrolled) but Java suppressed fling.

**Fix**: Java now always forwards velocity to `nativeFlingEvent`. Rust's `handle_touch(ACTION_UP)` is authoritative: it clears any stored fling before dispatching tap events (`MouseDown + MouseUp`).

### Fling position (ScrollWheelEvent.position) — Not a bug

The `position` field in fling-generated `ScrollWheelEvent`s is captured at fling start (the touch lift-off point) and does not change during the fling. This is correct: GPUI uses `position` only for hit-testing (which element handles the event), and the scrollable container's bounds in window coordinates do not move as the content scrolls. The lift-off position reliably identifies the target element for all fling frames.

### Enter key ignored in `Input` — Fixed

`Input` (`crates/zedra/src/mgpui/input.rs`) silently swallowed `Return`/`Enter` key presses. Now emits `InputSubmit { value }` so callers can subscribe and act on form submission.

## File Map

| File | Responsibility |
|------|---------------|
| `vendor/zed/crates/gpui_android/src/android/platform.rs` | `handle_touch`, `handle_fling`, `process_fling`, `has_active_fling` |
| `vendor/zed/crates/gpui_ios/src/ios/window.rs` | `handle_touch` (velocity tracking, fling start), `process_fling` |
| `vendor/zed/crates/gpui_ios/src/ios/ffi.rs` | `gpui_ios_request_frame` — calls `process_fling` before render |
| `crates/zedra/src/android/jni.rs` | JNI bridge: `nativeTouchEvent`, `nativeFlingEvent` |
| `crates/zedra/src/android/app.rs` | Delegates touch/fling commands to platform |
| `crates/zedra/src/mgpui/drawer_host.rs` | `on_scroll_wheel` handler: drawer ↔ content discrimination |
| `android/app/src/main/java/dev/zedra/app/GpuiSurfaceView.java` | Forwards raw touch events; velocity tracking via `VelocityTracker` |
