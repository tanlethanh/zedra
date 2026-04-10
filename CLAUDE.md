# Zedra

**Mobile remote editor. Code from anywhere.**

One QR scan connects you to your desktop. Full terminal, file browser, git, and AI agents over an encrypted P2P tunnel. Built with GPUI for native GPU-accelerated rendering at 60 FPS.

**Primary Platform**: iOS (Metal renderer via `gpui_ios`)
**Secondary Platform**: Android (wgpu/Vulkan via `gpui_android` + `gpui_wgpu`)

## Protocol Governance (Required)

`docs/PROTOCOL_SPECS.md` is the canonical protocol and RPC contract document.

Any change that touches protocol-layer behavior MUST:

1. Update `crates/zedra-rpc/src/proto.rs` as needed.
2. Update host/client protocol handlers (`zedra-host` + `zedra-session`).
3. Update or explicitly reference `docs/PROTOCOL_SPECS.md` in the same change.
4. Preserve protocol compatibility unless a breaking change is intentionally documented.

If code and doc ever diverge, align both immediately in the same PR.

## Telemetry Governance (Required)

`crates/zedra-telemetry/src/lib.rs` defines the canonical `Event` enum — the single source of truth for all telemetry events across app, host, and shared crates.

### Adding telemetry for new features

Any change that adds a **user-facing feature or significant behavior** MUST define telemetry events for it:

1. **Add a typed `Event` variant** in `crates/zedra-telemetry/src/lib.rs` with a dedicated context struct carrying relevant fields (timing, counts, transport state, enum labels).
2. **Implement `to_params()`** serialization for the new variant.
3. **Instrument the call site** using `zedra_telemetry::send(Event::Variant(...))`.
4. **Include meaningful context**: connection timing/phase, transport path (direct/relay), ALPN, relay URL, network classification, version info — whatever is relevant to understanding the feature's behavior in production.

### Privacy rules

- **Never** include personal data: usernames, file paths, file contents, IP addresses, hostnames.
- Use opaque IDs only (node ID short forms, session IDs, terminal IDs).
- Durations, counts, enum labels, and boolean flags are always safe.
- `record_panic` strips filesystem paths via `sanitize_panic_message()`.

### Architecture

```
zedra-telemetry (pure crate, no platform deps)
  ├── Event enum        — typed variants with context structs
  ├── TelemetryBackend  — trait: send(), record_error(), record_panic(), ...
  ├── send(Event)       — global free function, delegates to registered backend
  └── init(Box<dyn TelemetryBackend>)  — called once at startup

App (iOS/Android):  FirebaseBackend  → crates/zedra/src/telemetry.rs
Host (GA4):         HostBackend      → crates/zedra-host/src/telemetry.rs
No backend:         silent no-op (default)
```

### Current event catalog

**App events** (mobile + shared crates):

| Event | Context | Crate |
|-------|---------|-------|
| `AppOpen` | saved_workspaces, app_version, platform, arch | zedra |
| `ScreenView` | screen name | zedra |
| `QrScanInitiated` | — | zedra |
| `ConnectSuccess` | phase timings, path, network, relay, ALPN, NAT, ipv4/6 | zedra-session |
| `ConnectFailed` | phase, error label, relay, ALPN, discovery state | zedra-session |
| `SessionResumed` | terminal_count, resume_ms | zedra |
| `Disconnect` | — | zedra |
| `ReconnectStarted` | reason | zedra-session |
| `ReconnectSuccess` | attempt, elapsed_ms, reason, phase timings, path, network, rtt_ms, relay, alpn, has_ipv4/6 | zedra-session |
| `ReconnectExhausted` | attempts, elapsed_ms, reason | zedra-session |
| `PathUpgraded` | network, rtt_ms, from_relay | zedra-session |
| `TerminalOpened` | source, terminal_count | zedra, workspace_view |
| `TerminalClosed` | remaining count | workspace_view |

**Host events** (desktop daemon):

| Event | Context | Crate |
|-------|---------|-------|
| `DaemonStart` | relay_type | zedra-host |
| `NetReport` | has_ipv4, has_ipv6, symmetric_nat | zedra-host |
| `ClientPaired` | — | zedra-host |
| `AuthSuccess` | is_new_client, duration_ms, path_type | zedra-host |
| `AuthFailed` | reason | zedra-host |
| `SessionEnd` | duration_ms, terminal_count, path_type | zedra-host |
| `HostTerminalOpen` | has_launch_cmd | zedra-host |
| `DaemonHeartbeat` | uptime_secs, session_count, terminal_count | zedra-host |
| `BandwidthSample` | bytes_sent, bytes_recv, interval_secs | zedra-host |

### Runtime opt-out

- **Host**: `--no-telemetry` flag or `ZEDRA_TELEMETRY=0` env var.
- **App**: `zedra_telemetry::set_enabled(false)` at runtime (also disables Firebase SDK collection).

## Quick Start

### iOS (Primary)

```bash
# First-time setup
git submodule update --init --recursive
rustup target add aarch64-apple-ios aarch64-apple-ios-sim

# Full build + install + launch on connected device
./scripts/run-ios.sh device

# Incremental ObjC-only rebuild (~5s vs ~60s full)
cd ios && xcodegen generate && xcodebuild build -project Zedra.xcodeproj -scheme Zedra ...

# Stream device logs
./scripts/ios-log.sh [--filter <pattern>] [--select-device]
```

See `docs/IOS_WORKFLOW.md` for complete build pipeline, FFI patterns, and debugging.

### Android

```bash
# First-time setup
git submodule update --init --recursive
rustup target add aarch64-linux-android

# Build + install + launch
./scripts/dev-cycle.sh

# Or manual:
./scripts/build-android.sh
cd android && ./gradlew installDebug && cd ..
adb logcat | grep zedra
```

### Prerequisites

| | iOS | Android |
|---|---|---|
| Build tool | Xcode 26+, xcodegen, libimobiledevice | Android NDK r25c+, Android SDK API 31+ |
| Rust targets | `aarch64-apple-ios`, `aarch64-apple-ios-sim` | `aarch64-linux-android` |
| Device | Physical (or simulator) | Physical device (emulator not tested) |
| Submodules | `git submodule update --init --recursive` | same |

## Architecture

### High-Level Design

**iOS**: `ObjC UIKit → FFI → Rust → GPUI → Metal`

**Android**: `JNI Thread → Command Queue → Main Thread → GPUI → wgpu → Vulkan`

### Key Components

**Shared (both platforms)**:
- **`crates/zedra/src/app.rs`** — `ZedraApp`: root view, workspace management, QR pairing, deeplinks
- **`crates/zedra/src/workspace_view.rs`** — per-session workspace: `DrawerHost` + header/main-view stack, wired to `SessionHandle`
- **`crates/zedra/src/workspace_drawer.rs`** — `WorkspaceDrawer`: tabbed sidebar (files, git, terminal, session)
- **`crates/zedra/src/workspace_state.rs`** — `WorkspaceState`: single source of truth for display data; persisted to `workspaces.json`
- **`crates/zedra/src/platform_bridge.rs`** — `PlatformBridge` trait + global accessor; never call platform APIs directly from UI code
- **`crates/zedra/src/mgpui/`** — mobile GPUI primitives: `DrawerHost`, keyboard `input`

**iOS platform** (`vendor/zed/crates/gpui_ios/`):
- `IosWindow` — Metal-backed window, touch input, safe area insets, UIKit lifecycle
- `ios/Zedra/ZedraFirebase.m` — Firebase Analytics + Crashlytics (ObjC)
- `ios/Zedra/SwiftCompatibilityShim.swift` — required for Firebase static pods (must exist)

**Android platform** (`vendor/zed/crates/gpui_android/`, `gpui_wgpu/`):
- `AndroidWindow` — surface lifecycle, atlas sharing; must use `WgpuRenderer::new_with_atlas()` (not `new()`)
- `src/android/command_queue.rs` — bounded channel (512); use `try_send()`, never block JNI thread
- `src/android/jni.rs` — all `#[no_mangle]` entry points must wrap body in `jni_call("name", || {...})`
- `src/android/app.rs` — touch handling (tap, scroll, drawer pan, fling) in addition to surface/window lifecycle

### Critical Design Decisions

1. **WorkspaceState as Single Source of Truth** — All display reads from `WorkspaceState`, never `SessionHandle` directly. `SessionHandle` fields are empty during connecting; `WorkspaceState` is seeded from persisted data before connection starts. See `docs/CONVENTIONS.md` § "WorkspaceState as Single Source of Truth".

2. **render() must be pure** — `render()` is side-effect free. Mutations/async work go in event handlers, `cx.spawn()`, or subscriptions. Use `SharedPendingSlot<T>` as the async-to-UI bridge pattern.

3. **PlatformBridge** — always access platform APIs via `platform_bridge::bridge()`. Never call platform APIs directly from UI code so both platforms share call sites.

4. **Pixel Handling (Android)** — logical pixels × scale = physical pixels. Conversion at `window.rs:handle_surface_created()`. Wrong conversion = black screen.

5. **Atlas Sharing (Android)** — `WgpuRenderer::new_with_atlas()` is mandatory. Using `new()` creates a separate atlas → index-out-of-bounds crash during draw.

## Project Structure

```
Cargo.toml                      # Workspace root
crates/
  ├── zedra-terminal/           # Terminal emulation (alacritty VTE + GPUI rendering)
  ├── zedra-rpc/                # Protocol types + QR pairing codec
  │   └── src/
  │       ├── proto.rs          # ZedraProto enum + all request/response types
  │       └── pairing.rs        # ZedraPairingTicket encode/decode
  ├── zedra-telemetry/           # Pure telemetry: typed Event enum + TelemetryBackend trait
  │   └── src/
  │       └── lib.rs            # Event enum, context structs, global dispatch, backend trait
  ├── zedra-session/            # Mobile client: SessionHandle, iroh connection, auto-reconnect
  │   └── src/
  │       ├── lib.rs            # SessionHandle, terminal buffers, reconnect state machine
  │       └── signer.rs         # ClientSigner trait + FileClientSigner (Ed25519)
  ├── zedra/                    # Mobile cdylib (iOS + Android)
  │   └── src/
  │       ├── lib.rs            # Module declarations + platform entry points
  │       ├── app.rs            # ZedraApp: root view, workspace management, QR pairing
  │       ├── workspace_view.rs # Per-session workspace: DrawerHost + views, SessionHandle wiring
  │       ├── workspace_drawer.rs # WorkspaceDrawer: tabbed sidebar
  │       ├── workspace_state.rs # WorkspaceState: persisted display data, workspaces.json
  │       ├── connecting_view.rs # Connection progress screen
  │       ├── home_view.rs      # Home/add-workspace screen
  │       ├── terminal_panel.rs # Terminal tab panel
  │       ├── terminal_card.rs  # Terminal card component
  │       ├── session_panel.rs  # Session info panel
  │       ├── transport_badge.rs # Connection status badge
  │       ├── file_explorer.rs  # FileExplorer tree view
  │       ├── quick_action_panel.rs # Quick action panel
  │       ├── active_terminal.rs # Active terminal state management
  │       ├── telemetry.rs      # FirebaseBackend: registers Firebase backend with zedra-telemetry
  │       ├── platform_bridge.rs # PlatformBridge trait + global accessor
  │       ├── pending.rs        # SharedPendingSlot<T>: async→main-thread channel
  │       ├── keyboard.rs       # Keyboard handler factories
  │       ├── fonts.rs          # Font loading
  │       ├── theme.rs          # Color constants, inset helpers
  │       ├── deeplink.rs       # Deep link / QR URL handling
  │       ├── button.rs         # Shared button component
  │       ├── app_preview.rs    # App preview component
  │       ├── ios.rs            # iOS module declarations
  │       ├── ios_stub.c        # Weak FFI stubs for iOS
  │       ├── android/          # Android platform integration
  │       │   ├── app.rs        # Main-thread GPUI app, surface/window lifecycle, touch handling
  │       │   ├── jni.rs        # JNI bridge; all entry points use jni_call()
  │       │   ├── command_queue.rs # Crossbeam command queue (bounded 512)
  │       │   └── bridge.rs     # AndroidBridge: PlatformBridge impl
  │       ├── ios/              # iOS platform integration
  │       │   ├── app.rs        # iOS app lifecycle + GPUI init
  │       │   ├── bridge.rs     # IosBridge: PlatformBridge impl + iOS FFI exports
  │       │   ├── analytics.rs  # Firebase Analytics bridge
  │       │   ├── logger.rs     # NSLog bridge
  │       │   └── nslog_bridge.m
  │       ├── mgpui/            # Mobile GPUI primitives (shared)
  │       │   ├── drawer_host.rs # DrawerHost: slide-from-left overlay
  │       │   └── input.rs      # Keyboard input handling
  │       └── editor/           # Code editor components
  │           ├── code_editor.rs, git_sidebar.rs, git_diff_view.rs
  │           ├── syntax_highlighter.rs, syntax_theme.rs, text_buffer.rs
  └── zedra-host/               # Desktop host daemon
      └── src/
          ├── main.rs           # CLI: start / client / stop
          ├── rpc_daemon.rs     # RPC dispatch, PKI auth, TermAttach bidi streaming
          ├── session_registry.rs # PKI sessions: ACLs, pairing slots, persistence
          ├── iroh_listener.rs  # iroh Endpoint creation + accept loop
          ├── identity.rs       # Persistent Ed25519 host identity (~/.config/zedra/)
          ├── qr.rs             # QR code generation (terminal + JSON output)
          ├── ga4.rs            # GA4 Measurement Protocol HTTP transport (telemetry_id, send, panic hook)
          ├── telemetry.rs      # HostBackend: bridges GA4 ↔ zedra-telemetry
          ├── fs.rs             # Filesystem RPC handlers (list, read, write, stat, remove, mkdir)
          ├── git.rs            # Git RPC handlers (status, diff, log, commit, branches, checkout)
          └── pty.rs            # PTY management (spawn, resize, I/O streaming)

ios/                            # Xcode project (xcodegen from project.yml)
  ├── project.yml               # OTHER_LDFLAGS must include $(inherited) before -ObjC -all_load
  ├── Podfile                   # use_frameworks! :linkage => :static
  └── Zedra/
      ├── ZedraFirebase.m, ZedraQRScanner.m, main.m
      └── SwiftCompatibilityShim.swift  # Must exist — Firebase static pods need Swift

android/app/src/main/java/dev/zedra/app/
  ├── MainActivity.java, GpuiSurfaceView.java, QRScannerActivity.java

vendor/zed/crates/
  ├── gpui_ios/src/ios/         # iOS GPUI platform (Metal renderer)
  ├── gpui_android/src/android/ # Android GPUI platform (wgpu)
  └── gpui_wgpu/src/            # wgpu renderer (shaders, atlas, context)

packages/relay-worker/          # [DEPRECATED] do not use
packages/relay-monitor/         # Docker sidecar (Discord + SSH to local relay) — deploy/relay
packages/relay-check/           # Local CLI: SSH to relay hosts, print metrics/history
deploy/relay/                   # Production iroh-relay binary deployment
```

### Dependency Graph

```
zedra-telemetry (pure: typed Event enum, TelemetryBackend trait, global dispatch)
    ↑
zedra-rpc (protocol types, QR pairing codec)
    ↑
zedra-session (iroh connection, RPC, auto-reconnect — emits telemetry events)
    ↑
zedra-terminal (VTE emulation, sends input via zedra-session)
    ↑
zedra (app cdylib — registers FirebaseBackend, emits app-level events)

zedra-host (iroh listener, RPC daemon — registers GA4 HostBackend)
    ↑
zedra-rpc + zedra-telemetry
```

## What Works

- 60 FPS GPU rendering: Metal (iOS), wgpu/Vulkan (Android)
- Touch input: tap, scroll, drawer pan, fling momentum
- Navigation: DrawerHost (slide-from-left), StackNavigator (push/pop)
- Code editor: tree-sitter syntax highlighting, cursor, virtual scrolling
- Remote terminal: alacritty VTE, bidi streaming, PTY resize
- iroh transport: QUIC/TLS 1.3 direct P2P, relay fallback
- PKI authentication: QR pairing (HMAC-SHA256 registration), Ed25519 challenge-response reconnect
- Session persistence: `workspaces.json`, PTY survival across reconnects
- Auto-reconnect: exponential backoff (1s–30s), terminal output buffers, backlog replay
- Connection monitoring: RTT, path type (direct/relay), byte stats
- Firebase Analytics + Crashlytics (iOS), GA4 analytics (host daemon)
- `zedra client` CLI: RTT measurement, relay vs P2P statistics

## Known Limitations (Technical Debt)


1. **Terminal Session Persistence** (High Priority)
   - PTYs survive disconnect but fresh clients can't discover or resume them
   - No screen state restoration after long disconnect
   - Solution: server-side `vt100` screen capture + terminal discovery flow

2. **Touch Gestures** (Medium Priority)
   - No pinch-to-zoom or multi-touch gestures yet
   - Solution: multi-pointer tracking in `TouchHandler`

3. **No Real Filesystem Access on Mobile** (Low Priority)
   - Editor shows hardcoded sample files; filesystem browsing goes through host RPC only
   - Solution: Expand host RPC to serve full directory trees

## Common Issues and Solutions

### iOS

- **Swift linker error**: Ensure `ios/Zedra/SwiftCompatibilityShim.swift` exists
- **Firebase crash**: Check `OTHER_LDFLAGS` in `project.yml` includes `$(inherited)` before `-ObjC -all_load`
- **No logs**: Use `./scripts/ios-log.sh` — requires USB-paired device, never use `sudo log collect`

### Android

- **Black screen**: Surface dimensions must be physical pixels (e.g. 1080×2400 not 360×800). Screenshot: `adb shell screencap -p /sdcard/test.png && adb pull /sdcard/test.png /tmp/`
- **Build errors**: Check `ANDROID_NDK_ROOT` is set; verify `rustup target add aarch64-linux-android`
- **Crash on launch**: Check Vulkan 1.1+ — `adb shell getprop ro.hardware.vulkan`

## Pre-Commit Checks (Required)

CI enforces these on push/PR.

```bash
# Rust
cargo fmt
cargo check -p zedra-rpc -p zedra-session -p zedra-terminal -p zedra-host

# JS/TS
bun run format   # auto-fix
bun run check    # CI mode (check only)
```

## Roadmap

- **Phases 1–6.7**: ✅ Complete (rendering, text, input, navigation, transport, PKI auth, reconnect)
- **Phase 7**: Terminal Persistence — server-side vt100 screen capture, fresh client terminal discovery, on-disk credentials
- **Phase 8**: ACP Agent Panel — AI agent supervision UI (Claude Code / Codex relay via host daemon)
- **Phase 9**: Production Hardening — multi-touch, real file access via host RPC, E2E encryption

## Important Notes for Future Development

1. **iOS is the primary platform** — new UI work targets iOS first, Android second
2. **WorkspaceState rule** — display data always flows through `WorkspaceState`; never read `SessionHandle` in display code
3. **PlatformBridge** — always use `platform_bridge::bridge()`, never call platform APIs directly from UI
4. **render() must be pure** — no mutations, no async; use `cx.spawn()` and `SharedPendingSlot`
5. **Logging** — use `tracing::` everywhere, never `log::` directly; see `docs/CONVENTIONS.md` for levels and format
6. **Threading** — all GPUI on main thread; Android: command queue; iOS: ObjC main thread → FFI
7. **Android atlas** — always `WgpuRenderer::new_with_atlas()`, never `new()`
8. **Physical device testing** — always test on real device

## Documentation

- Code conventions (imports, render purity, logging, WorkspaceState, channel notifications): `docs/CONVENTIONS.md`
- Threading / async boundaries (JNI, Tokio session runtime, channel wake): `docs/THREADING.md`
- iOS build pipeline: `docs/IOS_WORKFLOW.md`
- Protocol/RPC: `docs/PROTOCOL_SPECS.md`
- Architecture decisions: `docs/ARCHITECTURE.md`
- Relay deployment: `docs/RELAY.md`
- Terminal persistence design: `docs/TERMINAL_PERSISTENCE.md`
- Debug workflow: `docs/DEBUGGING.md`

- ACP agent panel design: `docs/ACP_PLAN_2.md`

---

**Last Updated**: 2026-04-02
