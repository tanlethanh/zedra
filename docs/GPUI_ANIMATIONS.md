# GPUI Animations

Practical reference for animation in Zedra.

Primary sources:
- `vendor/zed/crates/gpui/src/elements/animation.rs`
- `vendor/zed/crates/ui/src/traits/animation_ext.rs`

## Mental Model

GPUI animations are element wrappers, not a separate state system.

- Call `.with_animation(id, animation, closure)` on an element.
- Each frame GPUI computes `delta` in `[0.0, 1.0]`.
- Your closure returns a visually modified copy of the element.
- Animation state is keyed by the element `ElementId`.
- A fresh `ElementId` means a fresh animation state.

That implies two important rules:
- most animations are naturally enter animations
- restarting an animation means changing its ID

## Core API

```rust
element.with_animation(
    id,
    Animation::new(Duration::from_millis(200)),
    |el, delta| { ... },
)

element.with_animations(
    id,
    vec![anim1, anim2],
    |el, phase_ix, delta| { ... },
)
```

`Animation`:

```rust
Animation::new(duration)
    .repeat()              // loop forever
    .with_easing(easing)   // change easing
```

Useful helper methods from the `ui` crate:

```rust
icon.with_rotate_animation(2)
icon.with_keyed_rotate_animation("spinner", 2)
```

## Easing

Common choices:

- `linear`
- `quadratic`
- `ease_in_out`
- `ease_out_quint()`
- `bounce(ease_in_out)`
- `pulsating_between(0.3, 1.0)`

Important distinction:

- `linear`, `quadratic`, `ease_in_out` are passed directly
- `ease_out_quint()`, `bounce(...)`, `pulsating_between(...)` must be called to produce a closure

## The 4 Patterns That Matter

### 1. Enter animation

If the element appears conditionally, the animation starts from `delta = 0`.

```rust
if self.visible {
    div()
        .with_animation(
            "panel-enter",
            Animation::new(Duration::from_millis(200)).with_easing(ease_in_out),
            |el, delta| el.opacity(delta),
        )
}
```

### 2. Re-triggered or exit-like animation

GPUI does not provide a special exit animation primitive. Keep the element alive and animate it to a hidden state. To restart a oneshot animation, change the `ElementId`.

```rust
panel.with_animation(
    ElementId::NamedInteger("panel-slide".into(), self.anim_id),
    Animation::new(Duration::from_millis(250)).with_easing(ease_out_quint()),
    move |el, delta| el.left(px(from + (target - from) * delta)),
)
```

Rule:
- re-triggering = new ID

### 3. Looping animation

Use `.repeat()` for spinners, cursors, loading pulses, skeletons.

```rust
div().with_animation(
    "skeleton",
    Animation::new(Duration::from_secs(3))
        .repeat()
        .with_easing(pulsating_between(0.04, 0.24)),
    |el, delta| el.bg(gpui::black().opacity(delta)),
)
```

### 4. Sequenced animation

Use `with_animations` when phases are explicit and ordered.

```rust
label.with_animations(
    "loading-text",
    vec![
        Animation::new(Duration::from_secs(1)),
        Animation::new(Duration::from_secs(1)).repeat(),
    ],
    |label, phase_ix, delta| {
        match phase_ix {
            0 => label,
            1 => label,
            _ => label,
        }
    },
)
```

## Gesture-Driven Motion

GPUI does not have a React Native style shared animated value. Use normal component state as the source of truth.

Recommended pattern:

- `Idle`: settled value in state
- `Dragging`: gesture handlers mutate state directly and call `cx.notify()`
- `Snapping`: capture `from`, `target`, and a fresh animation ID, then render an animation between them

Rule:
- gesture updates mutate state
- animation closures do not mutate state

## Dragging Rules

These are the repo-specific details that matter most:

- `on_drag` is declarative. GPUI activates it automatically after more than `2 px` of pointer movement.
- `on_drag` belongs on each drag initiator.
- `on_drag_move::<T>` usually belongs on a common ancestor.
- `on_mouse_down` should seed any baseline values you need for incremental delta tracking.
- dropping a `Task<()>` cancels it; no special cancel API is required.

Typical structure:

```text
root container
  - on_drag_move::<DragType>
  - on_mouse_up
  initiator A
    - on_drag(DragType, ...)
    - on_mouse_down
  initiator B
    - on_drag(DragType, ...)
    - on_mouse_down
```

## Positioning Rule For `div`

For regular GPUI `div`s, use positional properties like `.left(px(...))`.

Do not plan around `translateX`/`transform` for `div` animation:
- `Transformation` / `Transformable` is effectively for SVG-based types
- regular `div` elements should be positioned with layout properties
- in GPUI this is acceptable because the scene is rebuilt every frame anyway

## Common Animator Closures

```rust
|el, d| el.opacity(d)                     // fade in
|el, d| el.opacity(1.0 - d)              // fade out
|el, d| el.left(px(from + (to - from) * d))
|el, d| el.h(px(max_h * d))
|el, d| el.bg(color.opacity(d))
|label, d| label.alpha(d)
```

## Repo Rules

1. IDs must be stable and unique among live animated elements.
2. New ID = restarted animation.
3. Keep `render()` pure. Animator closures return visuals only.
4. If an animated element is also a scroll container, assign its `.id(...)` before scroll helpers.
5. Exit animations require you to keep the element in the tree until the animation is finished.

## Recommended Defaults

Use these unless there is a clear reason not to:

- enter / fade / slide: `200-300ms`
- snap / panel movement: `250-300ms` with `ease_out_quint()`
- loading pulse: `2-3s`, repeated
- spinner: `1-2s`, repeated

## Minimal Examples

Fade in:

```rust
div().with_animation(
    "fade-in",
    Animation::new(Duration::from_millis(200)).with_easing(ease_in_out),
    |el, delta| el.opacity(delta),
)
```

Slide panel:

```rust
panel.with_animation(
    ElementId::NamedInteger("drawer-snap".into(), animation_id),
    Animation::new(Duration::from_millis(280)).with_easing(ease_out_quint()),
    move |el, delta| el.left(px(from_x + (to_x - from_x) * delta)),
)
```

Looping cursor:

```rust
div()
    .w(px(2.0))
    .h(px(18.0))
    .bg(cursor_color)
    .with_animation(
        cursor_id,
        Animation::new(Duration::from_millis(1000)).repeat(),
        |el, delta| el.opacity(if delta < 0.5 { 1.0 } else { 0.0 }),
    )
```
