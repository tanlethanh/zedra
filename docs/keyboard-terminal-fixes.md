# iOS Keyboard and Terminal Fixes

## Overview

Three interrelated issues were fixed in the iOS keyboard/terminal interaction:

1. Tapping the terminal toggled the keyboard correctly but could get stuck
2. The keyboard accessory bar (shortcut keys above keyboard) was not appearing
3. After typing `clear` and dismissing the keyboard, old terminal content was pulled into view

---

## Fix 1: Keyboard Toggle (tap to show/hide)

**File**: `crates/zedra-terminal/src/view.rs`

The tap handler in `TerminalView` previously tracked keyboard state with a local bool, which could desync from UIKit's actual state. Changed to read the real platform state via `is_keyboard_visible_fn` before toggling:

```rust
let current = this.is_keyboard_visible_fn.as_ref().map_or(false, |f| f());
this.request_keyboard(!current);
```

---

## Fix 2: Keyboard Accessory Bar

**Problem**: The accessory bar (row of shortcut keys shown above the software keyboard) was never appearing.

**Root cause**: `inputAccessoryView` in `vendor/zed/crates/gpui_ios/src/ios/window.rs` was guarded by a `SOFTWARE_KEYBOARD_VISIBLE` flag. UIKit queries `inputAccessoryView` at `becomeFirstResponder` time, *before* `UIKeyboardWillShowNotification` fires. Since the flag was still false at that moment, the method returned nil and UIKit cached that nil — no accessory bar ever appeared.

**Fix**: Removed the guard so `inputAccessoryView` always returns the registered view pointer:

```rust
extern "C" fn input_accessory_view(_this: &Object, _sel: Sel) -> *mut Object {
    let ptr = KEYBOARD_ACCESSORY_VIEW.load(std::sync::atomic::Ordering::Relaxed);
    ptr as *mut Object
}
```

UIKit only shows the accessory view when a keyboard is actually on screen, so returning a non-nil view unconditionally is safe.

**Accessory bar styling** (`ios/Zedra/main.m`): The original `UIToolbar` with `UIVisualEffectView` (blur/glass material) was overflowing its bounds on iOS 26. Replaced with a plain transparent `UIView` containing `UIButton`s and a 0.33pt hairline border at `alpha=0.12`. This keeps the bar visually minimal without the glass overflow.

**Keys**: Esc, Tab, ←, ↓, ↑, →, ⏎ (7 keys, no dismiss button — users tap the terminal to dismiss).

---

## Files Changed

| File | Change |
|------|--------|
| `crates/zedra-terminal/src/view.rs` | Tap-toggle fix using `is_keyboard_visible_fn` |
| `crates/zedra/src/ios/bridge.rs` | `dismiss_keyboard` key name handling in `zedra_ios_send_key_input` |
| `crates/zedra/src/keyboard.rs` | Debug logging in `make_keyboard_handler` |
| `vendor/zed` (gpui_ios) | Remove `SOFTWARE_KEYBOARD_VISIBLE` guard from `input_accessory_view` |
| `ios/Zedra/main.m` | Replace `UIToolbar` with plain `UIView` + `UIButton`s; 7-key accessory bar |
| `ios/Zedra.xcodeproj/project.pbxproj` | Xcode project file updates |
