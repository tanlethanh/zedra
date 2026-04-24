# GPUI Mobile Interaction

RFC for GPUI's mobile interaction model.

This document recommends a new public interaction layer for GPUI that is suitable
for mobile touch input, multi-input devices, and gesture-heavy UI. It is written
against the current Zedra codebase and GPUI vendored crates.

Status: proposed

---

## 1. Problem Statement

GPUI's current public interaction model is desktop-first:

- low-level input is mostly `MouseDown`, `MouseMove`, `MouseUp`, `ScrollWheel`
- drag/drop is modeled as mouse-driven state in the view tree
- many handlers are hover-gated
- touch support is mostly implemented by mapping platform touch input into
  mouse-like or scroll-like events

That model is good enough for desktop UI and trackpads. It is not a sound
foundation for mobile touch interaction.

The failure mode is visible in `crates/zedra/src/ui/drawer_host.rs`:

- drawer swipe depends on `.on_drag(...)`, `.on_drag_move(...)`, `.on_drop(...)`
- release depends on `on_drop` or `on_mouse_up`
- GPUI `on_drop` is effectively a typed `MouseUp` handler while a drag is active
- on iOS, moved touches may intentionally suppress `MouseUp`
- on mobile, hover-based release routing is not a reliable abstraction

Result: components like `DrawerHost` are forced to patch around touch release,
gesture ownership, and nested scroll conflicts using mouse semantics that were
not designed for those problems.

This RFC recommends a new interaction model with two public layers:

1. Public pointer events for low-level control
2. Public gesture recognizers for the default app-facing API

The recommendation is intentionally additive. Existing mouse, wheel, and
desktop drag/drop APIs remain supported.

---

## 2. Current State In This Repo

### 2.1 Core GPUI model

Current GPUI interaction types live primarily in:

- `vendor/zed/crates/gpui/src/interactive.rs`
- `vendor/zed/crates/gpui/src/elements/div.rs`
- `vendor/zed/crates/gpui/src/window.rs`

Today GPUI exposes:

- `MouseDownEvent`
- `MouseMoveEvent`
- `MouseUpEvent`
- `ScrollWheelEvent`
- `PinchEvent`
- typed drag/drop built on `cx.active_drag`

Current drag/drop assumptions:

- drag activation is tied to left-button mouse movement over a threshold
- `on_drag_move` fires from mouse move while `cx.active_drag` exists
- `on_drop` is driven by `MouseUp`
- `on_mouse_up` and `on_drop` often depend on `hitbox.is_hovered(window)`

GPUI already has one touch-oriented concept:

- `ScrollWheelEvent.touch_phase`

That field is useful for scroll bridging, but it is not a complete pointer or
gesture model.

### 2.2 Web backend

Current web input lives in:

- `vendor/zed/crates/gpui_web/src/events.rs`
- `vendor/zed/crates/gpui_web/src/window.rs`

Important facts:

- the web backend listens to DOM `pointerdown`, `pointerup`, `pointermove`,
  `pointerleave`, and wheel events
- it lowers DOM pointer input directly into GPUI mouse events
- it does not preserve pointer identity (`pointerId`)
- it does not preserve pointer type (`mouse`, `touch`, `pen`)
- it does not use DOM pointer capture
- canvas root currently sets `touch-action: none`

This means GPUI already starts from a modern pointer-capable platform on web,
but discards the information needed for a proper cross-device model.

### 2.3 iOS backend

Current iOS input lives in:

- `vendor/zed/crates/gpui_ios/src/ios/events.rs`
- `vendor/zed/crates/gpui_ios/src/ios/window.rs`

Important facts:

- `UITouch` begin maps to `MouseDown`
- move emits both `MouseMove` and `ScrollWheel`
- moved touches may suppress `MouseUp` if travel exceeds tap slop
- current implementation tracks one primary touch for several behaviors
- mobile release semantics are therefore not equivalent to desktop mouse-up

This is the exact mismatch that breaks mobile drag-release logic in
`DrawerHost`.

### 2.4 Android backend

Current Android input lives in:

- `vendor/zed/crates/gpui_android/src/android/platform.rs`
- `crates/zedra/src/android/*.rs`

Important facts:

- backend distinguishes tap vs drag using touch slop
- drags are converted to `ScrollWheelEvent`
- fling is modeled separately
- tap path dispatches `MouseDown` and `MouseUp` at release time
- touch is therefore already interpreted as a gesture category, not as a raw
  mouse-equivalent stream

iOS and Android already diverge, which is a signal that GPUI needs a first-class
model instead of more adapter-specific patching.

### 2.5 Existing Zedra precedents

Zedra already contains touch-specific logic that points toward a better model:

- `crates/zedra-terminal/src/view.rs` uses `ScrollWheelEvent.touch_phase`
- `crates/zedra/src/editor/code_editor.rs` contains horizontal gesture-locking
  logic on scroll deltas
- `docs/GPUI_NATIVE_PRESENTATIONS.md` explicitly accepts native ownership for
  sheet gestures and uses scroll injection as the interop boundary

The repo has already moved away from "touch is just mouse" in several places.
The public GPUI API should catch up.

---

## 3. Research Summary

This section captures the specific design lessons that should shape GPUI's new
interaction layer.

### 3.1 Web Pointer Events

Primary reference:

- [W3C Pointer Events Level 3](https://www.w3.org/TR/pointerevents3/)

Key takeaways:

- pointer input is modeled as a first-class stream, not as mouse emulation
- every pointer has identity (`pointerId`)
- events preserve device type (`pointerType`)
- cancellation is first-class (`pointercancel`)
- pointer capture is first-class
- gesture negotiation is declarative through `touch-action`
- compatibility mouse events are adapters, not the primary abstraction

Relevant design implications for GPUI:

- GPUI should preserve pointer identity and type
- GPUI should have explicit cancel semantics
- GPUI should support pointer capture per pointer, not a single global capture
- GPUI should expose a declarative gesture negotiation API analogous to
  `touch-action`

### 3.2 React Native Responder System

Primary references:

- [Gesture Responder System](https://reactnative.dev/docs/gesture-responder-system)
- [PanResponder](https://reactnative.dev/docs/panresponder)

Key takeaways:

- touch handling is negotiated, not assumed
- a view can claim touch ownership at start or after movement begins
- the owner receives move, release, and termination
- termination can come from another component or from the OS
- gesture state tracks accumulated distance, velocity, and active touches

Relevant design implications for GPUI:

- gesture ownership must support both release and termination/cancel
- nested interaction requires explicit negotiation rules
- pan-like gestures should expose accumulated translation and velocity
- GPUI should not force app code to rebuild these patterns from raw mouse events

### 3.3 React Native Pointer Events

Primary reference:

- [Pointer Events in React Native](https://reactnative.dev/blog/2022/12/13/pointer-events-in-react-native)

Key takeaways:

- React Native chose W3C Pointer Events as the cross-platform baseline
- native `MotionEvent` and `UITouch` are mapped into pointer events
- the implementation is "best effort with well documented deviations"
- pointer APIs and gesture/responder APIs are related but not identical
- React Native explicitly identifies pointer capture and `touch-action` as
  important future work

Relevant design implications for GPUI:

- GPUI should also adopt pointer semantics as the common low-level contract
- GPUI can document platform deviations where perfect web equivalence is not
  possible
- recognizer APIs should sit above pointer events, not replace them

### 3.4 React Native Gesture Handler

Primary references:

- [Introduction](https://docs.swmansion.com/react-native-gesture-handler/docs/)
- [Handler State](https://docs.swmansion.com/react-native-gesture-handler/docs/under-the-hood/state/)
- [Pan gesture](https://docs.swmansion.com/react-native-gesture-handler/docs/gestures/use-pan-gesture/)
- [Gesture composition](https://docs.swmansion.com/react-native-gesture-handler/docs/fundamentals/gesture-composition/)

Key takeaways:

- gestures are recognizers with explicit states
- recognizers expose callbacks at begin, activate, update, finalize
- activation thresholds are configurable
- relations between gestures are first-class:
  - competing/race
  - simultaneous
  - exclusive / priority-based
  - require-to-fail / block relations across components
- native gesture systems are the right foundation on mobile when gesture
  arbitration matters

Relevant design implications for GPUI:

- GPUI's public gesture API should be recognizer-oriented
- gestures should have explicit state machines
- gesture composition should be built into the API, not left to ad hoc app code

### 3.5 UIKit

Primary references:

- [UIGestureRecognizer](https://developer.apple.com/documentation/uikit/uigesturerecognizer)
- [Allowing the simultaneous recognition of multiple gestures](https://developer.apple.com/documentation/uikit/allowing-the-simultaneous-recognition-of-multiple-gestures)
- [UIScreenEdgePanGestureRecognizer](https://developer.apple.com/documentation/uikit/uiscreenedgepangesturerecognizer)
- [UIPanGestureRecognizer velocity(in:)](https://developer.apple.com/documentation/uikit/uipangesturerecognizer/velocity%28in%3A%29?language=objc)

Key takeaways:

- gesture recognizers are state machines
- simultaneous recognition is explicit and opt-in
- gesture recognizers may cancel or delay underlying touch delivery
- edge-pan is a first-class gesture type
- translation and velocity are core pan outputs

Relevant design implications for GPUI:

- mobile gesture APIs should feel recognizer-like
- cancellation is not exceptional; it is normal platform behavior
- edge gestures deserve explicit support in configuration, even if not a v1
  standalone recognizer type

### 3.6 Android Views

Primary references:

- [Manage touch events in a ViewGroup](https://developer.android.com/training/gestures/viewgroup)
- [MotionEvent](https://developer.android.com/reference/android/view/MotionEvent)
- [VelocityTracker](https://developer.android.com/reference/android/view/VelocityTracker)
- [ViewConfiguration](https://developer.android.com/reference/android/view/ViewConfiguration)

Key takeaways:

- parent views may intercept touch streams after movement begins
- when interception changes ownership, child gets `ACTION_CANCEL`
- pointer ids and multi-pointer transitions are first-class
- touch slop and fling thresholds are platform primitives
- velocity is a normal part of gesture recognition

Relevant design implications for GPUI:

- cancellation is a mandatory part of the model
- nested ownership changes must be representable
- pointer identity must survive backend translation

---

## 4. Design Goals

GPUI's new interaction model should:

- work for touch, pen, mouse, and trackpad-backed pointer devices
- preserve raw input information that desktop-mouse APIs discard
- make mobile gesture handling first-class
- provide explicit cancel semantics
- support nested gesture negotiation and ownership transfer
- support native-first recognition on mobile
- remain compatible with current desktop interactions
- be testable without platform-specific app code

It should not:

- replace desktop drag/drop semantics in v1
- require every GPUI app to manage raw pointers directly
- force native sheets or system-owned gestures under GPUI ownership
- attempt a large breaking rewrite of all existing mouse APIs in one step

---

## 5. Recommended Public Model

### 5.1 Two public layers

GPUI should expose two public layers:

1. **Pointer API**
   - low-level
   - cross-platform
   - public
   - suitable for advanced components, drawing, custom input, and expert code

2. **Gesture API**
   - recognizer-oriented
   - public
   - the default ergonomic API for mobile-style interaction

Gesture recognizers are built on the pointer layer conceptually, but on mobile
their recognition should use native gesture machinery when appropriate.

### 5.2 Native-first mobile rule

On iOS and Android:

- GPUI should preserve raw platform input as public pointer events
- GPUI gesture recognizers should be backed by native gesture recognition when a
  native recognizer exists and gesture ownership/arbitration matters
- GPUI should use recognizer adapters so app code sees one GPUI API
- GPUI should not manually reconstruct standard mobile gestures in Rust on
  those platforms when native recognizers already provide correct activation,
  cancellation, velocity, and simultaneous-recognition behavior

On web and desktop:

- GPUI should use pointer-stream-backed recognizer adapters
- recognizers should expose the same public state model and event payload shape

This is the central recommendation of this RFC:

> **Public pointer events everywhere. Public gesture recognizers everywhere.
> Native-first recognition on mobile. Pointer-stream adapters elsewhere.**

---

## 6. Proposed Public Pointer API

### 6.1 Types

```rust
pub struct PointerId(pub u64);

pub enum PointerKind {
    Mouse,
    Touch,
    Pen,
}

pub enum PointerPhase {
    Down,
    Move,
    Up,
    Cancel,
    Enter,
    Leave,
    HoverMove,
}

pub struct PointerButtons {
    pub primary: bool,
    pub secondary: bool,
    pub middle: bool,
    pub back: bool,
    pub forward: bool,
}

pub struct PointerEvent {
    pub id: PointerId,
    pub kind: PointerKind,
    pub phase: PointerPhase,
    pub position: Point<Pixels>,
    pub delta: Point<Pixels>,
    pub buttons: PointerButtons,
    pub is_primary: bool,
    pub pressure: Option<f32>,
    pub modifiers: Modifiers,
    pub timestamp: std::time::Duration,
}
```

### 6.2 Element API

```rust
div()
    .on_pointer_down(...)
    .on_pointer_move(...)
    .on_pointer_up(...)
    .on_pointer_cancel(...)
    .on_pointer_enter(...)
    .on_pointer_leave(...)
```

Capture-phase variants should exist for the same reasons GPUI already supports
capture-phase mouse handling:

```rust
div()
    .capture_pointer_down(...)
    .capture_pointer_move(...)
    .capture_pointer_up(...)
    .capture_pointer_cancel(...)
```

### 6.3 Pointer capture

Current GPUI pointer capture is effectively single-hitbox global capture.
That is not sufficient for real multi-pointer interaction.

Recommended API:

```rust
impl Window {
    pub fn capture_pointer(&mut self, pointer_id: PointerId, hitbox_id: HitboxId);
    pub fn release_pointer(&mut self, pointer_id: PointerId);
    pub fn has_pointer_capture(&self, pointer_id: PointerId, hitbox_id: HitboxId) -> bool;
}
```

Rules:

- capture is scoped per pointer
- capture survives movement outside bounds
- capture is implicitly released on `Up` and `Cancel`
- capture release must be observable by recognizers and raw handlers

### 6.4 Declarative negotiation

GPUI should expose a declarative property analogous to web `touch-action`.

```rust
pub enum TouchAction {
    Auto,
    None,
    PanX,
    PanY,
    PanXY,
    Manipulation,
}
```

Element API:

```rust
div().touch_action(TouchAction::PanX)
```

Meaning:

- `Auto`: platform/default ownership rules
- `None`: GPUI wants raw pointer delivery and custom gesture handling
- `PanX`: horizontal pan may be claimed by GPUI content; vertical pan remains
  available to ancestor/native scroll unless recognizer relations say otherwise
- `PanY`: inverse of `PanX`
- `PanXY`: GPUI content wants both axes
- `Manipulation`: optimized platform handling for common pan/zoom/tap-style
  manipulations where custom raw pointer control is not required

This is the primary public negotiation primitive for nested scroll and custom
gesture conflicts.

---

## 7. Proposed Public Gesture API

### 7.1 Recognizer model

Recognizers are the primary public interaction API for gesture-rich UI.

On mobile backends, the recognizer API is a GPUI abstraction over native
recognizers first, not a request for app code to hand-build pan/tap/press logic
from raw pointer deltas.

Every gesture recognizer should share a common state model:

```rust
pub enum GestureState {
    Possible,
    Began,
    Active,
    Ended,
    Failed,
    Cancelled,
}
```

V1 recognizers:

- `TapGesture`
- `LongPressGesture`
- `PanGesture`
- `PinchGesture`

Planned follow-on recognizers:

- `RotationGesture`
- `FlingGesture`
- `HoverGesture`
- `EdgePanGesture`

### 7.2 Core recognizer builders

Representative shape:

```rust
let pan = PanGesture::new()
    .min_distance(px(8.0))
    .active_offset_x((-Pixels::MAX, px(8.0)))
    .fail_offset_y((-px(12.0), px(12.0)))
    .on_begin(...)
    .on_update(...)
    .on_end(...)
    .on_cancel(...);

div().gesture(pan)
```

Representative event payloads:

```rust
pub struct TapGestureEvent {
    pub state: GestureState,
    pub position: Point<Pixels>,
    pub pointer_kind: PointerKind,
    pub timestamp: std::time::Duration,
}

pub struct LongPressGestureEvent {
    pub state: GestureState,
    pub position: Point<Pixels>,
    pub duration: std::time::Duration,
}

pub struct PanGestureEvent {
    pub state: GestureState,
    pub position: Point<Pixels>,
    pub translation: Point<Pixels>,
    pub delta: Point<Pixels>,
    pub velocity: Point<Pixels>,
    pub number_active_touches: usize,
}

pub struct PinchGestureEvent {
    pub state: GestureState,
    pub center: Point<Pixels>,
    pub scale: f32,
    pub scale_delta: f32,
    pub velocity: f32,
}
```

### 7.3 Element sugar

Recognizer objects are the core API, but GPUI should also expose sugar for the
common cases:

```rust
div()
    .on_tap(...)
    .on_long_press(...)
    .on_pan(...)
    .on_pinch(...)
```

Rules:

- sugar is built on recognizers
- sugar uses default thresholds and default relations
- advanced configuration uses explicit recognizer objects

### 7.4 Composition and relations

Gesture composition must be first-class.

Same-element composition:

```rust
Gesture::race([g1, g2])
Gesture::simultaneous([g1, g2, g3])
Gesture::exclusive([high_priority, low_priority])
```

Cross-element relations:

```rust
g1.simultaneous_with(&g2)
g1.require_to_fail(&g2)
g1.blocks(&g2)
```

Semantics:

- `race`: first recognizer to activate cancels others
- `simultaneous`: all may remain active together
- `exclusive`: later recognizers wait for earlier recognizers to fail
- `require_to_fail`: defer activation until dependency fails
- `blocks`: reverse relation useful for scroll containers waiting on child
  gestures

This should be the default solution for nested drawer-vs-scroll and
double-tap-vs-single-tap conflicts.

---

## 8. Platform Mapping

### 8.1 iOS

Raw pointer layer:

- preserve `UITouch` identity
- expose `Down`, `Move`, `Up`, `Cancel`
- preserve multiple simultaneous touches
- preserve pressure when available
- synthesize `PointerKind::Touch` and `PointerKind::Pen` where platform can
  distinguish

Gesture layer:

- use native recognizers first for tap, long press, pan, pinch, rotation, and
  edge-pan style interactions
- map `UIGestureRecognizer` state to GPUI `GestureState`
- honor simultaneous/failure rules through native recognizer relationships
- do not route GPUI mobile gestures through desktop `on_drag` / `on_drop`

Interop rule:

- GPUI does not try to steal system gestures that UIKit owns
- container-native interactions such as sheet detents remain native
- GPUI uses `touch_action`, relation rules, and explicit bridges for ownership
  boundaries

### 8.2 Android

Raw pointer layer:

- preserve `MotionEvent` pointer ids
- preserve `ACTION_POINTER_DOWN`, `ACTION_POINTER_UP`, `ACTION_CANCEL`
- preserve multi-touch state
- preserve timestamps and pressure where appropriate

Gesture layer:

- use native-first recognition semantics
- touch slop, minimum fling velocity, and maximum fling velocity should come
  from platform configuration
- ownership changes should map to `Cancelled` for the losing recognizer

### 8.3 Web

Raw pointer layer:

- preserve DOM pointer fields instead of lowering immediately to mouse
- use DOM pointer capture
- handle `pointercancel`
- stop hard-coding root `touch-action: none`; negotiate at the relevant element

Gesture layer:

- implement GPUI recognizers over pointer streams
- match public GPUI recognizer states and payloads
- document any browser-specific deviations where exact mobile parity is not
  possible

### 8.4 Desktop

Desktop continues to use:

- mouse-backed pointer events
- trackpad wheel and pinch
- desktop drag/drop

Desktop does not need native gesture recognizers for v1, but it should still see
the same public pointer and gesture APIs.

---

## 9. Compatibility Rules

This RFC is additive.

Existing APIs remain supported:

- `MouseDownEvent`
- `MouseMoveEvent`
- `MouseUpEvent`
- `ScrollWheelEvent`
- `PinchEvent`
- `on_drag`
- `on_drag_move`
- `on_drop`

Recommended status:

- keep them as compatibility APIs
- continue using them for desktop patterns and external file drag/drop
- do not use them as the default foundation for new mobile gesture components

Important v1 rule:

> New mobile interaction work in GPUI should target pointer + gesture APIs, not
> mouse drag/drop.

`ScrollWheelEvent.touch_phase` remains useful for:

- native sheet scroll bridging
- compatibility with existing mobile scroll code
- trackpad-style scrolling interop

It should **not** remain the primary future touch interaction API.

---

## 10. DrawerHost As Pilot Migration

`DrawerHost` should be the first migration target after the new APIs exist.

Current behavior problems:

- release depends on mouse-up/drop
- drag activation is tied to mouse drag semantics
- nested scroll and backdrop conflicts are hard to reason about
- iOS and Android backends do not produce equivalent mouse semantics

Recommended migration:

1. express drawer interaction as a `PanGesture`
2. configure horizontal activation thresholds explicitly
3. use relation rules against inner scroll views
4. snap on `Ended`
5. reset on `Cancelled`
6. use velocity from the pan recognizer for snap bias
7. keep desktop drag/drop behavior unchanged

Representative API shape:

```rust
let drawer_pan = PanGesture::new()
    .touch_action(TouchAction::PanX)
    .active_offset_x((-Pixels::MAX, px(8.0)))
    .fail_offset_y((-px(12.0), px(12.0)))
    .on_begin(...)
    .on_update(...)
    .on_end(...)
    .on_cancel(...);

div().gesture(drawer_pan)
```

This is the validation target for the entire RFC. If the new model is awkward
for `DrawerHost`, the model is not finished.

---

## 11. Testing And Acceptance Criteria

### 11.1 Pointer tests

- single pointer down -> move -> up
- single pointer down -> move -> cancel
- multiple simultaneous pointers with stable ids
- pointer capture per pointer
- capture release on up and cancel
- hover-capable devices remain correct on desktop

### 11.2 Gesture tests

- tap vs long press separation
- pan threshold activation
- pinch begin/update/end
- exclusive single tap vs double tap
- simultaneous pan + pinch
- require-to-fail for nested tap handlers
- cancel path from parent/native interception

### 11.3 Platform tests

- web pointer capture and `pointercancel`
- iOS recognizer cancellation and simultaneous recognition
- Android interception -> cancel semantics
- slop/velocity thresholds match platform units

### 11.4 Pilot integration tests

- `DrawerHost` edge swipe open/close
- nested drawer vs scroll interaction
- gesture cancel when system/native container takes ownership
- existing desktop drag/drop remains unchanged

Acceptance criteria for v1:

- mobile drag-like UI no longer relies on `on_drop`
- pointer ids and cancel semantics are public and testable
- gesture ownership is explicit
- drawer and scroll conflicts are solved using gesture relations, not ad hoc
  backend-specific patches

---

## 12. Migration Plan

### Phase 1 - Public pointer substrate

- add public pointer types and element handlers
- preserve pointer identity and cancel semantics in all backends
- implement per-pointer capture
- add `touch_action`

### Phase 2 - Public gesture recognizers

- add recognizer object model
- add v1 recognizers: tap, long press, pan, pinch
- add composition and relation APIs

### Phase 3 - Mobile native-first backends

- iOS gesture adapters backed by native recognizers
- Android gesture adapters backed by native-first recognition
- web recognizer adapters backed by pointer streams

### Phase 4 - Component migration

- migrate `DrawerHost`
- migrate the next highest-value mobile gesture components
- keep legacy mouse APIs as compatibility layer

---

## 13. Non-Goals

This RFC does not propose:

- replacing external desktop file drag/drop
- deleting mouse APIs in v1
- a broad bidirectional native gesture abstraction for sheet-specific content
- solving every platform-specific system gesture in the first release
- immediate generalization of all current scroll-bridge code

Those may follow later. They are not prerequisites for the new public model.

---

## 14. Recommendation

Adopt the following as GPUI policy:

- **Public pointer events are first-class**
- **Public gestures are recognizer-first**
- **Mobile gesture recognition is native-first**
- **Web and desktop use recognizer-compatible adapters**
- **Existing mouse and drag/drop APIs stay additive**

This is the smallest model that is:

- correct for mobile
- compatible with the web pointer ecosystem
- familiar to React Native and native mobile engineers
- implementable without a breaking rewrite of existing GPUI apps

---

## 15. Internal References

Relevant current code and docs in this repo:

- `vendor/zed/crates/gpui/src/interactive.rs`
- `vendor/zed/crates/gpui/src/elements/div.rs`
- `vendor/zed/crates/gpui/src/window.rs`
- `vendor/zed/crates/gpui_web/src/events.rs`
- `vendor/zed/crates/gpui_web/src/window.rs`
- `vendor/zed/crates/gpui_ios/src/ios/events.rs`
- `vendor/zed/crates/gpui_ios/src/ios/window.rs`
- `vendor/zed/crates/gpui_android/src/android/platform.rs`
- `crates/zedra/src/ui/drawer_host.rs`
- `crates/zedra-terminal/src/view.rs`
- `crates/zedra/src/editor/code_editor.rs`
- `docs/GPUI_NATIVE_PRESENTATIONS.md`
- `docs/GPUI_FOCUS_INPUT_KEYBOARD.md`

---

## 16. External References

- W3C Pointer Events Level 3:
  https://www.w3.org/TR/pointerevents3/
- React Native Gesture Responder System:
  https://reactnative.dev/docs/gesture-responder-system
- React Native PanResponder:
  https://reactnative.dev/docs/panresponder
- React Native Pointer Events blog:
  https://reactnative.dev/blog/2022/12/13/pointer-events-in-react-native
- React Native Gesture Handler docs:
  https://docs.swmansion.com/react-native-gesture-handler/docs/
- Apple UIKit gesture recognizers:
  https://developer.apple.com/documentation/uikit/uigesturerecognizer
- Apple simultaneous recognition:
  https://developer.apple.com/documentation/uikit/allowing-the-simultaneous-recognition-of-multiple-gestures
- Apple screen edge pan:
  https://developer.apple.com/documentation/uikit/uiscreenedgepangesturerecognizer
- Android touch event interception:
  https://developer.android.com/training/gestures/viewgroup
- Android `MotionEvent`:
  https://developer.android.com/reference/android/view/MotionEvent
- Android `VelocityTracker`:
  https://developer.android.com/reference/android/view/VelocityTracker
- Android `ViewConfiguration`:
  https://developer.android.com/reference/android/view/ViewConfiguration
