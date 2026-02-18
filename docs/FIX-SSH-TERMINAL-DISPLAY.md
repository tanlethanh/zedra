# Fix: SSH Terminal Display Issues

**Date**: 2026-02-14
**Branch**: `feat/ssh-terminal-display`

## Problems

When running Claude Code TUI through the Zedra SSH terminal, two categories of issues were observed:

1. **Rendering size mismatch** — The terminal UI did not fill the available width, and the TUI input box overlapped with previous command output.
2. **Missing TUI elements** — The Claude Code banner, header, and interactive UI components were absent or corrupted.

## Root Causes

### 1. Display Scale Factor Never Set (android_app.rs / MainActivity.java)

The GPUI Android platform defaults `display_scale` to `3.0`, but the test device reports a density of `2.75`. Because `platform.set_display_scale()` was never called, the BladeRenderer created a **1178×2618** pixel buffer for a **1080×2400** surface — a ~9% oversize that distorted all coordinate calculations.

Additionally, `getDisplayDensity()` in Java was called *after* `gpuiProcessCriticalCommands()`, so even after adding the Rust-side call, the density value was still the default `3.0` at platform init time.

**Fix**: Call `platform.set_display_scale(density)` immediately after platform creation, and reorder `MainActivity.java` so `getDisplayDensity()` runs before `gpuiProcessCriticalCommands()`.

### 2. VTE Processor State Loss (zedra-terminal/src/lib.rs)

`TerminalState::advance_bytes()` created a **new `Processor`** on every invocation:

```rust
// BEFORE (broken):
pub fn advance_bytes(&mut self, bytes: &[u8]) {
    let mut processor = Processor::new();  // state discarded each call
    processor.advance(&mut self.term, bytes);
}
```

The VTE `Processor` maintains parser state for multi-byte escape sequences. SSH delivers data in arbitrary network-packet-sized chunks, so escape sequences are frequently split across calls. Discarding the processor between calls corrupted the terminal's understanding of cursor movement, colors, and TUI drawing commands — which is why Claude Code's rich UI elements were missing.

**Fix**: Store the `Processor` as a persistent field in `TerminalState`:

```rust
// AFTER (fixed):
pub struct TerminalState {
    term: Term<ZedraListener>,
    processor: Processor,  // persisted across calls
    // ...
}

pub fn advance_bytes(&mut self, bytes: &[u8]) {
    self.processor.advance(&mut self.term, bytes);
    self.mode = *self.term.mode();
}
```

### 3. Column Count Subpixel Rounding (zedra_app.rs)

At non-integer scale factors (2.75), the calculated column count could exceed what the element bounds actually fit due to subpixel rounding. The PTY was told 54 columns, but only 53 characters of width were available in the rendered element.

**Fix**: Subtract 1 column as a safety margin after the floor calculation:

```rust
let columns = ((available_width / cell_width).floor() as usize).saturating_sub(1);
```

## Files Changed

| File | Change |
|------|--------|
| `crates/zedra/src/android_app.rs` | Call `platform.set_display_scale(density)`; simplify terminal sizing; increase font to 14px |
| `crates/zedra-terminal/src/lib.rs` | Persist `Processor` in `TerminalState` instead of recreating per call |
| `android/app/src/main/java/dev/zedra/app/MainActivity.java` | Move `getDisplayDensity()` before `gpuiProcessCriticalCommands()` |
| `crates/zedra-terminal/src/element.rs` | Remove diagnostic logging |
| `crates/zedra-terminal/src/view.rs` | Remove diagnostic logging |

## Verification

After all fixes, Claude Code TUI renders correctly:
- Full banner and header UI elements visible
- Input box on its own row, no overlap
- Terminal width fills available space
- Text is readable at 14px line height (~60 columns on 1080px-wide display)
