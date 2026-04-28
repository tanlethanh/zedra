# Zedra

Mobile remote editor for iOS and Android. Primary platform is iOS (`gpui_ios` + Metal). Secondary platform is Android (`gpui_android` + `gpui_wgpu` + Vulkan).

## Agent Workflow

- Inspect the relevant code paths first and infer local patterns before proposing or making changes.
- Ask before making any meaningful product or architectural decision. Tiny details may follow existing patterns without approval.
- For normal feature work, prefer the smallest diff that fits the current design.
- If the current structure is blocking quality, propose the refactor and wait for approval before doing broader cleanup.
- Keep code concise, readable, and modular. Prefer clarifying code over adding comments.
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
- Use `tracing` for logging. Never add `log::` calls.
- Read `docs/DESIGN.md` before creating or redesigning UI.

## Protocol And Telemetry

- `docs/PROTOCOL_SPECS.md` is canonical. Any protocol change must update `zedra-rpc/src/proto.rs`, the relevant host and client handlers, and `docs/PROTOCOL_SPECS.md` in the same change.
- `crates/zedra-telemetry/src/lib.rs` defines the canonical telemetry `Event` enum.
- Telemetry must not include personal data. Use opaque IDs, durations, counts, enum labels, and booleans only.

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
- Commits must include only changes related to the current feature or fix. Never stage or commit unrelated work, even when the worktree has multiple concurrent edits.

## Platform Scope

- iOS is the primary development path. See `docs/IOS_WORKFLOW.md` for build, install, launch, and logging commands.
- Native iOS presentations should keep UIKit responsible for alerts, sheets, and keyboard accessories.
- `UIGlassEffect` is public UIKit on iOS 26+. Use `if #available(iOS 26.0, *)`, not runtime probing.
- In Swift bridge code, keep access control consistent with helper type visibility.

## Vendor Zed

- `vendor/zed` is a git submodule and an intentional part of the architecture, not just third-party reference code.
- We patch `vendor/zed` and related GPUI/mobile crates directly when Zedra needs mobile support that upstream GPUI or Zed does not provide yet.
- When changing behavior that touches GPUI, iOS/Android platform crates, rendering, input, or text handling, inspect `vendor/zed` first and treat it as part of the codebase.
- Run vendored package checks through `vendor/zed/Cargo.toml`. The root workspace excludes `vendor`, and `cargo check -p gpui_ios` from the root can hit a Cargo feature resolver panic because `gpui_ios` is only a target-specific path dependency there.
- For editor features, use Zed desktop code as a reference for concepts and architecture, but implement minimal mobile-specific versions in Zedra rather than trying to port desktop behavior wholesale.

## Docs Map

- `docs/CONVENTIONS.md` — imports, logging, git commit subjects, async runtime choice, `WorkspaceState`, platform bridge, scroll container rules
- `docs/ARCHITECTURE.md` — crate boundaries, session flow, auth, RPC, transport
- `docs/DESIGN.md` — product UI tone and component direction
- `docs/IOS_WORKFLOW.md` — iOS build pipeline, FFI workflow, pitfalls
- `docs/PROTOCOL_SPECS.md` — protocol contract
- `docs/TELEMETRY.md` — telemetry events and privacy
- `docs/MANUAL_TEST.md` — manual verification steps for UI and device work
- `vendor/zed/` — GPUI, platform crates, grammars, and desktop reference implementations

## Recent Learnings

- GPUI scroll containers need an explicit `.id(...)`. In nested flex layouts, also constrain the full parent chain with `size_full()` and `min_h_0()` or scrolling can silently fail.
- Prefer `cx.spawn(...)` for UI-thread async work and `session_runtime().spawn(...)` for Tokio session or network work when `cx` is unavailable.
- Session-to-UI flow is `Session / SessionHandle -> ConnectEvent -> SessionState -> WorkspaceState -> Views`. Preserve that layering.
