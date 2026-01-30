# Debugging

## Quick Commands

```bash
# Full dev cycle (build + install + launch + monitor)
./scripts/dev-cycle.sh

# Pre-flight check
./scripts/preflight-check.sh

# Background log monitoring
./scripts/logcat-daemon.sh start   # Start
./scripts/logcat-daemon.sh tail    # View
./scripts/logcat-daemon.sh stop    # Stop

# Crash analysis
./scripts/analyze-crash.sh

# Screenshot capture
adb shell screencap -p /sdcard/test.png && adb pull /sdcard/test.png /tmp/test.png
```

## Log Monitoring

```bash
# Basic filtered logs
adb logcat | grep zedra

# With timestamps
adb logcat -v threadtime | grep --line-buffered "zedra"

# Include crash info
adb logcat | grep "zedra\|FATAL\|VK_ERROR\|panicked"
```

## Screenshot Verification

Essential for UI debugging - logs show frames, screenshots prove rendering.

```bash
# Capture after deployment
./scripts/dev-cycle.sh
sleep 3
adb shell screencap -p /sdcard/test.png
adb pull /sdcard/test.png /tmp/test.png
```

**Verification checklist**:
- UI elements visible (not black screen)
- Correct colors and positions
- Proper scaling (1080x2400 for test device)
- Text rendering (if Phase 2+)

## Common Issues

### Black Screen

**Causes**:
- Surface not created
- BladeRenderer initialization failed
- Physical vs logical pixel confusion

**Debug**:
```bash
adb logcat -d | grep "Surface created\|BladeRenderer\|VK_ERROR"
```

**Key location**: `window.rs:handle_surface_created()` - verify physical pixel dimensions.

### Wrong Scaling

**Causes**:
- Incorrect scale factor
- Hardcoded dimensions don't match device

**Key locations**:
- `android_app.rs:151-157` - screen dimensions
- `window.rs:206-215` - pixel conversion

### Crash on Launch

**Check**:
```bash
# Vulkan support
adb shell getprop ro.hardware.vulkan

# Recent crash
./scripts/analyze-crash.sh
```

**Common**:
- `UnsatisfiedLinkError` - rebuild with `./scripts/build-android.sh`
- `VK_ERROR` - check Vulkan 1.1+ support
- `panicked at` - Rust panic, check location in log

### Missing Text

**Causes**:
- Font not found
- Font loading failed
- Glyph not in loaded fonts

**Debug**:
```bash
adb logcat | grep "font\|Font\|glyph"
```

## Device Health Check

```bash
# Device connected
adb devices

# Vulkan support
adb shell getprop ro.hardware.vulkan

# Storage
adb shell df /data/local/tmp

# Boot complete
adb shell getprop sys.boot_completed  # Should be "1"
```

## Error Patterns

| Pattern | Cause | Fix |
|---------|-------|-----|
| `UnsatisfiedLinkError` | JNI lib not found | Rebuild Rust |
| `VK_ERROR_*` | Vulkan error | Check device support |
| `Surface` errors | Lifecycle issue | Check surface callbacks |
| `panicked at` | Rust panic | Check panic location |
| `JNI ERROR` | JNI method mismatch | Verify Java/Rust signatures |

## Video Recording

```bash
# Record (max 180s)
adb shell screenrecord /sdcard/demo.mp4  # Ctrl+C to stop
adb pull /sdcard/demo.mp4 /tmp/demo.mp4
```
