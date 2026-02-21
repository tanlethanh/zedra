# Zedra - GPUI on Android

## Project Overview

Zedra is a port of Zed's GPUI UI framework to Android using Blade graphics with Vulkan 1.1 support. This is the first successful port of GPUI to a mobile platform.

**Current Status**: Interactive UI with navigation, editor, and touch input working at 60 FPS on Android with Vulkan 1.1

**Test Device**: Mali-G68 MC4 (Vulkan 1.1.0, 1080x2400 @ 3x DPI)

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
  - `vendor/zed` - Zed with GPUI Android platform
  - `vendor/blade` - Blade graphics with Vulkan 1.1 support

## Architecture

### High-Level Design

```
JNI Thread → Command Queue → Main Thread → GPUI → Blade → Vulkan
```

**Key Components**:

1. **Command Queue Pattern** (`src/android_command_queue.rs`)
   - Decouples multi-threaded JNI from single-threaded GPUI
   - Thread-safe crossbeam channel
   - Main thread drains queue at 60 FPS via Choreographer

2. **JNI Bridge** (`src/android_jni.rs`)
   - Java ↔ Rust interface
   - Queues commands from any thread
   - Surface lifecycle callbacks

3. **Android App** (`src/android_app.rs`)
   - Main-thread-only GPUI application state
   - Processes commands from queue
   - Manages window and platform lifecycle

4. **GPUI Platform** (`vendor/zed/crates/gpui/src/platform/android/`)
   - AndroidPlatform - Platform trait implementation
   - AndroidWindow - Window management and rendering
   - AndroidDispatcher - Task queue integration

5. **Blade Integration** (`vendor/zed/crates/gpui/src/platform/blade/`)
   - Vulkan 1.1 traditional renderpass support
   - 90% device compatibility (vs 30% with Vulkan 1.3 dynamic rendering)

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
   - Blade renderer needs physical pixels
   - Conversion at window/surface boundary in `window.rs:handle_surface_created()`

3. **Lazy BladeContext**
   - GPU initialization deferred until first window opens
   - Faster app startup
   - Context reused across windows

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
  ├── zedra-editor/             # Code editor with syntax highlighting
  │   └── src/
  │       ├── lib.rs            # Re-exports
  │       ├── buffer.rs         # Text buffer with line indexing
  │       ├── highlighter.rs    # Tree-sitter parsing + highlight queries
  │       ├── theme.rs          # Capture name → HighlightStyle mapping
  │       └── editor_view.rs    # GPUI view: UniformList + StyledText + cursor
  ├── zedra-nav/                # Mobile navigation primitives (gpui only)
  │   └── src/
  │       ├── lib.rs            # Re-exports + NavigationEvent types
  │       ├── stack.rs          # StackNavigator: push/pop with header bar
  │       ├── tab.rs            # TabNavigator: bottom tab bar with lazy views
  │       ├── modal.rs          # ModalHost: deferred overlay with backdrop
  │       └── drawer.rs         # DrawerHost: slide-from-left drawer overlay
  ├── zedra-rpc/                # RPC protocol, Transport trait, framing
  │   └── src/
  │       ├── transport.rs      # Transport trait + TcpTransport + spawn_from_channels
  │       └── protocol.rs       # RPC methods incl. session/resume_or_create, terminal/list
  ├── zedra-relay/              # Relay client + RelayTransport impl
  │   └── src/
  │       ├── lib.rs            # Re-exports, DEFAULT_RELAY_URL
  │       ├── client.rs         # HTTP client for relay API (reqwest + rustls)
  │       ├── transport.rs      # RelayTransport: Transport trait via HTTP polling
  │       └── types.rs          # Shared types (RoomCode, RelayMessage, etc.)
  ├── zedra-transport/          # TransportManager, discovery chain, providers, pairing
  │   └── src/
  │       ├── lib.rs            # PeerInfo struct, re-exports
  │       ├── pairing.rs        # QR code pairing protocol (v1 + v2 multi-transport)
  │       ├── discovery.rs      # Concurrent provider racing (JoinSet)
  │       ├── manager.rs        # Transport lifecycle, health, switching
  │       └── providers/
  │           ├── mod.rs        # TransportProvider trait
  │           ├── lan.rs        # LAN TCP provider (priority 0)
  │           ├── tailscale.rs  # Tailscale provider (priority 1)
  │           └── relay.rs      # Relay provider (priority 2, fallback)
  ├── zedra/                    # Android cdylib (final binary crate)
  │   ├── build.rs
  │   └── src/
  │       ├── lib.rs            # JNI exports + module declarations
  │       ├── android_app.rs    # Main thread GPUI app + touch/scroll/key handling
  │       ├── android_command_queue.rs # Thread-safe queue
  │       ├── android_jni.rs    # JNI bridge
  │       ├── zedra_app.rs      # DrawerHost + TabNavigator + transport/reconnect badge
  │       ├── file_explorer.rs  # FileExplorer tree view (demo data)
  │       └── file_preview_list.rs # Preview card grid for code samples
  └── zedra-host/               # Desktop SSH server daemon
      └── src/
          ├── main.rs           # CLI: listen + relay subcommands
          ├── rpc_daemon.rs     # handle_transport_connection, terminal/list handler, clear_notif_senders on disconnect
          ├── session_registry.rs # Persistent sessions with terminal ownership
          ├── relay_bridge.rs   # Relay mode: room creation, QR, bridge to RPC
          └── qr.rs             # QR generation (v1 LAN + v2 relay)

android/app/src/main/java/dev/zedra/app/
  ├── MainActivity.java         # Activity + frame loop
  ├── GpuiSurfaceView.java     # Surface management + IME + touch/scroll detection
  └── QRScannerActivity.java   # QR code scanner for pairing

vendor/zed/crates/gpui/src/platform/android/
  ├── platform.rs               # AndroidPlatform
  ├── window.rs                 # AndroidWindow (CRITICAL: surface sizing)
  ├── text_system.rs            # CosmicTextSystem for Android
  ├── dispatcher.rs             # Task queue
  └── keyboard.rs               # Input (stub)

vendor/zed/crates/gpui/src/platform/blade/
  └── blade_renderer.rs         # Vulkan 1.1 compatibility

vendor/blade/ (git submodule - vulkan-1.1-compat branch)
  └── blade-graphics/src/vulkan/
      ├── init.rs               # Vulkan 1.1 extension detection
      ├── surface.rs            # Traditional renderpass creation
      ├── command.rs            # Compatible rendering commands
      └── pipeline.rs           # Pipeline for traditional renderpass

packages/relay-worker/              # Cloudflare Worker relay server (relay.zedra.dev)
  ├── wrangler.toml             # CF Worker config + KV + DO bindings
  └── src/
      ├── index.ts              # Router: /ping, /generate_204, /relay (WS upgrade)
      ├── relay-endpoint.ts     # RelayEndpoint Durable Object (per-endpoint WS relay)
      ├── frame-codec.ts        # iroh relay wire protocol (13 frame types)
      ├── crypto.ts             # BLAKE3 KDF + Ed25519 handshake verification
      ├── types.ts              # TypeScript types + CF bindings
      └── utils.ts              # JSON responses, error helpers

docs/
  ├── README.md                 # Project overview
  ├── ARCHITECTURE.md           # Design decisions
  ├── TECHNICAL_DEBT.md         # Known issues with solutions
  ├── DEBUGGING.md              # Debug workflow and tools
  └── RELAY.md                  # Relay wire protocol + DO topology docs
```

### Dependency Graph

```
zedra-rpc (Transport trait, RpcClient, framing)
    ↑
zedra-relay (RelayTransport impl, HTTP client for CF Worker)
    ↑
zedra-transport (TransportManager, discovery chain, providers, pairing)
    ↑
zedra-session (RemoteSession, uses TransportManager or direct TCP)
    ↑
zedra-terminal (terminal emulation, sends input via zedra-session)
    ↑
zedra (Android cdylib, depends on all crates)
    ↑
zedra-editor (standalone: gpui + tree-sitter)
zedra-nav (standalone: gpui only)

zedra-host (SessionRegistry, relay bridge, uses Transport trait + zedra-relay)
    ↑
zedra-rpc, zedra-relay
```

## What Works

- Core rendering: Colored shapes, borders, shadows
- Text rendering: CosmicTextSystem with Android fonts
- 60 FPS via Choreographer
- Vulkan 1.1 compatibility (traditional renderpass)
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
- iroh transport: QUIC/TLS 1.3 with automatic NAT traversal and relay fallback
- Cross-network connectivity: relay.zedra.dev (Cloudflare Worker, iroh-compatible relay protocol)
- Automatic path selection: iroh handles direct → hole-punch → relay internally
- Path upgrade: relay → direct P2P when hole-punch succeeds
- Session persistence: server-side SessionRegistry with terminal PTY survival across reconnects
- Client-side auto-reconnect: exponential backoff (1s–30s), persistent terminal output buffers survive reconnect
- Notification backlog replay: missed terminal output replayed on session resume via LAST_NOTIF_SEQ tracking
- Reconnecting UI badge: transport indicator shows "Reconnecting... (N)" with red dot during reconnect
- Connection monitoring: path watcher tracks direct vs relay, RTT, bytes sent/recv

## Known Limitations (Technical Debt)

See `docs/TECHNICAL_DEBT.md` for detailed solutions.

1. **Keyboard Integration** (Medium Priority)
   - Soft keyboard can be shown/hidden programmatically but no GPUI text input fields trigger it yet
   - `GpuiSurfaceView.requestKeyboard()` / `dismissKeyboard()` are wired but not called from Rust
   - Solution: Add JNI calls from Rust when GPUI text inputs gain focus

2. **Touch Gesture Refinement** (Medium Priority)
   - Tap vs scroll detection uses a simple distance threshold (TAP_SLOP = 20px)
   - No fling/momentum scrolling, pinch-to-zoom, or multi-touch gestures
   - Solution: Implement velocity tracking and `TouchPhase::Ended` fling events

3. **Sample Data Only** (Low Priority)
   - Editor shows 4 hardcoded Rust sample files, file explorer shows demo tree
   - No filesystem access on Android yet
   - Solution: Use Android Storage Access Framework or SSH file browsing

## Critical Code Locations

### Surface Sizing Fix (THE breakthrough that made rendering work)

**File**: `vendor/zed/crates/gpui/src/platform/android/window.rs:206-215`

```rust
pub fn handle_surface_created(&mut self, native_window: NativeWindow, context: &BladeContext) {
    // Convert logical pixels to physical pixels
    let size = gpu::Extent {
        width: (self.bounds.size.width.0 * self.scale) as u32,   // 360 * 3 = 1080
        height: (self.bounds.size.height.0 * self.scale) as u32,  // 800 * 3 = 2400
        depth: 1,
    };

    let config = BladeSurfaceConfig {
        size,
        transparent: matches!(self.background_appearance,
            WindowBackgroundAppearance::Transparent |
            WindowBackgroundAppearance::Blurred
        ),
    };

    let renderer = BladeRenderer::new(context, &raw_window, config)?;
}
```

**Why Critical**: This is where logical pixels are converted to physical pixels. Getting this wrong causes black screen or incorrectly sized UI.

### Window Creation with Actual Dimensions

**File**: `crates/zedra/src/android_app.rs:handle_surface_created()`

Window dimensions are now derived from the native surface size and display density (via JNI `get_density()`), not hardcoded. The surface `width`/`height` come from `surfaceChanged` and are divided by the scale factor to get logical pixels.

**Why Critical**: Using DEFAULT_WINDOW_SIZE here caused the original black screen issue. Must use actual screen dimensions.

### Touch-to-Scroll Conversion

**File**: `crates/zedra/src/android_app.rs:handle_touch()`

GPUI scrollable elements (`uniform_list`, `overflow_y_scroll`) respond to `ScrollWheel` events, not mouse drags. The touch handler converts:
- `ACTION_DOWN` → `MouseDown` (records position for delta tracking)
- `ACTION_MOVE` → `ScrollWheel` with `ScrollDelta::Pixels` (delta from last position)
- `ACTION_UP` → `MouseUp` (clears tracking)

### Descriptor Pool Fix

**File**: `vendor/blade/blade-graphics/src/vulkan/descriptor.rs`

Blade's descriptor pools grow exponentially (`16^(growth_iter+1)` sets). The `growth_iter` counter is now reset to 0 in `reset_descriptor_pool()` to prevent unbounded growth across frames, and pool size is capped at 65536 sets.

### Tree-sitter Highlight Deduplication

**File**: `crates/zedra-editor/src/editor_view.rs:line_highlights()`

Tree-sitter can return overlapping capture ranges. GPUI's `compute_runs()` requires non-overlapping, sorted ranges. The `line_highlights()` method sorts by start position and deduplicates overlapping spans before passing to `StyledText::with_default_highlights()`.

## Common Issues and Solutions

### Black Screen
- Check surface dimensions are physical pixels (1080x2400, not 360x800)
- Verify BladeRenderer created successfully
- **Take screenshot** to confirm: `adb shell screencap -p /sdcard/test.png && adb pull /sdcard/test.png /tmp/test.png`
- Check logs for "BladeRenderer::draw() called with N quads"

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

## Roadmap

- **Phase 1**: Foundation ✅ Complete
- **Phase 2**: Text Rendering ✅ Complete
- **Phase 3**: Dynamic Configuration ✅ Complete (DisplayMetrics via JNI)
- **Phase 4**: Input Integration ✅ Complete (touch→scroll, keyboard, tap detection)
- **Phase 5**: Navigation + Editor ✅ Complete (tabs, stacks, drawer, syntax editor)
- **Phase 6**: Transport + Relay ✅ Complete (transport abstraction, CF Worker relay, discovery chain, health monitoring, session persistence)
- **Phase 6.5**: Session Reconnect ✅ Complete (auto-reconnect with exponential backoff, persistent terminal buffers, backlog replay, reconnecting UI badge)
- **Phase 7**: Production Hardening - Next (momentum scrolling, real file access, multi-touch, E2E encryption)

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
5. **Vulkan 1.1 compatibility** - Don't use dynamic rendering features

## Documentation

- Overview and quick start: `docs/README.md`
- Architecture and design patterns: `docs/ARCHITECTURE.md`
- Known issues with solutions: `docs/TECHNICAL_DEBT.md`
- Debug workflow and tools: `docs/DEBUGGING.md`
- Relay wire protocol + DO topology: `docs/RELAY.md`

## Performance Testing

Run the performance testing script after deployment to measure frame times, memory, and descriptor pool health:

```bash
./scripts/perf-test.sh
```

This captures ~10 seconds of logcat, then reports:
- Frame timing statistics (min/max/avg/p95)
- Memory usage (RSS from `dumpsys meminfo`)
- Descriptor pool allocation counts
- Any warnings or errors

## Achievement

First successful port of GPUI to Android with:
- Vulkan 1.1 traditional renderpass support (90% device compatibility)
- CosmicTextSystem with Android font support
- Clean command queue architecture for threading
- Proper surface lifecycle management
- Optimized font loading (82% faster startup)
- 60 FPS rendering
- Full touch input (tap + scroll) with IME keyboard support
- Mobile navigation (tabs, stacks, drawer)
- Syntax-highlighted code editor with tree-sitter
- iroh transport: QUIC/TLS 1.3 with automatic path selection (direct, hole-punch, relay)
- Cross-network relay: iroh-compatible CF Worker relay at relay.zedra.dev
- Connection path monitoring with automatic relay → direct P2P upgrade
- Auto-reconnect: exponential backoff, persistent output buffers, notification backlog replay

---

**Last Updated**: 2026-02-21
