# Zedra - GPUI on Android

## Project Overview

Zedra is a port of Zed's GPUI UI framework to Android using wgpu for GPU rendering. This is the first successful port of GPUI to a mobile platform.

**Current Status**: Interactive UI with navigation, editor, and touch input working at 60 FPS on Android via wgpu/Vulkan

**Test Device**: Mali-G68 MC4 (Vulkan 1.1.0, 1080x2400 @ 2.75x DPI)

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

App (iOS/Android):  FirebaseBackend  → crates/zedra/src/analytics.rs
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
| `ReconnectSuccess` | attempt, elapsed_ms, reason | zedra-session |
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
| `BandwidthSample` | bytes_sent, bytes_recv, interval_secs | zedra-host |

### Runtime opt-out

- **Host**: `--no-telemetry` flag or `ZEDRA_TELEMETRY=0` env var.
- **App**: `zedra_telemetry::set_enabled(false)` at runtime (also disables Firebase SDK collection).

## Quick Start

### Build and Deploy

```bash
# First-time setup: Initialize git submodules
git submodule update --init --recursive

# Recommended: Automated development cycle (build + install + launch + monitor)
./scripts/dev-cycle.sh

# Or manual steps:
./scripts/build-android.sh                    # Build Rust libraries
cd android && ./gradlew installDebug && cd .. # Install APK
adb logcat | grep zedra                       # View logs
```

### Developer Tools

**Pre-flight check** (verify environment before building):
```bash
./scripts/preflight-check.sh
```

**Background log monitoring** (continuous logcat with filtering):
```bash
./scripts/logcat-daemon.sh start  # Start background monitor
./scripts/logcat-daemon.sh tail   # View live logs
./scripts/logcat-daemon.sh stop   # Stop monitor
```

**Crash analysis** (automatically diagnose crashes):
```bash
./scripts/analyze-crash.sh
```

See `docs/DEBUGGING.md` for complete debugging guide.

### Prerequisites

- Android NDK r25c+
- Rust 1.75+ with aarch64-linux-android target
- Android SDK with API 31+
- Physical Android device (emulator not tested)
- Git submodules initialized:
  ```bash
  git submodule update --init --recursive
  ```
  - `vendor/zed` - Zed with GPUI Android platform (gpui, gpui_android, gpui_wgpu crates)

## Architecture

### High-Level Design

```
JNI Thread → Command Queue → Main Thread → GPUI → wgpu → Vulkan
```

**Key Components**:

1. **Command Queue Pattern** (`src/android/command_queue.rs`)
   - Decouples multi-threaded JNI from single-threaded GPUI
   - Thread-safe crossbeam channel
   - Main thread drains queue at 60 FPS via Choreographer

2. **JNI Bridge** (`src/android/jni.rs`)
   - Java ↔ Rust interface
   - Queues commands from any thread
   - Surface lifecycle callbacks

3. **Android App** (`src/android/app.rs`)
   - Main-thread-only GPUI application state
   - Processes commands from queue
   - Manages window and platform lifecycle

4. **Touch Handler** (`src/android/touch.rs`)
   - Tap detection (TAP_SLOP), scroll/drawer gesture disambiguation
   - Fling momentum scrolling with frame-rate independent friction
   - Delegates to GestureArena (`src/mgpui/gesture.rs`) for pan vs scroll

4. **GPUI Android Platform** (`vendor/zed/crates/gpui_android/`)
   - AndroidPlatform - Platform trait implementation
   - AndroidWindow - Window management, surface lifecycle, atlas sharing
   - AndroidDispatcher - Task queue integration

5. **wgpu Renderer** (`vendor/zed/crates/gpui_wgpu/`)
   - WgpuRenderer - GPU rendering via wgpu (Vulkan backend on Android)
   - WgpuAtlas - Texture atlas for glyph/sprite caching
   - WgpuContext - Device/adapter initialization
   - Dual-source blending shaders conditionally compiled (not available on all GPUs)

### Critical Design Decisions

1. **Threading Model**
   - GPUI requires single-threaded execution
   - JNI callbacks come from any thread
   - Solution: Command queue isolates threading complexity

2. **Pixel Handling**
   ```
   Android DP → GPUI Logical Pixels → Vulkan Physical Pixels
   Physical = Logical × Scale Factor (3.0 for high-DPI)
   ```
   - GPUI works in logical pixels
   - wgpu renderer needs physical pixels
   - Conversion at window/surface boundary in `window.rs:handle_surface_created()`

3. **Lazy WgpuContext**
   - GPU initialization deferred until first window opens
   - Faster app startup
   - Context reused across windows

5. **Shared Atlas Pattern**
   - Android window creates `WgpuAtlas` at construction (before surface exists)
   - GPUI uses this atlas for scene construction (rasterizing glyphs, etc.)
   - When surface arrives, renderer is created via `new_with_atlas()` sharing the same atlas
   - Critical: scene texture IDs must reference the same atlas the renderer reads from

4. **Surface Lifecycle**
   - Window (logical) persists across surface recreation
   - Renderer (physical) created/destroyed with surface
   - Matches Android's view lifecycle

## Project Structure

```
Cargo.toml                      # Workspace root (no package)
crates/
  ├── zedra-terminal/           # Terminal emulation (alacritty + GPUI rendering)
  │   └── src/
  │       ├── lib.rs            # TerminalState + types
  │       ├── element.rs        # GPUI Element for terminal grid
  │       ├── keys.rs           # Keystroke → escape sequence mapping
  │       └── view.rs           # TerminalView + GPUI Render
  ├── zedra-telemetry/           # Pure telemetry: typed Event enum + TelemetryBackend trait
  │   └── src/
  │       └── lib.rs            # Event enum, context structs, global dispatch, backend trait
  ├── zedra-rpc/                # irpc protocol types + QR pairing codec
  │   └── src/
  │       ├── lib.rs            # Re-exports
  │       ├── proto.rs          # irpc protocol enum (ZedraProto) + request/response types
  │       └── pairing.rs        # EndpointAddr encode/decode (postcard + base64-url)
  ├── zedra-session/            # Mobile client: iroh connection, RPC, auto-reconnect
  │   └── src/
  │       ├── lib.rs            # RemoteSession, terminal buffers, reconnect state
  │       └── signer.rs         # ClientSigner trait + FileClientSigner (Ed25519 app key)
  ├── zedra/                    # Android cdylib (final binary crate)
  │   ├── build.rs
  │   └── src/
  │       ├── lib.rs            # Module declarations + JNI exports
  │       ├── android/          # Android platform integration
  │       │   ├── mod.rs        # Module declarations + legacy JNI stubs
  │       │   ├── app.rs        # Main-thread GPUI app, surface/window lifecycle
  │       │   ├── jni.rs        # JNI bridge (Java ↔ Rust)
  │       │   ├── command_queue.rs # Thread-safe crossbeam command queue
  │       │   ├── bridge.rs     # AndroidBridge: PlatformBridge impl (density, keyboard)
  │       │   └── touch.rs      # TouchHandler: tap, scroll, drawer pan, fling momentum
  │       ├── mgpui/            # Mobile GPUI primitives
  │       │   ├── mod.rs        # Re-exports + drawer gesture globals
  │       │   ├── gesture.rs    # GestureArena: pan vs scroll disambiguation
  │       │   ├── drawer_host.rs # DrawerHost: slide-from-left overlay
  │       │   ├── stack_navigator.rs # StackNavigator: push/pop with header bar
  │       │   └── input.rs      # Keyboard input handling
  │       ├── editor/           # Code editor components
  │       │   ├── mod.rs        # Re-exports
  │       │   ├── code_editor.rs # GPUI view: UniformList + StyledText + cursor
  │       │   ├── git_sidebar.rs # Git status sidebar
  │       │   ├── git_diff_view.rs # Git diff viewer
  │       │   ├── syntax_highlighter.rs # Tree-sitter parsing + highlight queries
  │       │   ├── syntax_theme.rs # Capture name → HighlightStyle mapping
  │       │   └── text_buffer.rs # Text buffer with line indexing
  │       ├── app.rs            # ZedraApp root view: DrawerHost + screens + badges
  │       ├── app_drawer.rs     # AppDrawer: tabbed sidebar (files, git, terminal, session)
  │       ├── file_explorer.rs  # FileExplorer tree view
  │       ├── home_view.rs      # Home/connection screen
  │       ├── pending.rs        # PendingSlot<T>: generic async→main-thread channel
  │       ├── keyboard.rs       # Keyboard show/hide handler factory
  │       ├── transport_badge.rs # Connection status badge rendering
  │       ├── terminal_panel.rs # Terminal tab panel for app drawer
  │       ├── session_panel.rs  # Session info panel for app drawer
  │       ├── analytics.rs      # FirebaseBackend: bridges Firebase ↔ zedra-telemetry
  │       ├── platform_bridge.rs # PlatformBridge trait + global accessor
  │       └── theme.rs          # Color constants and theme helpers
  └── zedra-host/               # Desktop host daemon
      └── src/
          ├── main.rs           # CLI: start (daemon) + client + stop
          ├── client.rs         # `zedra client` — local RTT test client (PKI auth + ping loop)
          ├── rpc_daemon.rs     # irpc RPC dispatch, PKI auth phase, TermAttach bidi streaming
          ├── session_registry.rs # PKI sessions: ACLs, pairing slots, persistence, active client
          ├── iroh_listener.rs  # iroh Endpoint creation + accept loop
          ├── identity.rs       # Persistent Ed25519 host identity (~/.config/zedra/)
          ├── qr.rs             # QR code generation (terminal + JSON output)
          ├── analytics.rs      # GA4 Measurement Protocol (host-specific events + sync panic hook)
          ├── telemetry.rs      # HostBackend: bridges GA4 Analytics ↔ zedra-telemetry
          ├── fs.rs             # Filesystem RPC handlers (list, read, write, stat, remove, mkdir)
          ├── git.rs            # Git RPC handlers (status, diff, log, commit, branches, checkout)
          └── pty.rs            # PTY management (spawn, resize, I/O streaming)

android/app/src/main/java/dev/zedra/app/
  ├── MainActivity.java         # Activity + frame loop
  ├── GpuiSurfaceView.java     # Surface management + IME + touch/scroll detection
  └── QRScannerActivity.java   # QR code scanner for pairing

vendor/zed/crates/gpui_android/src/android/
  ├── platform.rs               # AndroidPlatform
  ├── window.rs                 # AndroidWindow (CRITICAL: surface sizing + atlas sharing)
  ├── text_system.rs            # CosmicTextSystem for Android
  ├── dispatcher.rs             # Task queue
  └── keyboard.rs               # Input (stub)

vendor/zed/crates/gpui_wgpu/src/
  ├── gpui_wgpu.rs              # Crate root, re-exports
  ├── wgpu_renderer.rs          # WgpuRenderer: pipelines, draw, new_with_atlas()
  ├── wgpu_atlas.rs             # WgpuAtlas: texture atlas allocation + uploads
  ├── wgpu_context.rs           # WgpuContext: device/adapter init, feature detection
  ├── shaders.wgsl              # Base shaders (all primitives except subpixel sprites)
  └── shaders_subpixel.wgsl     # Subpixel sprite shaders (requires dual-source blending)

packages/relay-worker/              # [DEPRECATED] Experimental Cloudflare Worker relay — do not use
                                    # Production relay now uses official iroh-relay binary (see deploy/relay/)

docs/
  ├── ARCHITECTURE.md           # Design decisions
  ├── DEBUGGING.md              # Debug workflow and tools
  ├── RELAY.md                  # Relay wire protocol + DO topology docs
  └── IOS_WORKFLOW.md           # iOS build pipeline, FFI patterns, debugging, pitfalls
```

### Dependency Graph

```
zedra-telemetry (pure: typed Event enum, TelemetryBackend trait, global dispatch)
    ↑
zedra-rpc (irpc protocol types, QR pairing codec)
    ↑
zedra-session (iroh connection, RPC, auto-reconnect — emits telemetry events)
    ↑
zedra-terminal (terminal emulation, sends input via zedra-session)
    ↑
zedra (app cdylib — registers FirebaseBackend, emits app-level events)

zedra-host (iroh listener, irpc RPC daemon — registers GA4 HostBackend)
    ↑
zedra-rpc + zedra-telemetry
```

## What Works

- Core rendering: Colored shapes, borders, shadows
- Text rendering: CosmicTextSystem with Android fonts
- 60 FPS via Choreographer
- wgpu rendering via Vulkan backend
- Proper surface lifecycle management
- Thread-safe command queue
- Correct pixel density handling
- Optimized font loading (82% faster startup)
- Touch input: tap for clicks, drag for scrolling (touch-to-scroll conversion)
- Soft keyboard: programmatic show/hide via `requestKeyboard()`/`dismissKeyboard()`
- Navigation: TabNavigator (bottom tabs), StackNavigator (push/pop with back button)
- DrawerHost: slide-from-left file explorer drawer
- Code editor: syntax-highlighted Rust code with tree-sitter, cursor, and virtual scrolling
- File preview grid: card-based file browser that opens editor views
- Remote terminal: connection form with RPC session (zedra-session + zedra-terminal)
- iroh transport: QUIC/TLS 1.3, direct P2P (RelayMode::Disabled — LAN/routable IPs only)
- irpc typed RPC: postcard binary serialization, bidi streaming for terminal I/O
- QR pairing: compact postcard+base64-url EndpointAddr encoding (~50 bytes)
- Connection monitoring: path watcher tracks direct vs relay, RTT, bytes sent/recv (2s polling fallback)
- Session persistence: server-side SessionRegistry with terminal PTY survival across reconnects
- Client-side auto-reconnect: exponential backoff (1s–30s), persistent terminal output buffers survive reconnect
- Terminal backlog replay: missed output replayed per-terminal via TermAttach bidi stream
- Reconnecting UI badge: transport indicator shows "Reconnecting... (N)" with red dot during reconnect
- PKI authentication: QR encodes `ZedraPairingTicket` (endpoint_id + handshake_key + session_id); first pairing via HMAC-SHA256; reconnects via Ed25519 challenge-response; `auth_token` fully removed
- Per-session ACLs: each session tracks authorized client pubkeys; exclusive single-client ownership (one active client per session)
- Session state persistence: `sessions.json` survives daemon restarts; authorized pubkeys restored on reload
- Ping/Pong RTT: `Ping { timestamp_ms }` replaces `Heartbeat`; host echoes timestamp for RTT measurement
- PTY output coalescing: buffered chunks merged before relay send, separate output task decouples slow sends from input
- `zedra client` CLI: connects to running daemon via pre-authorized key, measures relay vs P2P RTT with statistics
- `HostUnreachable` state: after 10 failed reconnect attempts; shown in home screen and session panel

## Known Limitations (Technical Debt)

See `docs/TECHNICAL_DEBT.md` for detailed solutions.

1. **Keyboard Integration** (Medium Priority)
   - Soft keyboard can be shown/hidden programmatically but no GPUI text input fields trigger it yet
   - `GpuiSurfaceView.requestKeyboard()` / `dismissKeyboard()` are wired but not called from Rust
   - Solution: Add JNI calls from Rust when GPUI text inputs gain focus

2. **Touch Gesture Refinement** (Medium Priority)
   - Tap vs scroll uses GestureArena with TAP_SLOP = 4px logical
   - Fling/momentum scrolling implemented with frame-rate independent friction
   - No pinch-to-zoom or multi-touch gestures yet
   - Solution: Add multi-pointer tracking in `TouchHandler`

3. **Terminal Session Persistence** (High Priority)
   - PTYs survive disconnect but fresh clients can't discover or resume them
   - No screen state restoration (blank/garbled terminal after long disconnect)
   - No on-disk credential storage (app restart = new session)
   - See `docs/TERMINAL_PERSISTENCE.md` for full analysis and implementation plan
   - Solution: Server-side `vt100` parser for screen capture + terminal discovery flow

4. **Sample Data Only** (Low Priority)
   - Editor shows 4 hardcoded Rust sample files, file explorer shows demo tree
   - No filesystem access on Android yet
   - Solution: Use Android Storage Access Framework or SSH file browsing

## Critical Code Locations

### Surface Sizing Fix (THE breakthrough that made rendering work)

**File**: `vendor/zed/crates/gpui_android/src/android/window.rs:handle_surface_created()`

```rust
pub fn handle_surface_created(&mut self, native_window: NativeWindow, context: &WgpuContext) {
    let size = Size {
        width: DevicePixels((f32::from(self.bounds.size.width) * self.scale) as i32),
        height: DevicePixels((f32::from(self.bounds.size.height) * self.scale) as i32),
    };
    let config = WgpuSurfaceConfig { size, transparent: false };
    // Pass the window's atlas so renderer shares it with scene construction
    let renderer = WgpuRenderer::new_with_atlas(context, &raw_window, config, Some(self.atlas.clone()))?;
}
```

**Why Critical**: This is where logical pixels are converted to physical pixels. Getting this wrong causes black screen or incorrectly sized UI. The `new_with_atlas()` call is also critical — using `new()` would create a separate atlas, causing index-out-of-bounds crashes during draw.

### Window Creation with Actual Dimensions

**File**: `crates/zedra/src/android/app.rs:handle_surface_created()`

Window dimensions are now derived from the native surface size and display density (via JNI `get_density()`), not hardcoded. The surface `width`/`height` come from `surfaceChanged` and are divided by the scale factor to get logical pixels.

**Why Critical**: Using DEFAULT_WINDOW_SIZE here caused the original black screen issue. Must use actual screen dimensions.

### Touch-to-Scroll Conversion

**File**: `crates/zedra/src/android/touch.rs:handle_touch()`

GPUI scrollable elements (`uniform_list`, `overflow_y_scroll`) respond to `ScrollWheel` events, not mouse drags. The `TouchHandler` uses a `GestureArena` to disambiguate drawer pan vs content scroll:
- `ACTION_DOWN` → Cancel fling, record origin for tap detection
- `ACTION_MOVE` → Feed gesture arena; once winner decided, dispatch DrawerPan or ScrollWheel
- `ACTION_UP` → If no drag (within TAP_SLOP=4px), dispatch tap; otherwise end gesture
- Fling: momentum scrolling with frame-rate independent friction (0.95^(dt*60))

### Tree-sitter Highlight Deduplication

**File**: `crates/zedra-editor/src/editor_view.rs:line_highlights()`

Tree-sitter can return overlapping capture ranges. GPUI's `compute_runs()` requires non-overlapping, sorted ranges. The `line_highlights()` method sorts by start position and deduplicates overlapping spans before passing to `StyledText::with_default_highlights()`.

## Common Issues and Solutions

### Black Screen
- Check surface dimensions are physical pixels (1080x2400, not 360x800)
- Verify WgpuRenderer created successfully (check logcat for errors)
- **Take screenshot** to confirm: `adb shell screencap -p /sdcard/test.png && adb pull /sdcard/test.png /tmp/test.png`
- Check logs for rendering errors or panics

### Build Errors
- Ensure NDK path is set: `export ANDROID_NDK_ROOT=...`
- Check Rust target installed: `rustup target add aarch64-linux-android`
- Verify vendor/zed submodule initialized

### Crash on Launch
- Check Vulkan 1.1+ support on device: `adb shell getprop ro.hardware.vulkan`
- Verify JNI methods match Java signatures
- Check logcat for panic messages

### Visual Verification (Screenshot Debugging)
**Essential for UI verification** - logcat shows frames, screenshots prove rendering:
```bash
# After deployment
adb shell screencap -p /sdcard/test.png
adb pull /sdcard/test.png /tmp/test.png
# Claude Code can display and analyze the screenshot
```
See `docs/DEBUGGING.md` for complete workflow.

## Development Workflow

1. **Make Changes**: Edit Rust or Java code
2. **Build**: `./scripts/build-android.sh`
3. **Install**: `cd android && ./gradlew installDebug`
4. **Test**: Launch app and check `adb logcat | grep zedra`
5. **Verify**: Look for frame logs and rendering confirmation

## Pre-Commit Checks (Required)

Run these before every commit. CI enforces them on push/PR.

### Rust

```bash
# Format check (must pass before committing)
cargo fmt

# Check host crates compile cleanly (no Android NDK needed)
cargo check -p zedra-rpc -p zedra-session -p zedra-terminal -p zedra-host
```

### JS/TS

```bash
# Format + lint all packages (auto-fix)
bun run format

# Check only (what CI runs)
bun run check
```

## Roadmap

- **Phase 1**: Foundation ✅ Complete
- **Phase 2**: Text Rendering ✅ Complete
- **Phase 3**: Dynamic Configuration ✅ Complete (DisplayMetrics via JNI)
- **Phase 4**: Input Integration ✅ Complete (touch→scroll, keyboard, tap detection)
- **Phase 5**: Navigation + Editor ✅ Complete (tabs, stacks, drawer, syntax editor)
- **Phase 6**: Transport ✅ Complete (iroh QUIC direct P2P, irpc typed RPC, session persistence, health monitoring)
- **Phase 6.5**: Session Reconnect ✅ Complete (auto-reconnect with exponential backoff, persistent terminal buffers, backlog replay, reconnecting UI badge)
- **Phase 6.7**: PKI Authentication ✅ Complete (ZedraPairingTicket QR, HMAC registration, Ed25519 challenge-response, per-session ACLs, session persistence, `zedra client` CLI)
- **Phase 7**: Terminal Persistence - Next (server-side vt100 screen capture, fresh client terminal discovery, on-disk credential storage)
- **Phase 8**: Production Hardening (momentum scrolling, real file access, multi-touch, E2E encryption)

## Performance Characteristics

**Current Performance (Phase 2)**:
- Platform init: ~51ms (82% faster after font optimization)
- CPU per frame: <5ms (plenty of headroom for 16ms target)
- GPU per frame: <4ms
- Memory: ~40-50 MB for single-window app
- Frame rate: Stable 60 FPS

## Important Notes for Future Development

1. **Always test on physical device** - Emulator Vulkan support is inconsistent
2. **Watch for threading issues** - All GPUI code MUST run on main thread
3. **Pixel conversions** - Be explicit about logical vs physical pixels
4. **Surface lifecycle** - Window persists, renderer is created/destroyed
5. **wgpu/Vulkan compatibility** - Not all GPUs support dual-source blending; subpixel shaders are conditionally compiled
6. **Atlas sharing** - The Android window's atlas MUST be passed to `WgpuRenderer::new_with_atlas()`, never use `new()` on Android

## Documentation

- Overview and quick start: `docs/README.md`
- Architecture and design patterns: `docs/ARCHITECTURE.md`
- Known issues with solutions: `docs/TECHNICAL_DEBT.md`
- Debug workflow and tools: `docs/DEBUGGING.md`
- Relay wire protocol + DO topology: `docs/RELAY.md`
- Protocol/RPC conventions + compatibility: `docs/PROTOCOL_SPECS.md`
- Terminal persistence design: `docs/TERMINAL_PERSISTENCE.md`
- **iOS development workflow**: `docs/IOS_WORKFLOW.md` — build pipeline, commands, FFI patterns, debugging, pitfalls

## Performance Testing

Run the performance testing script after deployment to measure frame times, memory, and descriptor pool health:

```bash
./scripts/perf-test.sh
```

This captures ~10 seconds of logcat, then reports:
- Frame timing statistics (min/max/avg/p95)
- Memory usage (RSS from `dumpsys meminfo`)
- Any warnings or errors

## Achievement

First successful port of GPUI to Android with:
- wgpu rendering via Vulkan backend with conditional dual-source blending
- CosmicTextSystem with Android font support
- Clean command queue architecture for threading
- Proper surface lifecycle management with shared atlas pattern
- Optimized font loading (82% faster startup)
- 60 FPS rendering
- Full touch input (tap + scroll) with IME keyboard support
- Mobile navigation (tabs, stacks, drawer)
- Syntax-highlighted code editor with tree-sitter
- iroh transport: QUIC/TLS 1.3 direct P2P (RelayMode::Disabled)
- Connection path monitoring with RTT and byte stats
- irpc typed RPC: postcard binary serialization, bidi streaming for terminal I/O (no JSON, no base64)
- Auto-reconnect: exponential backoff, persistent output buffers, per-terminal backlog replay

---

**Last Updated**: 2026-02-22
