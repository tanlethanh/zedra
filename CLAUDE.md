# Zedra - GPUI on Android

## Project Overview

Zedra is a port of Zed's GPUI UI framework to Android using Blade graphics with Vulkan 1.1 support. This is the first successful port of GPUI to a mobile platform.

**Current Status**: Phase 2 Complete - Core rendering + text rendering working at 60 FPS on Android with Vulkan 1.1

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
src/
  ├── android_jni.rs           # JNI bridge
  ├── android_command_queue.rs # Thread-safe queue
  ├── android_app.rs           # Main thread GPUI app
  └── zedra_app.rs             # Demo UI with text rendering

android/app/src/main/java/dev/zedra/app/
  ├── MainActivity.java        # Activity + frame loop
  └── GpuiSurfaceView.java    # Surface management

vendor/zed/crates/gpui/src/platform/android/
  ├── platform.rs              # AndroidPlatform
  ├── window.rs                # AndroidWindow (CRITICAL: surface sizing)
  ├── text_system.rs           # CosmicTextSystem for Android
  ├── dispatcher.rs            # Task queue
  └── keyboard.rs              # Input (stub)

vendor/zed/crates/gpui/src/platform/blade/
  └── blade_renderer.rs        # Vulkan 1.1 compatibility

vendor/blade/ (git submodule - vulkan-1.1-compat branch)
  └── blade-graphics/src/vulkan/
      ├── init.rs              # Vulkan 1.1 extension detection
      ├── surface.rs           # Traditional renderpass creation
      ├── command.rs           # Compatible rendering commands
      └── pipeline.rs          # Pipeline for traditional renderpass

docs/
  ├── README.md                # Project overview
  ├── ARCHITECTURE.md          # Design decisions
  ├── TECHNICAL_DEBT.md        # Known issues with solutions
  └── DEBUGGING.md             # Debug workflow and tools
```

## What Works (Phase 2)

- Core rendering: Colored shapes, borders, shadows
- Text rendering: CosmicTextSystem with Android fonts
- 60 FPS via Choreographer
- Vulkan 1.1 compatibility (traditional renderpass)
- Proper surface lifecycle management
- Thread-safe command queue
- Correct pixel density handling
- Optimized font loading (82% faster startup)

## Known Limitations (Technical Debt)

See `docs/TECHNICAL_DEBT.md` for detailed solutions.

1. **Hardcoded Dimensions** (High Priority - Phase 3)
   - Only works on 1080x2400 @ 3x DPI
   - Location: `src/android_app.rs:151-157`
   - Solution: Get DisplayMetrics via JNI

2. **Input Not Forwarded** (High Priority - Phase 4)
   - Touch/keyboard captured but not sent to GPUI
   - Location: `src/android_app.rs:247, 262`
   - Solution: Convert to PlatformInput and dispatch to window

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

**File**: `src/android_app.rs:151-167`

```rust
// Hardcoded for MVP - TODO: get from DisplayMetrics (Phase 3)
let screen_width_px = 1080.0;
let screen_height_px = 2400.0;
let scale = 3.0;

let window_bounds = WindowBounds::Windowed(Bounds {
    origin: point(px(0.0), px(0.0)),
    size: size(px(screen_width_px / scale), px(screen_height_px / scale)),
});

let window_options = WindowOptions {
    window_bounds: Some(window_bounds),
    focus: true,
    show: true,
    window_background: WindowBackgroundAppearance::Transparent,
    ..Default::default()
};
```

**Why Critical**: Using DEFAULT_WINDOW_SIZE here caused the original black screen issue. Must use actual screen dimensions.

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
- **Phase 3**: Dynamic Configuration (DisplayMetrics) - Next
- **Phase 4**: Input Integration (touch/keyboard forwarding)
- **Phase 5**: Production Hardening

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

## Achievement

First successful port of GPUI to Android with:
- Vulkan 1.1 traditional renderpass support (90% device compatibility)
- CosmicTextSystem with Android font support
- Clean command queue architecture for threading
- Proper surface lifecycle management
- Optimized font loading (82% faster startup)
- 60 FPS rendering

---

**Last Updated**: 2026-01-30
