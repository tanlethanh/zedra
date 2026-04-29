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

## Git Commits

Use a conventional subject line with one of the repo-approved types:

```text
feat|fix|chore|docs: <description>
```

When the change is scoped to a specific platform, feature, or crate, include a short lowercase scope:

```text
feat(ios): <description>
fix(host): <description>
chore(rpc): <description>
```

Keep the description concise and describe the user-visible or maintainer-visible change.

Keep each commit scoped to the current feature or fix. This repo often has multiple active edits in the same worktree, so do not stage or commit unrelated files or hunks.

`vendor/zed` is a separate git submodule and follows the convention documented in `vendor/zed/.rules`:

- Clear, capitalized, imperative subject with no trailing punctuation.
- No conventional prefixes such as `fix:`, `feat:`, or `docs:`.
- Optional crate scope when one crate is the clear scope, such as `git_ui: Add history view`.
- Upstream squash commits append the PR number, such as `Fix crash in project panel (#12345)`.
- The parent Zedra commit that updates the submodule pointer still follows the root repo convention.

Before committing:

- Inspect `git diff --cached --stat`, `git diff --cached --name-only`, and the staged hunks.
- If a file contains both related and unrelated edits, stage only the related hunks or apply an exact patch to the index.
- Run `git diff --cached --check` before the commit.

## Platform Bridge

Always `platform_bridge::bridge()`. Never call platform APIs directly from UI code.

## UI Design Source

Read `docs/DESIGN.md` before creating or redesigning UI.

- Treat it as the visual source of truth for tone, density, spacing, typography, and component styling.
- New product UI should match the repo's dark, flat, quiet, tool-like direction.

## Swift Native Integration

Keep Swift access control consistent across native bridge helper types and APIs.

- If a function returns a `fileprivate` type, mark the function `fileprivate`.
- If a stored property uses a `fileprivate` type, mark the property or enclosing type `fileprivate`.
- Do not mix `internal` APIs with `fileprivate` helper types by accident when adding iOS presentation helpers.

## Async Runtime Selection

Choose the executor based on which thread/context owns the work:

- `cx.spawn(...)` or `cx.spawn_in(window, ...)` for UI-thread async work in GPUI.
- `zedra_session::session_runtime().spawn(...)` for session/network tasks that must run on Tokio even when called from GPUI or other non-Tokio threads.
- `tokio::spawn(...)` only when the current function is already guaranteed to run inside the session Tokio runtime and the task is not part of a reusable API that may also be called from GPUI.

**Rule of thumb**: library/session-layer code should not assume the caller has entered a Tokio runtime. If it needs to spawn Tokio tasks internally, prefer `session_runtime()` over bare `tokio::spawn()`.

## GPUI Rendering

For redraw, invalidation, `deferred(...)`, and `AnyView::cached(...)` behavior, see `docs/GPUI_RENDERING_MODEL.md`.

## GPUI Mobile Interaction

Zedra product UI is mobile-only today. Do not add hover-dependent behavior or visual states to app surfaces.

- Avoid `.hover(...)`, `visible_on_hover`, `hoverable_tooltip`, and hover-only reveals in `crates/zedra`.
- Use pressed, selected, active, disabled, text, icon, border, and hit-slop states for touch UI instead.
- Keep `.cursor_pointer()` only as a pointer cursor hint; it is not a substitute for a touch-readable state.
- Vendor GPUI and desktop reference code may keep hover APIs. App code should use them only when a pointer-capable platform is intentionally supported.

## GPUI Scroll Containers

`overflow_scroll()` and `overflow_y_scroll()` require the `Div` to have a stable `.id(...)`.

Use:

```rust
div()
    .id("my-scroll-area")
    .overflow_y_scroll()
```

Do not apply GPUI scroll overflow helpers to anonymous `Div`s.

When the scroll area lives inside nested flex layouts, the parent chain must also provide a constrained height.

- Use `size_full()` on the viewport wrapper that is expected to fill the sheet/window body.
- Add `min_h_0()` to each intermediate flex child between the constrained viewport and the GPUI scroll node.
- This is required for embedded native iOS sheet content, where GPUI otherwise tends to measure the scroll node at content height and `overflow_y_scroll()` will not produce a usable scroll range.

See `docs/GPUI_NATIVE_PRESENTATIONS.md` for the native sheet gesture bridge and ownership split.

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

## Native Presentation Callback Lifecycle

Call `platform_bridge::clear_pending_alerts()` on app background to release
captured closures for alerts, selection sheets, and native notifications.
