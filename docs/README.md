# Zedra Documentation

GPUI on Android with Vulkan 1.1 support.

## Current Status

**Phase 2 Complete**: Core rendering + text rendering working at 60 FPS.

- Vulkan 1.1 traditional renderpass (90% device compatibility)
- CosmicTextSystem with Android font support
- Thread-safe command queue architecture
- Optimized font loading (82% faster startup)

**Test Device**: Mali-G68 MC4, 1080x2400 @ 3x DPI

## Quick Start

```bash
# Setup
git submodule update --init --recursive
rustup target add aarch64-linux-android

# Build and run
./scripts/dev-cycle.sh

# Or manual:
./scripts/build-android.sh
cd android && ./gradlew installDebug
adb logcat | grep zedra
```

## Architecture

```
JNI Thread -> Command Queue -> Main Thread -> GPUI -> Blade -> Vulkan
```

**Key Design**:
- Command queue decouples JNI threading from single-threaded GPUI
- Lazy BladeContext created on first window
- Logical pixels (GPUI) -> Physical pixels (Vulkan) at window boundary
- Vulkan 1.1 fallback for broad device support

## Project Structure

```
src/
  android_jni.rs           # JNI bridge
  android_command_queue.rs # Thread-safe queue
  android_app.rs           # Main thread GPUI app
  zedra_app.rs             # Demo UI

android/app/.../dev/zedra/app/
  MainActivity.java        # Activity + frame loop
  GpuiSurfaceView.java     # Surface management

vendor/zed/crates/gpui/src/platform/android/
  platform.rs              # AndroidPlatform
  window.rs                # AndroidWindow
  text_system.rs           # CosmicTextSystem
  dispatcher.rs            # Task queue
```

## Documentation

| Document | Purpose |
|----------|---------|
| [ARCHITECTURE.md](ARCHITECTURE.md) | Design decisions and patterns |
| [TECHNICAL_DEBT.md](TECHNICAL_DEBT.md) | Known issues with solutions |
| [DEBUGGING.md](DEBUGGING.md) | Debug workflow and tools |

## Roadmap

- **Phase 1**: Foundation - Complete
- **Phase 2**: Text Rendering - Complete
- **Phase 3**: Dynamic Configuration (DisplayMetrics)
- **Phase 4**: Input Integration (touch/keyboard)
- **Phase 5**: Production Hardening
