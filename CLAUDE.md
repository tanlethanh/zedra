# Zedra - GPUI on Android

## Project Overview

Zedra is a port of Zed's GPUI UI framework to Android using wgpu for GPU rendering. This is the first successful port of GPUI to a mobile platform.

**Current Status**: Interactive UI with navigation, editor, and touch input working at 60 FPS on Android via wgpu/Vulkan

**Test Device**: Mali-G68 MC4 (Vulkan 1.1.0, 1080x2400 @ 2.75x DPI)

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
  ├── zedra-rpc/                # irpc protocol types + QR pairing codec
  │   └── src/
  │       ├── lib.rs            # Re-exports, DEFAULT_RELAY_URL
  │       ├── proto.rs          # irpc protocol enum (ZedraProto) + request/response types
  │       └── pairing.rs        # EndpointAddr encode/decode (postcard + base64-url)
  ├── zedra-session/            # Mobile client: iroh connection, RPC, auto-reconnect
  │   └── src/
  │       └── lib.rs            # RemoteSession, terminal buffers, reconnect state
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
  │       ├── platform_bridge.rs # PlatformBridge trait + global accessor
  │       └── theme.rs          # Color constants and theme helpers
  └── zedra-host/               # Desktop host daemon
      └── src/
          ├── main.rs           # CLI: start (daemon) + qr (pairing)
          ├── rpc_daemon.rs     # irpc RPC dispatch, TermAttach bidi streaming
          ├── session_registry.rs # Persistent sessions with terminal ownership
          ├── iroh_listener.rs  # iroh Endpoint creation + accept loop
          ├── identity.rs       # Persistent Ed25519 host identity (~/.config/zedra-host/)
          ├── qr.rs             # QR code generation (terminal + JSON output)
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
  ├── ARCHITECTURE.md           # Design decisions
  ├── DEBUGGING.md              # Debug workflow and tools
  └── RELAY.md                  # Relay wire protocol + DO topology docs
```

### Dependency Graph

```
zedra-rpc (irpc protocol types, QR pairing codec)
    ↑
zedra-session (RemoteSession, iroh connection, terminal buffers, auto-reconnect)
    ↑
zedra-terminal (terminal emulation, sends input via zedra-session)
    ↑
zedra (Android cdylib, GPUI app, touch handling, editor, navigation)

zedra-host (iroh listener, irpc RPC daemon, session registry, host identity)
    ↑
zedra-rpc
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
- iroh transport: QUIC/TLS 1.3 with automatic NAT traversal and relay fallback
- irpc typed RPC: postcard binary serialization, bidi streaming for terminal I/O
- QR pairing: compact postcard+base64-url EndpointAddr encoding (~50 bytes)
- Cross-network connectivity: relay.zedra.dev (Cloudflare Worker, iroh-compatible relay protocol)
- Automatic path selection: iroh handles direct → hole-punch → relay internally
- Path upgrade: relay → direct P2P when hole-punch succeeds
- Session persistence: server-side SessionRegistry with terminal PTY survival across reconnects
- Client-side auto-reconnect: exponential backoff (1s–30s), persistent terminal output buffers survive reconnect
- Terminal backlog replay: missed output replayed per-terminal via TermAttach bidi stream
- Reconnecting UI badge: transport indicator shows "Reconnecting... (N)" with red dot during reconnect
- Connection monitoring: path watcher tracks direct vs relay, RTT, bytes sent/recv

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

3. **Sample Data Only** (Low Priority)
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
5. **wgpu/Vulkan compatibility** - Not all GPUs support dual-source blending; subpixel shaders are conditionally compiled
6. **Atlas sharing** - The Android window's atlas MUST be passed to `WgpuRenderer::new_with_atlas()`, never use `new()` on Android

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
- iroh transport: QUIC/TLS 1.3 with automatic path selection (direct, hole-punch, relay)
- Cross-network relay: iroh-compatible CF Worker relay at relay.zedra.dev
- Connection path monitoring with automatic relay → direct P2P upgrade
- irpc typed RPC: postcard binary serialization, bidi streaming for terminal I/O (no JSON, no base64)
- Auto-reconnect: exponential backoff, persistent output buffers, per-terminal backlog replay

---

**Last Updated**: 2026-02-22
