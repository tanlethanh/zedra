# Zedra

Mobile remote editor for iOS and Android. Primary platform is iOS (`gpui_ios` + Metal). Secondary platform is Android (`gpui_android` + `gpui_wgpu` + Vulkan).

`docs/CONVENTIONS.md` is canonical for code style, error handling, logging, git commits, PRs, docs style, theming, icon usage, async runtime selection, GPUI entity/task/layout rules, and `WorkspaceState` data flow. This file does not repeat them.

## Repo Map

- `crates/zedra` — mobile app: GPUI UI, workspace orchestration, platform bridge, iOS/Android native glue
- `crates/zedra-host` — desktop daemon + `zedra` CLI: iroh endpoint, auth, sessions, PTY, fs/git, managed agents
- `crates/zedra-session` — client-side connect/reconnect, auth, session events, remote terminal attach
- `crates/zedra-terminal` — reusable terminal emulator + GPUI renderer (alacritty model, input/IME, OSC)
- `crates/zedra-rpc` — irpc protocol types and QR pairing between client and host
- `crates/zedra-osc` — packet-safe OSC scanner for PTY byte streams
- `crates/zedra-telemetry` — typed telemetry events with per-runtime backend injection
- `ios/`, `android/` — native app shells (xcodegen project, Gradle)
- `packages/` — TypeScript workspaces, formatted by biome: `landing` (Astro/Starlight site for zedra.dev), `relay-check` (CLI), `relay-monitor` (Dockerized relay health monitor)
- `scripts/` — build/run/log/asset/setup tooling; `vendor/zed` — patched GPUI submodule
- `crates/zedra`, `zedra-host`, `zedra-session`, and `zedra-terminal` each have their own `AGENTS.md` — read it before working in that crate.

## Everyday Commands

- First-time setup: `git submodule update --init --recursive`; full prerequisites in `docs/GET_STARTED.md`.
- Host CLI (binary is named `zedra`): `cargo run -p zedra-host -- start|status|qr|setup|agent|logs ...`. The daemon is per-workspace — a workspace lock refuses a second `start` in the same workdir; use `start --detach` for background runs.
- iOS app: `./scripts/run-ios.sh [sim|device]`; logs via `./scripts/ios-log.sh tail`. Details: `docs/IOS_WORKFLOW.md`.
- Android app: `./scripts/run-android.sh` (never `adb shell am start` directly); logs via `./scripts/log-android.sh`.
- `zedra setup` flows: verify in the throwaway sandbox — `scripts/setup-sandbox.sh zedra setup <agent>` (shimmed provider CLIs, no network, real `$HOME` untouched).
- Icons: `bun run icons:gen` after adding an SVG.

## Agent Workflow

- Inspect the relevant code paths first and infer local patterns before proposing or making changes. Read the owning crate's `AGENTS.md`.
- When a request mentions the Zedra CLI or `zedra` command-line behavior, inspect `crates/zedra-host` first; that crate owns the daemon and CLI entrypoints.
- Ask before making any meaningful product or architectural decision. Tiny details may follow existing patterns without approval.
- Prefer the smallest diff that fits the current design. If the structure is blocking quality, propose the refactor and wait for approval.
- Keep code concise and modular; prefer clarifying code over comments. Comments stay 1 line (up to 3 only for genuinely confusing parts), same for doc comments. Add a minimal comment at regression-prone guards explaining the invariant.
- Prioritize correctness and clarity over cleverness or speed unless performance is the explicit problem.
- Avoid panic-prone shortcuts such as unchecked indexing or `unwrap()` in normal code paths. Propagate or handle errors.
- Surface blockers quickly with a recommendation. Keep progress updates short and include reasoning or tradeoffs.

## Debugging Workflow

- Read the relevant code path deeply before changing behavior.
- On mobile issues, prefer targeted `tracing` logs with a clear searchable prefix so the developer can run the app, reproduce, and return logs.
- After the first failed debugging attempt, stop and ask for more information instead of arguing from hypotheses.
- Prefer root-cause fixes once the issue is confirmed.

## Repo Invariants

- `WorkspaceState` is the single source of truth for display state; `render()` stays pure; platform access goes through `platform_bridge::bridge()`; logging uses `tracing`, never `log::`. Details and data flow: `docs/CONVENTIONS.md`.
- Read `docs/DESIGN.md` before creating or redesigning UI, and `docs/THEMING.md` before adding colors — `theme::` tokens only, no hardcoded hex in views.
- GPUI tasks cancel when the `Task` handle drops. Inside entity `update`/`read_with` closures, use the inner `cx`; reentrant updates panic.

## Managed Agents

- `docs/MANAGED_AGENTS.md` is canonical for adding or changing AI-agent support. The host actor is the source of truth; the app adapter is optional branding/behavior.
- One `AgentActor` per agent in `crates/zedra-host/src/agent/<slug>.rs`, registered in the `ACTORS` array in `agent/mod.rs`. Never add per-agent `match` arms to the REST API, host cache, CLI scans, hook dispatch, or installed-agent list — everything resolves through the registry.
- Agents are stable slug strings over RPC. Adding one never bumps ALPN or adds a protocol enum. The app holds no agent enum either — icons and display names resolve from the slug (missing icon falls back to the terminal glyph); app adapters in `crates/zedra/src/agent/mod.rs` override branding or behavior only.
- Agent hooks are debug-builds-only (`hooks_enabled()` is `cfg!(debug_assertions)`). Hook scripts installed by `zedra setup` call back into the host's token-authenticated localhost REST API via `zedra agent hook receive --agent <slug>`.

## Protocol And Telemetry

- `docs/PROTOCOL_SPECS.md` is canonical. Any protocol change must update `zedra-rpc/src/proto.rs`, the relevant host and client handlers, and `docs/PROTOCOL_SPECS.md` in the same change.
- ALPN bumps keep backward compatibility: the previous protocol is frozen as `zedra-rpc/src/proto_v{N}.rs` and served alongside the live one until the App Store transition completes (currently `zedra/rpc/4` live, `proto_v3` frozen).
- `crates/zedra-telemetry/src/lib.rs` defines the canonical telemetry `Event` enum.
- Telemetry must not include personal data. Use opaque IDs, durations, counts, enum labels, and booleans only.

## Zedra Delta Backend

- Zedra Delta is Zedra's cloud/backend service — push notifications, Live Activities, workspace sync signals, and any other backend capability. See `docs/DELTA_INTEGRATION.md`.
- The Delta source lives in a separate repo at `$ZEDRA_DELTA_REPO`. When a task needs Delta backend behavior, read and edit that repo like part of this codebase, and keep the host/mobile protocol contract in sync across both repos.
- On the app side, Delta represents auth and backend interaction only (sign-in, push-token registration, backend calls) — not app features that merely arrive over Delta.

## Documentation

- Read `docs/WRITING.md` before writing or editing documentation; style rules are in `docs/CONVENTIONS.md` § Documentation Style.
- Keep repo guidance high-signal: new rules must be non-obvious, repeatedly encountered, and specific enough to act on. Crate-specific rules belong in that crate's `AGENTS.md`.

## GitHub Issues

- Search existing open issues first to avoid duplicates.
- Use the matching template from `.github/ISSUE_TEMPLATE/` when one exists; preserve its headings and required fields, omitting only optional sections that clearly do not apply. Without a template, use `## Summary`, `## Reproduction`, `## Expected Behavior`, `## Actual Behavior`, `## Notes`.
- Keep issues concrete: problem, affected platform/scope, repro steps, expected vs actual, relevant logs or screenshots.

## Validation

- Prefer targeted checks over broad suites; run the checks that match what you touched.
- Add or update tests when there is an obvious existing place for them.
- For UI, platform, and device-driven changes, add or update manual verification steps in `docs/MANUAL_TEST.md`.
- CI-parity checks:
  - `cargo fmt --check`
  - `cargo check -p zedra-rpc -p zedra-session -p zedra-terminal -p zedra-host`
  - `cargo test -p zedra-rpc -p zedra-session -p zedra-terminal -p zedra-host -p zedra-osc -p zedra-telemetry`
  - `cargo check -p zedra --features ios-platform --target aarch64-apple-ios` for
    app-crate changes touching anything gated behind a real iOS dependency
    (e.g. `gpui_ios`, which is a `[target.'cfg(target_os = "ios")'.dependencies]`
    entry — a plain host-target `cargo check -p zedra` silently excludes it
    from the graph, so a feature/method that only compiles with `gpui_ios`
    present can pass a host check while failing this one; real CI runs both,
    see `.github/workflows/ci.yml`)
  - `cargo test -p zedra --features ios-platform` for app-crate changes
    (host target — tests must actually execute, so this one intentionally
    doesn't cross-compile and won't catch the gap above)
  - `cargo check --manifest-path vendor/zed/Cargo.toml -p gpui_ios -p gpui` for vendored GPUI/iOS framework patches
  - `bun run check` (biome) for `packages/` changes
- Host integration tests (`crates/zedra-host/tests/integration.rs`) spawn an in-process iroh relay — they run offline, no external network needed.

## Git Commits

- Subjects use `feat|fix|chore|docs: <description>`, with a scope when applicable (`fix(host): ...`). PR titles use the same format; bodies use only `## Summary` and `## Notes`.
- Commit only changes related to the current feature or fix, even when the worktree has concurrent edits.
- Full rules, including the `vendor/zed` submodule style and the Codex co-author trailer: `docs/CONVENTIONS.md` § Git Commits.

## Platform Scope

- iOS is the primary development path; Android is secondary.
- Native iOS presentations should keep UIKit responsible for alerts, sheets, and keyboard accessories.
- `UIGlassEffect` is public UIKit on iOS 26+. Use `if #available(iOS 26.0, *)`, not runtime probing.

## Icon Assets

- `crates/zedra/assets/icons/<slug>.svg` is the single source of truth; the kebab-case slug is the icon's name on every platform. Commit only the SVGs. Usage in code: `docs/CONVENTIONS.md` § Icons and Assets.
- Generated assets are gitignored — never hand-edit: iOS imagesets come from `scripts/generate-assets.sh` (Xcode pre-build script), Android drawables from the Gradle `generateIconDrawables` task (`preBuild` dependency).
- Add an icon by dropping a `currentColor` SVG at the path above, then run `scripts/generate-assets.sh` (or `bun run icons:gen`). No network fetch — source the SVG yourself.
- Android conversion uses Android Studio's `Svg2Vector` (`com.android.tools:sdk-common`, AGP-pinned in `android/build.gradle`) because lighter converters silently drop inherited `fill`/`stroke` and basic shapes. Future remote/dynamic icons should use a runtime image loader (Coil + coil-svg on Android), not this bundled pipeline.

## Vendor Zed

- `vendor/zed` is a git submodule and an intentional part of the architecture. It tracks the fork `tanlethanh/zed`, branch `feat/gpui-mobile` — mobile GPUI patches land there, then the submodule pointer updates here.
- When changing behavior that touches GPUI, iOS/Android platform crates, rendering, input, or text handling, inspect `vendor/zed` first and treat it as part of the codebase.
- Run vendored package checks through `vendor/zed/Cargo.toml`. The root workspace excludes `vendor`, and `cargo check -p gpui_ios` from the root can hit a Cargo feature resolver panic.
- For editor features, use Zed desktop code as a reference for concepts, but implement minimal mobile-specific versions rather than porting desktop behavior wholesale.

## Docs Map

- `docs/GET_STARTED.md` — prerequisites, toolchains, submodules, env vars
- `docs/CONVENTIONS.md` — imports, Rust style, error handling, GPUI lifecycle, logging, git commits, docs style, async runtime choice, `WorkspaceState`, platform bridge, layout/scroll rules
- `docs/ARCHITECTURE.md` — crate boundaries, session flow, auth, RPC, transport
- `docs/NETWORK_TRANSPORT.md` — QR pairing, PKI auth, discovery, relay fallback
- `docs/PROTOCOL_SPECS.md` — protocol contract
- `docs/MANAGED_AGENTS.md` — adding agent support: host actor registry, setup flow, app adapter
- `docs/EXTENSIONS_SYSTEM.md` — design notes for manifest-driven dynamic agent support
- `docs/DELTA_INTEGRATION.md` — every integration point between Zedra and the Delta backend
- `docs/LIVE_ACTIVITY.md` — aggregate Live Activity design (Dynamic Island, lock screen)
- `docs/DESIGN.md` — product UI tone and component direction
- `docs/THEMING.md` — theme tokens, light/dark, editor/terminal palettes, subscription pattern
- `docs/IOS_WORKFLOW.md` — iOS build pipeline, FFI workflow, pitfalls
- `docs/GPUI_*.md` — GPUI internals playbooks: rendering model, focus/input/keyboard, animations, mobile interaction, text protocol, native presentations, Android backend
- `docs/TELEMETRY.md` — telemetry events and privacy
- `docs/MANUAL_TEST.md` — manual verification steps for UI and device work
- `docs/WEBVIEW.md` — generic native in-app webview API (`webview.rs`): config, messaging, JS eval, navigation interception
- `docs/WEB_TUNNEL.md` — localhost web tunnel transport; `docs/WEB_TUNNEL_MODES.md` — exact-port vs alias adapters and origin tradeoffs
- `docs/DEVTOOL.md` — in-app HTTP devtool for agent-driven UI scripting (debug iOS + Android); wrapper at `scripts/devtool.sh`
- `docs/RELAY.md` — self-hosted iroh-relay deployment
- `docs/RELEASE.md` — how to cut a release
- `docs/WRITING.md` — documentation style guide
- `vendor/zed/` — GPUI, platform crates, grammars, and desktop reference implementations
