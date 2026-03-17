# Zedra Code Conventions

## Rust Import Style

Prefer short module paths over fully-qualified inline `crate::` paths.

**Bad** — inline `crate::` in function bodies:
```rust
fn density(&self) -> f32 {
    crate::platform_bridge::bridge().density()
}
let inset = crate::platform_bridge::status_bar_inset();
```

**Good** — `use` import at the top, short name at the call site:
```rust
use crate::platform_bridge;

fn density(&self) -> f32 {
    platform_bridge::bridge().density()
}
let inset = platform_bridge::status_bar_inset();
```

For items used directly (not just the module), import the item:
```rust
use crate::editor::git_diff_view::{FileDiff, parse_unified_diff};
// then: FileDiff, parse_unified_diff(...)
```

For cfg-gated platform modules, use cfg-gated `use` items:
```rust
#[cfg(target_os = "android")]
use crate::android::analytics as android_analytics;
#[cfg(target_os = "ios")]
use crate::ios::analytics as ios_analytics;
```

## GPUI Render Purity

`render()` must be side-effect free. Mutations and async work belong in:
- Event handlers (`on_click`, `on_key_down`, etc.)
- `cx.spawn()` tasks
- Subscriptions (`cx.subscribe()`, `cx.observe()`)

**Allowed** in render: polling a `SharedPendingSlot` and applying the result to child
entities via `entity.update(cx, ...)` — this is the established async-to-UI bridge.

## Async-to-UI Bridge Pattern

Use `SharedPendingSlot<T>` to pass async results to the render cycle without
blocking:

```rust
// Struct field
pending_result: SharedPendingSlot<MyData>,

// Async task
let pending = self.pending_result.clone();
runtime.spawn(async move {
    let data = fetch().await;
    pending.set(data);
    zedra_session::push_callback(Box::new(|| {})); // wake render
});

// In render()
if let Some(data) = self.pending_result.take() {
    self.child_view.update(cx, |v, cx| v.apply(data, cx));
}
```

## Platform Bridge

Always access the bridge via `platform_bridge::bridge()`. Never call platform
APIs directly from UI code — go through the `PlatformBridge` trait so both
Android and iOS share the same call sites.

## Terminal Colors

Use `TerminalTheme::one_dark()` to get the default palette. Pass `&TerminalTheme`
through rendering functions rather than accessing global constants, so themes can
be swapped per-terminal in the future.

## Android Command Queue

The command queue is bounded (`crossbeam::bounded(512)`). Use `try_send()` and
drop-with-warn on full — never block the JNI thread. Touch events are high-volume
and must not OOM the process if the main thread is slow.

## JNI Safety

All `#[no_mangle] extern "C"` JNI entry points must wrap their body in
`jni_call("name", || { ... })` to catch Rust panics before they cross the
JNI boundary (undefined behavior).

## Alert Lifecycle

Alert callbacks hold closures. Call `platform_bridge::clear_pending_alerts()` on
app background (Android `onPause`, iOS `applicationDidEnterBackground`) to release
captured resources and prevent accumulation across sessions.

## File Structure

- `crates/zedra/src/platform_bridge.rs` — platform-agnostic bridge trait + global
- `crates/zedra/src/android/bridge.rs` — `AndroidBridge` impl, delegates to `jni`
- `crates/zedra/src/ios/bridge.rs` — `IosBridge` impl + iOS FFI exports
- `crates/zedra/src/theme.rs` — color constants, inset helpers
- `crates/zedra/src/keyboard.rs` — keyboard handler factories (no UI code)
- `crates/zedra/src/analytics.rs` — platform-dispatched analytics API
