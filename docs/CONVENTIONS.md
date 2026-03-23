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

## Logging

Use `tracing::` macros everywhere across all crates. Never use `log::` directly —
`tracing` is a superset that provides structured fields, span context, and callsite
metadata (file, line, module). It bridges to the `log` backend automatically via
`features = ["log"]` in the workspace `Cargo.toml`.

**Level guidelines:**

| Level | When to use |
|---|---|
| `error` | Unrecoverable failure; something is definitely broken |
| `warn` | Expected failure path; operation skipped, degraded but continuing |
| `info` | Meaningful lifecycle events: connect, disconnect, surface, terminal create/close, auth |
| `debug` | JNI bookkeeping, keyboard events, routine platform calls, per-operation detail |
| `trace` | Not used |

**Message format:** `"<component>: <verb> <noun> [key=value ...]"`

- Lowercase first letter, no trailing period
- Use structured fields for key=value data, not string interpolation
- Errors: `{}` (Display), not `{:?}` (Debug), unless the type has no Display impl
- One log site per event — don't log entry and exit for short operations

```rust
// Good
tracing::info!(endpoint = %addr.id.fmt_short(), gen = my_gen, "session: connecting");
tracing::warn!(id = %terminal_id, err = %e, "terminal: attach failed");
tracing::debug!("jni: init complete");

// Bad — log::, prose format, entry+exit pair, Debug for error
log::info!("SessionHandle: connecting (endpoint: {}, gen: {})", addr, gen);
log::info!("gpuiInit called");
log::info!("gpuiInit completed successfully, handle: {}", ptr);
log::error!("Failed to open window: {:?}", e);
```

**Component prefixes** (lowercase, consistent within a file):

| Subsystem | Prefix |
|---|---|
| Session connect / state machine | `session:` |
| PKI auth phases | `auth:` |
| Terminal pump / attach | `terminal:` |
| Reconnect loop | `reconnect:` |
| Surface lifecycle | `surface:` |
| Workspace state / persistence | `store:` |
| JNI bridge (Android) | `jni:` |
| iOS lifecycle / FFI | `ios:` |
| Android lifecycle | `android:` |

See `docs/LOGGING_MIGRATION.md` for the current migration status and file-by-file
checklist.

## WorkspaceState as Single Source of Truth

All UI display components **must** read workspace display data from `WorkspaceState`,
never directly from `SessionHandle` or any other source.

**Rule:** If a field needed for display doesn't exist on `WorkspaceState`/`WorkspaceStateInner`,
add it there. Do not use `SessionHandle` methods as a fallback or shortcut.

**Why:** `SessionHandle` fields (project_name, hostname, workdir, etc.) are empty during
the connecting phase — they are only populated after a successful RPC round-trip.
`WorkspaceState` is seeded from persisted data before the connection starts, so it always
has something useful to show. Once connected, `ZedraApp::render()` copies live handle
fields back into the state, keeping everything in sync.

**Data flow:**
```
Persisted JSON → WorkspaceState (seeded at connect time)
                        ↓
           ZedraApp::render() updates entry.state from handle (non-empty fields only)
                        ↓
           ZedraApp::render() pushes state → WorkspaceView → WorkspaceContent + WorkspaceDrawer
                        ↓
           All display reads self.workspace_state.*
```

**Do:**
```rust
// In any display component render
let name = self.workspace_state.project_name();
let wd   = self.workspace_state.workdir();
```

**Don't:**
```rust
// Never do this in display code
let name = self.session_handle.project_name();     // empty during connecting
let name = if handle_name.is_empty() { fallback }  // no fallbacks
```

**Adding a new field:** Add it to `WorkspaceStateInner` in `workspace_state.rs`, add a
getter on `WorkspaceState`, and populate it in `ZedraApp::render()`'s state-update loop.
If the field comes from the server post-connect, copy it with the "non-empty only" guard
(same pattern as `project_name`, `hostname`, etc.).

## File Structure

- `crates/zedra/src/platform_bridge.rs` — platform-agnostic bridge trait + global
- `crates/zedra/src/android/bridge.rs` — `AndroidBridge` impl, delegates to `jni`
- `crates/zedra/src/ios/bridge.rs` — `IosBridge` impl + iOS FFI exports
- `crates/zedra/src/theme.rs` — color constants, inset helpers
- `crates/zedra/src/keyboard.rs` — keyboard handler factories (no UI code)
- `crates/zedra-telemetry/src/lib.rs` — typed Event enum, TelemetryBackend trait, runtime injection
- `crates/zedra/src/analytics.rs` — FirebaseBackend: registers with zedra-telemetry at app startup
