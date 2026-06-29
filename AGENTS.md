# Zedra

Mobile remote editor for iOS and Android. Primary platform is iOS (`gpui_ios` + Metal). Secondary platform is Android (`gpui_android` + `gpui_wgpu` + Vulkan).

## Agent Workflow

- Inspect the relevant code paths first and infer local patterns before proposing or making changes.
- When a request mentions the Zedra CLI or `zedra` command-line behavior, inspect `crates/zedra-host` first; that crate owns the daemon and CLI entrypoints.
- Ask before making any meaningful product or architectural decision. Tiny details may follow existing patterns without approval.
- For normal feature work, prefer the smallest diff that fits the current design.
- If the current structure is blocking quality, propose the refactor and wait for approval before doing broader cleanup.
- Keep code concise, readable, and modular. Prefer clarifying code over adding comments.
- When fixing an edge case or an important regression-prone path, add a minimal code comment at the relevant block explaining the invariant or reason for the guard.
- Prioritize correctness and clarity over cleverness or speed unless performance is the explicit problem.
- Avoid panic-prone shortcuts such as unchecked indexing or `unwrap()` in normal code paths. Propagate or handle errors instead of discarding fallible results.
- Surface blockers quickly with a recommendation. Keep progress updates short and include reasoning or tradeoffs.

## Debugging Workflow

- Read the relevant code path deeply before changing behavior.
- On mobile issues, prefer adding targeted `tracing` logs with a clear searchable prefix so the developer can run the app, reproduce, and return logs.
- After the first failed debugging attempt, stop and ask for more information instead of arguing from hypotheses.
- Prefer root-cause fixes once the issue is confirmed.

## Repo Invariants

- `WorkspaceState` is the single source of truth for display state. Views read `WorkspaceState`, never `SessionHandle`, during render.
- `render()` must stay pure. Side effects belong in event handlers, subscriptions, or async tasks.
- Use `platform_bridge::bridge()` for platform integration. Do not call platform APIs directly from UI code.
- Use existing imports and concise module-qualified calls for platform UI affordances, such as `platform_bridge::trigger_haptic(HapticFeedback::ImpactLight)`. Normal taps and workspace switches should use light haptics; reserve stronger feedback for long press, confirmation, or destructive actions.
- Use `tracing` for logging. Never add `log::` calls.
- Read `docs/DESIGN.md` before creating or redesigning UI. Read `docs/THEMING.md` before adding colors; new GPUI UI must use `theme::` tokens, not hardcoded hex in views.
- GPUI tasks are cancelled when their `Task` handle is dropped. Await, detach, or store tasks according to the intended lifetime.
- Inside GPUI entity `update`, `read_with`, and related closures, use the inner `cx` passed to the closure and avoid reentrant updates of the same entity.
- Keep lifecycle helpers aligned with their names. Entry points that own the user action should own policy checks such as dedupe, reconnect, or stale cleanup; lower-level helpers that create/connect/initialize should not also switch entries, disconnect existing state, or hide caller contracts.
- For workspace reconnect and duplicate-entry handling, treat `Connected`, `Idle`, and in-flight connecting phases as active entries to switch to. Only `Failed` or `Disconnected` entries should be treated as stale reconnect candidates.

## Protocol And Telemetry

- `docs/PROTOCOL_SPECS.md` is canonical. Any protocol change must update `zedra-rpc/src/proto.rs`, the relevant host and client handlers, and `docs/PROTOCOL_SPECS.md` in the same change.
- `crates/zedra-telemetry/src/lib.rs` defines the canonical telemetry `Event` enum.
- Telemetry must not include personal data. Use opaque IDs, durations, counts, enum labels, and booleans only.

## Zedra Delta Backend

- Zedra Delta is Zedra's cloud/backend service — push notifications, Live Activities, workspace sync signals, and any other backend capability the host or mobile app needs. See `docs/DELTA_INTEGRATION.md`.
- The Delta source lives in a separate repo at the path in the `ZEDRA_DELTA_REPO` environment variable. When a task needs Delta backend behavior (read its code, or add/change a backend feature), open `$ZEDRA_DELTA_REPO` directly — read and edit it like part of the codebase.
- Whenever this repo's work mentions the Delta backend or Zedra Delta, treat `$ZEDRA_DELTA_REPO` as the place those backend changes belong, and keep the host/mobile protocol contract in sync across both repos.
- On the app side, Delta represents auth and backend interaction only (sign-in, push-token registration, backend calls) — not app features that merely arrive over Delta. 

## Documentation

- Write docs in a practical, direct style: start with the goal, put common actions first, and avoid promotional or apologetic wording.
- Use complete working examples. Use `sh` fences for terminal command blocks and backticks for inline commands, paths, settings, and keybindings.
- Keep repo guidance high-signal. New rules should be non-obvious, repeatedly encountered, and specific enough to act on; crate-specific rules belong in that crate's `AGENTS.md`.

## GitHub Issues

- When asked to create an issue, first check `.github/ISSUE_TEMPLATE/` and use the matching template if one exists.
- Preserve the template's headings, prompts, and required fields. Only omit optional sections that clearly do not apply.
- If the repo has no issue template, use `## Summary`, `## Reproduction`, `## Expected Behavior`, `## Actual Behavior`, and `## Notes` as the fallback structure.
- Keep issues concrete and concise: state the problem, affected platform/scope, repro steps, expected vs actual behavior, and relevant logs, screenshots, or context when available.
- Search existing open issues before creating a new one to avoid duplicates.

## Validation

- Prefer targeted checks over broad suites.
- Add or update tests when there is an obvious existing place for them.
- For UI, platform, and device-driven changes, add or update manual verification steps in `docs/MANUAL_TEST.md`.
- Common checks:
  - `cargo fmt`
  - `cargo check -p zedra-rpc -p zedra-session -p zedra-terminal -p zedra-host`
  - `cargo check --manifest-path vendor/zed/Cargo.toml -p gpui_ios -p gpui` for vendored GPUI/iOS framework patches
  - `bun run format`
  - `bun run check`

## Git Commits

- Use commit subjects in the form `feat|fix|chore|docs: <description>`.
- When the change is scoped to a platform, feature, or crate, use `type(scope): <description>`, such as `feat(ios): ...`, `fix(host): ...`, or `chore(rpc): ...`.
- Pull request titles use the same root format, `{type}: {short_message}`. Descriptions use only `## Summary` and `## Notes`; omit validation because CI owns it.
- Commits must include only changes related to the current feature or fix. Never stage or commit unrelated work, even when the worktree has multiple concurrent edits.
- Commits made by Codex must include `Co-authored-by: Codex <codex@openai.com>`.
- `vendor/zed` is a separate submodule with its own git style: clear, capitalized, imperative subjects, no conventional prefixes, optional crate scope such as `git_ui: Add history view`, and no trailing punctuation.

## Platform Scope

- iOS is the primary development path. See `docs/IOS_WORKFLOW.md` for build, install, launch, and logging commands.
- To run the Android app on a connected device, use `./scripts/run-android.sh`. Do not use `adb shell am start` directly.
- Native iOS presentations should keep UIKit responsible for alerts, sheets, and keyboard accessories.
- `UIGlassEffect` is public UIKit on iOS 26+. Use `if #available(iOS 26.0, *)`, not runtime probing.
- In Swift bridge code, keep access control consistent with helper type visibility.

## Icon Assets

- `crates/zedra/assets/icons/<slug>.svg` is the single source of truth; the kebab-case slug is the name on every platform (GPUI, iOS, Android). Commit only the SVGs, and reference an icon by that bare slug string (no enum) — a typo resolves to a missing icon at runtime, so keep it exact.
- Generated assets are gitignored — never hand-edit: iOS imagesets (`ios/Zedra/Assets.xcassets/*.imageset`) from `scripts/generate-assets.sh`, Android drawables (`android/app/src/generated/res/drawable/*.xml`) from the Gradle `generateIconDrawables` task. Builds run both automatically (Xcode pre-build script in `ios/project.yml`; Android `preBuild` depends on `generateIconDrawables`).
- Add an icon by dropping a `currentColor` SVG at the path above, then run `scripts/generate-assets.sh` (or `bun run icons:gen`) for iOS/GPUI preview and/or build Android. No network fetch — source the SVG yourself.
- GPUI render: `svg().path("icons/<slug>.svg")`, tint with `.text_color(...)` (SVGs are `currentColor`). Native bridge (alerts, sheets, action rows): pass the bare slug to `.image("google")` → iOS imageset / Android `ic_<slug>` drawable. SF Symbols (dotted names like `doc.on.clipboard`) also go through `.image("…")` and are iOS-only.
- Android conversion uses Android Studio's `Svg2Vector` (`com.android.tools:sdk-common`, AGP-pinned in `android/build.gradle`): it resolves inherited `fill`/`stroke` and `circle`/`line`/`polyline` shapes that lighter converters silently drop, and bakes `currentColor` to a concrete color (drawables tinted at runtime). CI smoke-tests the iOS script; Android conversion runs in Gradle.
- Future remote/dynamic icons (host-delivered, not bundled) should use a runtime image loader (Coil + coil-svg on Android), scoped to that path — not these bundled monochrome glyphs.

## Vendor Zed

- `vendor/zed` is a git submodule and an intentional part of the architecture, not just third-party reference code.
- We patch `vendor/zed` and related GPUI/mobile crates directly when Zedra needs mobile support that upstream GPUI or Zed does not provide yet.
- When changing behavior that touches GPUI, iOS/Android platform crates, rendering, input, or text handling, inspect `vendor/zed` first and treat it as part of the codebase.
- Run vendored package checks through `vendor/zed/Cargo.toml`. The root workspace excludes `vendor`, and `cargo check -p gpui_ios` from the root can hit a Cargo feature resolver panic because `gpui_ios` is only a target-specific path dependency there.
- For editor features, use Zed desktop code as a reference for concepts and architecture, but implement minimal mobile-specific versions in Zedra rather than trying to port desktop behavior wholesale.

## Docs Map

- `docs/CONVENTIONS.md` — imports, Rust style, error handling, GPUI lifecycle, logging, git commit subjects, docs style, async runtime choice, `WorkspaceState`, platform bridge, scroll container rules
- `docs/ARCHITECTURE.md` — crate boundaries, session flow, auth, RPC, transport
- `docs/DESIGN.md` — product UI tone and component direction
- `docs/THEMING.md` — theme tokens, light/dark, editor/terminal palettes, subscription pattern
- `docs/IOS_WORKFLOW.md` — iOS build pipeline, FFI workflow, pitfalls
- `docs/PROTOCOL_SPECS.md` — protocol contract
- `docs/TELEMETRY.md` — telemetry events and privacy
- `docs/MANUAL_TEST.md` — manual verification steps for UI and device work
- `docs/DEVTOOL.md` — in-app HTTP devtool for agent-driven UI scripting (debug Android); wrapper at `scripts/devtool.sh`
- `vendor/zed/` — GPUI, platform crates, grammars, and desktop reference implementations

## Recent Learnings

- GPUI scroll containers need an explicit `.id(...)`. In nested flex layouts, also constrain the full parent chain with `size_full()` and `min_h_0()` or scrolling can silently fail.
- In GPUI `flex_col()` children, avoid `w_full()` for width fill; let stretch work with `min_w_0()`. For right-aligned row actions, make the left text column `flex_1()` and the action `flex_none()`.
- GPUI text wrapping requires definite width constraints. Use `.w(px(width))` not `.max_w(px(width))` for text containers or text wraps at viewport width.
- In `cx.spawn`, host RPC calls (`SessionHandle` methods, incl. `tokio::join!`) `.await` directly — irpc oneshots, no reactor on the GPUI thread. Only reactor-driving futures crash there (`tokio::time`, iroh I/O, Delta HTTP); wrap those in `Tokio::spawn_result(cx, fut).await` (`use gpui_tokio::Tokio;`), or `Tokio::handle(cx).spawn(...)` for fire-and-forget / background-surviving tasks. See `docs/CONVENTIONS.md` § Async Runtime Selection.
- Session-to-UI flow is `Session / SessionHandle -> ConnectEvent -> SessionState -> WorkspaceState -> Views`. Preserve that layering.
- New `extern "C"` Rust→Swift call: add a weak stub in `crates/zedra/src/ios_stub.c` matching the function signature or the iOS staticlib link will fail with undefined symbol.
