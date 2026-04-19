
# Zedra

**Mobile remote editor. Code from anywhere.**

One QR scan connects you to your desktop. Full terminal, file browser, git, and AI agents over an encrypted P2P tunnel. Built with GPUI for native GPU-accelerated rendering at 60 FPS.

**Primary Platform**: iOS (Metal renderer via `gpui_ios`)
**Secondary Platform**: Android (wgpu/Vulkan via `gpui_android` + `gpui_wgpu`)

## Rules

### Protocol Governance

`docs/PROTOCOL_SPECS.md` is canonical. Any protocol-layer change MUST update `zedra-rpc/src/proto.rs`, host/client handlers, and `docs/PROTOCOL_SPECS.md` in the same PR.

### Telemetry Governance

`crates/zedra-telemetry/src/lib.rs` defines the canonical `Event` enum — single source of truth for all telemetry.

**Privacy**: never include personal data (usernames, file paths, IPs). Opaque IDs, durations, counts, enum labels, booleans only.

### Critical Design Rules

1. **WorkspaceState = single source of truth** — all display reads from `WorkspaceState`, never `SessionHandle` directly. See `docs/CONVENTIONS.md`.
2. **render() must be pure** — no side effects. Mutations go in event handlers, `cx.spawn()`, or subscriptions.
3. **PlatformBridge** — always `platform_bridge::bridge()`, never call platform APIs directly from UI code.
4. **Logging** — `tracing` everywhere with `use tracing::*;` and `error|warn|info!`, never `log::` directly.
5. **New UI must read `docs/DESIGN.md` first** — before creating or redesigning any UI, review `docs/DESIGN.md` and follow its tone, spacing, typography, and component guidance.

### GPUI Conventions

#### Context Types

- `App` — root context, read/update entities. Functions taking `&App` also accept `&Context<T>`.
- `Context<T>` — provided when updating `Entity<T>`, derefs to `App`.
- `Window` — window state (focus, actions, drawing). Passed as `window`, comes before `cx`.
- `AsyncApp` — provided by `cx.spawn`, can be held across await points.

#### Entity (State Container)

`Entity<T>` is a handle to state `T`. All stateful components are entities.

- Create: `cx.new(|cx| T::new(...))`
- Read: `entity.read(cx)` returns `&T`
- Mutate: `entity.update(cx, |this, cx| { ... })` — provides `Context<T>`, returns closure value
- Signal re-render: `cx.notify()` inside update closures
- Store subscriptions: `_subscriptions: Vec<Subscription>` to keep them alive
- Avoid updating an entity while it's already being updated (panics)

#### Scroll Containers

- `overflow_scroll()` and `overflow_y_scroll()` require the `Div` to have a stable `.id(...)`.
- Always assign an explicit id before using GPUI scroll overflow helpers.
- In nested flex layouts, especially embedded native iOS sheets, the full parent chain must constrain height.
- Use `size_full()` on the hosted viewport and `min_h_0()` on each flex child between the window root and the scroll node, or GPUI may measure the scroll area at content height and scrolling will silently fail.
- For native-sheet GPUI content, keep the UIKit gesture bridge minimal and follow `docs/GPUI_NATIVE_PRESENTATIONS.md`.

#### Events vs Actions

**Events** (`cx.emit` + `cx.subscribe`) — child→parent communication:
- Child: `impl EventEmitter<MyEvent> for MyComponent {}` + `cx.emit(MyEvent::Something)`
- Parent: `cx.subscribe(&child_entity, |this, _emitter, event, cx| ...)`
- Used for: state changes parents react to (e.g. `WorkspaceEvent::Disconnected`, `HomeEvent::NavigateToWorkspace`)

**Actions** (`dispatch_action` + `on_action`) — cross-component commands through the view tree:
- Define: `#[derive(Clone, PartialEq, Action)] #[action(namespace = workspace, no_json)]`
- Dispatch: `window.dispatch_action(MyAction.boxed_clone(), cx)` — bubbles up the view tree
- Handle: `.on_action(cx.listener(Self::handle_my_action))` in `render()`
- Used for: commands from deep views handled by an ancestor (e.g. drawer button dispatches `ToggleDrawer`, `Workspace` handles it)

**When to use which**: events when parent holds `Entity` reference to child. Actions when sender doesn't know who handles it (decoupled, bubbles through view tree).

#### Concurrency

All entity use and UI rendering is on a single foreground thread.

- **`cx.spawn(async move |this, cx| { ... })`** — preferred for async work. Runs on foreground thread. `this: WeakEntity<T>`, `cx: &mut AsyncApp`. Use `this.update(cx, |this, cx| ...)` to mutate state on completion. Returns `Task<R>` — must be awaited, `.detach()`-ed, or stored (dropped = cancelled).
- **`cx.background_spawn(async move { ... })`** — for CPU work on background threads. Often awaited by a foreground task that updates state with results.
- **`session_runtime().spawn(...)`** — Tokio runtime for network I/O only. Use when `cx` is unavailable (e.g. inside native platform callbacks). Prefer `cx.spawn` everywhere else.

#### Separation of Concerns (Session → UI)

```
Session / SessionHandle  — networking + RPC on Tokio (no GPUI dep)
        ↓ mpsc::channel<ConnectEvent>
SessionState (Entity)    — UI-thread state, apply_event() in cx.spawn loop
        ↓ sync_from_session()
WorkspaceState (Entity)  — display state, persisted to workspaces.json
        ↓ .read(cx) in render()
Views (WorkspaceContent, WorkspaceDrawer, panels)  — pure rendering
```

- `Workspace` orchestrates: wires Session → SessionState → WorkspaceState, handles actions
- Views only read `Entity<WorkspaceState>` or `Entity<SessionState>` — never `SessionHandle`
- Event bridge: `Session` emits `ConnectEvent` via mpsc. `Workspace.connect()` takes the receiver, processes in `cx.spawn` loop, applies to `SessionState` entity on UI thread

### Native iOS UI Effects

- Prefer native UIKit for keyboard accessories, alerts, sheets — not GPUI.
- `UIGlassEffect` is public UIKit on iOS 26+ (`UIVisualEffect` subclass). Use compile-time `if #available(iOS 26.0, *)`, not runtime class probing.
- In Swift native integration code, keep access control consistent across helper types and APIs. If a return type or stored property uses a `fileprivate` type, the function/property must also be `fileprivate` unless you intentionally widen the type visibility.

## Quick Start

### iOS (Primary)

```bash
git submodule update --init --recursive
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
./scripts/run-ios.sh device              # full build + install + launch
./scripts/ios-log.sh [--filter <pattern>] # stream device logs
```

See `docs/IOS_WORKFLOW.md` for full pipeline.

### Android

```bash
git submodule update --init --recursive
rustup target add aarch64-linux-android
./scripts/dev-cycle.sh                   # build + install + launch
```

### Prerequisites

| | iOS | Android |
|---|---|---|
| Build tool | Xcode 26+, xcodegen, libimobiledevice | Android NDK r25c+, Android SDK API 31+ |
| Rust targets | `aarch64-apple-ios`, `aarch64-apple-ios-sim` | `aarch64-linux-android` |
| Device | Physical (or simulator) | Physical device |

## Architecture

**iOS**: `Swift UIKit runtime → C FFI ↔ Rust → GPUI → Metal`
**Android**: `JNI Thread → Command Queue → Main Thread → GPUI → wgpu → Vulkan`

## Project Structure

```
crates/
  zedra/              # Mobile cdylib (iOS + Android) — app views, workspace, platform bridge
  zedra-host/         # Desktop host daemon — CLI, RPC, PTY, filesystem, git handlers
  zedra-session/      # Mobile client — SessionHandle, iroh connection, auto-reconnect
  zedra-terminal/     # Terminal emulation — alacritty VTE + GPUI rendering
  zedra-rpc/          # Protocol types + QR pairing codec
  zedra-telemetry/    # Pure telemetry — typed Event enum + TelemetryBackend trait
ios/                  # Xcode project (xcodegen from project.yml)
android/              # Android app (Gradle)
vendor/zed/crates/    # GPUI platform crates (gpui_ios, gpui_android, gpui_wgpu)
deploy/relay/         # Production iroh-relay deployment
```

## What Works

- 60 FPS GPU rendering: Metal (iOS), wgpu/Vulkan (Android)
- Touch input: tap, scroll, drawer pan, fling momentum
- Navigation: DrawerHost (slide-from-left), push/pop views
- Code editor: tree-sitter syntax highlighting, cursor, virtual scrolling
- Remote terminal: alacritty VTE, bidi streaming, PTY resize
- iroh transport: QUIC/TLS 1.3 direct P2P, relay fallback
- PKI authentication: QR pairing (HMAC-SHA256), Ed25519 challenge-response reconnect
- Session persistence: `workspaces.json`, PTY survival across reconnects
- Auto-reconnect: exponential backoff (1s–30s), terminal output buffers, backlog replay
- Connection monitoring: RTT, path type (direct/relay), byte stats
- Firebase Analytics + Crashlytics (iOS), GA4 analytics (host daemon)

## Pre-Commit Checks

```bash
cargo fmt
cargo check -p zedra-rpc -p zedra-session -p zedra-terminal -p zedra-host
bun run format   # JS/TS auto-fix
bun run check    # JS/TS CI mode
```

## Documentation

| Doc | Topic |
|-----|-------|
| `docs/CONVENTIONS.md` | Code conventions, imports, logging, WorkspaceState |
| `docs/ARCHITECTURE.md` | Architecture, crates, auth flow, RPC methods |
| `docs/GET_STARTED.md` | Build setup for iOS, Android, host daemon |
| `docs/IOS_WORKFLOW.md` | iOS build pipeline, FFI, pitfalls |
| `docs/PROTOCOL_SPECS.md` | Protocol/RPC contract |
| `docs/TELEMETRY.md` | Telemetry events, privacy, backend setup |
| `docs/RELAY.md` | Relay deployment |
| `docs/NETWORK_TRANSPORT.md` | iroh transport, NAT traversal |

---

**Last Updated**: 2026-04-17
