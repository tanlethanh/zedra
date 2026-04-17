# Code Conventions

## Imports

Use glob imports for common framework crates:

```rust
use gpui::*;
use tracing::*;
use zedra_telemetry::*;
```

Prefer short module paths over inline `crate::` paths:

```rust
use crate::platform_bridge;

let inset = platform_bridge::status_bar_inset();
```

For items used directly, import the item:
```rust
use crate::editor::git_diff_view::{FileDiff, parse_unified_diff};
```

## Logging

Use `tracing::` everywhere. Never `log::` directly.

```rust
use tracing::*;

info!(endpoint = %addr.id.fmt_short(), "session: connecting");
warn!(id = %terminal_id, err = %e, "terminal: attach failed");
```

**Levels**: `error` = broken, `warn` = degraded, `info` = lifecycle events, `debug` = bookkeeping. No `trace`.

**Format**: `"component: verb noun"`, lowercase, no trailing period. Use structured fields for key=value, `{}` (Display) for errors.

## Platform Bridge

Always `platform_bridge::bridge()`. Never call platform APIs directly from UI code.

## Async Runtime Selection

Choose the executor based on which thread/context owns the work:

- `cx.spawn(...)` or `cx.spawn_in(window, ...)` for UI-thread async work in GPUI.
- `zedra_session::session_runtime().spawn(...)` for session/network tasks that must run on Tokio even when called from GPUI or other non-Tokio threads.
- `tokio::spawn(...)` only when the current function is already guaranteed to run inside the session Tokio runtime and the task is not part of a reusable API that may also be called from GPUI.

**Rule of thumb**: library/session-layer code should not assume the caller has entered a Tokio runtime. If it needs to spawn Tokio tasks internally, prefer `session_runtime()` over bare `tokio::spawn()`.

## WorkspaceState as Single Source of Truth

All display reads from `WorkspaceState`, never `SessionHandle`.

**Why**: `SessionHandle` fields are empty during connecting. `WorkspaceState` is seeded from persisted data before connection starts.

**Data flow**:
```
Persisted JSON → WorkspaceState (Entity, seeded at connect time)
Session emits ConnectEvent → SessionState.apply_event() → WorkspaceState.sync_from_session()
Views read Entity<WorkspaceState> in render()
```

**Adding a new display field**: add to `WorkspaceState` struct + populate in `sync_from_session()`.

## Android-Specific

- **Command queue**: bounded (`crossbeam::bounded(512)`). Use `try_send()`, drop-with-warn on full. Never block JNI thread.
- **JNI safety**: all `#[no_mangle] extern "C"` JNI entry points must wrap body in `jni_call("name", || { ... })`.

## Alert Lifecycle

Call `platform_bridge::clear_pending_alerts()` on app background to release captured closures.
