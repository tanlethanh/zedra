# Technical Debt

Known limitations and planned solutions.

## High Priority

### 1. Hardcoded Screen Dimensions

**Issue**: Only works on 1080x2400 @ 3x DPI devices.

**Location**: `src/android_app.rs:151-157`

```rust
let screen_width_px = 1080.0;  // TODO: Get from DisplayMetrics
let screen_height_px = 2400.0;
let scale = 3.0;
```

**Solution** (Phase 3):
1. Add JNI method to get DisplayMetrics
2. Pass width, height, density to Rust
3. Use dynamic values in window creation

**Effort**: Low (1 day)

---

### 2. Input Events Not Forwarded

**Issue**: Touch and keyboard events captured but not forwarded to GPUI.

**Location**:
- `src/android_app.rs:247` (handle_touch)
- `src/android_app.rs:262` (handle_key)

**Solution** (Phase 4):
```rust
fn handle_touch(&mut self, action: i32, x: f32, y: f32, pointer_id: i32) {
    let event = match action {
        0 => PlatformInput::MouseDown(...),
        1 => PlatformInput::MouseUp(...),
        2 => PlatformInput::MouseMove(...),
        _ => return,
    };
    if let Some(window) = &self.window {
        window.handle_input(event);
    }
}
```

**Effort**: Low (2-3 days)

---

## Medium Priority

### 3. Window Background Hardcoded

**Issue**: Transparent background hardcoded.

**Location**: `src/android_app.rs:168`

**Solution**: Make configurable via app config.

**Effort**: Very Low

---

### 4. Orientation Changes Untested

**Issue**: Portrait/landscape behavior unknown.

**Expected**: Should work due to surface lifecycle design, but needs testing.

**Effort**: Low

---

## Low Priority

### 5. Multi-Window Not Implemented

**Issue**: Single window only.

**Location**: Global NATIVE_WINDOW in `android_jni.rs`

**Solution**: Use HashMap<WindowId, NativeWindow>

---

### 6. App Lifecycle Minimal

**Issue**: Pause/resume handlers don't save/restore state.

**Current**:
```rust
fn handle_pause(&mut self) {
    log::info!("App paused");
}
```

**Should**: Pause animations, save state, release resources.

---

## Completed

### ~~Text Rendering~~ (Phase 2 Complete)

~~Issue: NoopTextSystem stub~~

**Implemented**: CosmicTextSystem with Android font support
- Font loading from `/system/fonts/`
- Sans-serif fallback (Roboto, DroidSans)
- Emoji support (NotoColorEmoji)
- Lazy font loading (82% faster startup)

---

## Summary

| Issue | Priority | Effort | Phase |
|-------|----------|--------|-------|
| Hardcoded Dimensions | High | Low | 3 |
| Input Forwarding | High | Low | 4 |
| Background Config | Medium | Very Low | 3 |
| Orientation | Low | Low | 5 |
| Multi-Window | Low | Medium | Future |
| Lifecycle | Low | Medium | 5 |

**Next Up**: Phase 3 - Dynamic Configuration
