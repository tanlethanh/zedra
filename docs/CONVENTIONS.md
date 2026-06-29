# Code Conventions

## Rust Style

Prioritize correctness and clarity. Treat speed and cleverness as secondary
unless performance is the stated problem.

- Prefer adding behavior to existing files unless the change introduces a real new logical component.
- Avoid creating `mod.rs` module paths. Prefer `src/name.rs`.
- Use full words for variable names. Avoid terse abbreviations like `q` for `queue`.
- Keep comments focused on non-obvious reasons, invariants, lifecycle constraints, or regression guards. Do not add comments that only summarize nearby code.
- Use variable shadowing to scope clones for async moves, so borrowed references do not live longer than necessary.

## Error Handling

Avoid panic-prone shortcuts in normal code paths.

- Prefer `?`, explicit `match`, or `if let Err(err)` over `unwrap()` and `expect()`.
- Be careful with indexing. Prefer checked access when out-of-bounds input is possible.
- Do not silently discard fallible results with `let _ = ...` when the error affects behavior or observability.
- When an error is intentionally ignored, make the reason visible with explicit handling such as `.log_err()` where available, a `tracing` warning, or a short comment for expected shutdown paths.
- Async operations that can fail should propagate errors to the layer that can show useful feedback to the user.

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

Commits made by Codex must include:

```text
Co-authored-by: Codex <codex@openai.com>
```

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

## Pull Requests

Use the same title convention as root commits:

```text
{type}: {short_message}
```

Keep the description to this shape:

```md
## Summary

- ...

## Notes

- ...
```

Use `## Summary` for what changed and why. Use `## Notes` for risks, follow-ups, screenshots, or manual context. Do not add a validation section; CI owns validation.

## Documentation Style

Write docs for developers who are scanning while trying to do a task.

- Start with the goal or action, not background.
- Put the common path first and edge cases later.
- Use short, direct sentences in present tense.
- Address the reader as `you` in user-facing docs.
- Avoid promotional phrasing, superlatives, filler words, and hedging such as "simply", "just", "easily", "powerful", or "seamless".
- State limitations directly and pair them with a workaround or next step when useful.
- Use `sh` fences for terminal command blocks and backticks for inline commands, paths, settings, and keybindings.
- Show complete working examples, not fragments.

## Repo Guidance Hygiene

Keep `AGENTS.md` and crate-level guidance files high-signal.

- Add a new rule only when it is non-obvious, repeatedly encountered, and specific enough to act on.
- Put crate-specific rules in that crate's `AGENTS.md`.
- Avoid architectural descriptions that go stale quickly. Prefer traps to avoid over maps to follow.
- Do not add drive-by rules from a one-off observation. Capture the suggested rule for review, then add it in a focused docs change once the pattern is validated.

## Platform Bridge

Always `platform_bridge::bridge()`. Never call platform APIs directly from UI code.

## UI Design Source

Read `docs/DESIGN.md` before creating or redesigning UI.

- Treat it as the visual source of truth for tone, density, spacing, typography, and component styling.
- New product UI should match the repo's flat, quiet, tool-like direction in both dark and light appearance.

## Theming

Read `docs/THEMING.md` before adding or changing product UI colors.

- **Workspace GPUI**: use `theme::palette(cx)` or `theme::bg_primary(cx)` (and related accessors) in `render()`. Layout sizes use constants in `theme.rs` (`SPACING_*`, `FONT_*`, `ICON_*`).
- **Editor**: sync `theme::bundle(cx).editor` into editor entities on create and on `ThemeStateEvent::Changed`.
- **Terminal**: sync `theme::bundle(cx).terminal` through `WorkspaceTerminal` / `TerminalView::set_terminal_theme`; do not map terminal ANSI colors from `ThemePalette`.
- **Do not** hardcode `0xRRGGBB` or inline light/dark branches in views. Add tokens in `crates/zedra/src/theme.rs` or `crates/zedra-terminal/src/theme.rs` when a new color is needed.
- Subscribe to `ThemeStateEvent::Changed` on the owning entity when the subtree is not refreshed by a parent that already notifies on theme change.

## Icons and Assets

`crates/zedra/assets/icons/<slug>.svg` is the single source of truth. The kebab-case slug is the icon's name on GPUI, iOS, and Android. Reference an icon by its slug string — it must match a filename in `assets/icons/`, since a wrong slug resolves to a missing icon at runtime. See AGENTS.md "Icon Assets" for the pipeline.

- **GPUI render**: `svg().path("icons/<slug>.svg")`. Source SVGs use `currentColor`, so set the tint with `.text_color(rgb(theme::text_primary(cx)))` (or another `theme::` token) and size with `.size(px(theme::ICON_MD))`; do not hardcode fill colors in the SVG or hex in the view.
- **Native bridge** (alerts, sheets, action rows): pass the bare slug with `.icon("google")` on `AlertButton` and friends. iOS resolves it via the asset-catalog imageset (named by slug); Android via the generated `ic_<slug>` drawable (the `ic_` prefix keeps the resource name identifier-safe — same rule at build and runtime).
- **SF Symbols** (names containing `.`, e.g. `doc.on.clipboard`) are iOS-only — use `.image("doc.on.clipboard")`.
- Add an icon by dropping a `currentColor` SVG at `assets/icons/<slug>.svg` and running `scripts/generate-assets.sh` (or `bun run icons:gen`). Never hand-edit the generated imagesets or drawables.

## Swift Native Integration

Keep Swift access control consistent across native bridge helper types and APIs.

- If a function returns a `fileprivate` type, mark the function `fileprivate`.
- If a stored property uses a `fileprivate` type, mark the property or enclosing type `fileprivate`.
- Do not mix `internal` APIs with `fileprivate` helper types by accident when adding iOS presentation helpers.

## Async Runtime Selection

The GPUI executor has no Tokio reactor. A future that drives the reactor on the calling thread panics (`there is no reactor running`) when polled inside `cx.spawn`. The runtime is owned by `gpui_tokio` (created in `app::init_platform_app` via `gpui_tokio::init`); `zedra-session` owns none and takes `Tokio::handle(cx)` at `Session::new`. With `use gpui_tokio::Tokio;`:

- **Host RPC calls** (`SessionHandle` methods, incl. `tokio::join!` over them) — `.await` directly in `cx.spawn`. They're irpc oneshots; no reactor touched on the calling thread.
- **Reactor-driving futures** (`tokio::time`, iroh I/O, Delta HTTP/`reqwest`) — wrap in `Tokio::spawn_result(cx, fut).await`. It folds `JoinError` into `anyhow`, so `match`/`?` is unchanged. Use `Tokio::spawn` for non-`Result` outputs (e.g. a `tokio::join!` tuple).
- **Fire-and-forget** from a `'static` callback (no `cx`) or work that must outlive backgrounding (GPUI executor pauses) — `Tokio::handle(cx).spawn(...)`, captured before the closure. Not `Tokio::spawn`, whose join half rides the paused GPUI executor.
- Pure GPUI awaits (`cx.background_executor().timer`, `cx.background_spawn`, channel `recv()`) — directly in `cx.spawn`.
- Inside `zedra-session`, bare `tokio::spawn` only when already on a runtime-polled future (e.g. the `connect()` loop).

## GPUI Entities And Tasks

- Use the `window, cx` parameter order when both are present.
- Put callback parameters after `cx` in function signatures that accept callbacks.
- Inside `Entity<T>::read_with`, `update`, or `update_in` closures, use the inner `cx` passed to the closure, not an outer context captured from the caller.
- Avoid updating an entity while it is already being updated. Reentrant entity updates panic.
- Prefer `WeakEntity<T>` for long-running async work or mutually-referential entity graphs so dropped entities do not stay alive accidentally.
- Use `cx.listener(...)` for element handlers that need to mutate the current entity.
- Use `cx.emit(...)` and `cx.subscribe(...)` for entity events, and store returned `Subscription`s on the subscribing entity.
- Call `cx.notify()` after state changes that affect rendering.
- `cx.spawn(...)` and `cx.background_spawn(...)` return a `Task`. Dropping the handle cancels the work, so await it, detach it, or store it according to the intended lifetime.
- Use `Task::ready(value)` when a task only needs to provide an already-available value.

## GPUI Tests

In GPUI tests that rely on `run_until_parked()`, use GPUI executor timers instead of `smol::Timer::after(...)`.

```rust
cx.background_executor().timer(duration).await;
```

This keeps timeout and delay work scheduled on GPUI's dispatcher, so the test harness can drive it.

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

## GPUI Flex Layout — Width Resolution in Column Containers

Do not use `w_full()` (`width: 100%`) on flex items inside a `flex_col()` container to make them fill the container's width. Taffy only resolves percentage widths against a definite size. When a flex container's width comes from cross-axis stretch (the default), it is not considered definite for percentage resolution, so `width: 100%` on children resolves against a higher ancestor and produces wrong widths.

**Use instead**: omit the explicit width and let the default `align-self: stretch` fill the cross axis.

```rust
// Wrong — w_full() resolves against the wrong ancestor
div().flex_col()
    .child(div().w_full().flex().flex_row()...)

// Correct — stretch is the default and uses the definite container width
div().flex_col()
    .child(div().min_w_0().flex().flex_row()...)
```

Keep `min_w_0()` on flex items that contain truncated text or overflow content to prevent them from overflowing their container.

For the column container itself to have a definite width (so its children can use stretch reliably), give it an explicit `w_full()` or absolute pixel width. Do not rely solely on cross-axis stretch being inherited transitively through multiple flex levels.

Do not combine `justify_between()` with `flex_1()` on a sibling to push a right-hand element to the far edge. With `flex_1()` consuming all free space, `justify-content: space-between` has no remaining space to distribute and behaves identically to `flex-start`. Use `flex_1()` on the left child alone — it naturally pushes the right child to the far edge without `justify_between`.

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
