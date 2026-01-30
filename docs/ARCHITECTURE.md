# Architecture

## Threading Model

**Problem**: JNI callbacks from any thread, GPUI requires single-threaded execution.

**Solution**: Command queue pattern.

```
JNI Thread (any)
    | send command
AndroidCommandQueue (crossbeam channel)
    | drain @ 60 FPS via Choreographer
Main Thread
    | process command
GPUI (single-threaded)
```

Commands:
- `Initialize`, `Destroy`
- `SurfaceCreated`, `SurfaceChanged`, `SurfaceDestroyed`
- `Touch`, `Key`, `Resume`, `Pause`

**Trade-off**: One frame latency, acceptable for 60 FPS.

## Surface Lifecycle

Window (logical) separated from Surface (physical rendering):

```rust
// Window persists
app.open_window(options, |window, cx| { ... });

// Surface attached/detached with Android lifecycle
window.handle_surface_created(native_window, &blade_context);
window.handle_surface_destroyed();
```

## Pixel Density

```rust
// GPUI works in logical pixels
let bounds = size(px(360.0), px(800.0));

// Vulkan needs physical pixels
let physical = gpu::Extent {
    width: (bounds.width * scale) as u32,   // 360 * 3 = 1080
    height: (bounds.height * scale) as u32, // 800 * 3 = 2400
};
```

Conversion happens at `window.rs:handle_surface_created()`.

## Vulkan Compatibility

**Problem**: Blade required VK_KHR_dynamic_rendering (Vulkan 1.3), only 30% of Android devices support it.

**Solution**: Made dynamic rendering optional, fallback to traditional renderpass.

```
Vulkan 1.3+  -> Dynamic rendering (modern path)
Vulkan 1.1   -> Traditional renderpass (fallback)
```

**Result**: 90% device compatibility.

## Lazy Initialization

BladeContext created on first window open, not at app startup:

```rust
fn ensure_blade_context(&self) -> Result<Arc<BladeContext>> {
    if self.blade_context.get().is_none() {
        *context = Some(Arc::new(BladeContext::new()?));
    }
    Ok(context.clone())
}
```

## Font Loading

Following Zed's desktop pattern - minimal fonts synchronous, rest in background:

```rust
// Critical path: 6 essential fonts (~50ms)
load_essential_fonts(&text_system);

// Background: remaining 200+ fonts (non-blocking)
std::thread::spawn(move || load_system_fonts(&text_system));
```

**Result**: Platform init 282ms -> 51ms (82% faster).

## Frame Loop

```
Choreographer.doFrame() @ 60 FPS
    -> JNI: gpuiProcessCommands()
    -> Drain command queue
    -> platform.request_frame_for_all_windows()
    -> GPUI render pipeline
    -> BladeRenderer::draw(scene)
    -> Vulkan submit
```

## Performance

| Component | Time |
|-----------|------|
| Platform init | ~51ms |
| BladeContext | ~20ms |
| Pipeline compilation | ~130ms |
| **Total startup** | ~200ms |

Per frame: <5ms CPU, <4ms GPU (60 FPS with headroom).

## Key Files

| File | Purpose |
|------|---------|
| `src/android_command_queue.rs` | Thread-safe command queue |
| `src/android_jni.rs` | JNI bridge |
| `src/android_app.rs` | Main thread GPUI app |
| `vendor/.../android/platform.rs` | Platform trait impl |
| `vendor/.../android/window.rs` | Window + surface lifecycle |
| `vendor/.../android/text_system.rs` | CosmicTextSystem |
