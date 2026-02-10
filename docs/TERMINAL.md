# Terminal Implementation

This document describes the terminal emulation implementation in Zedra, which provides SSH terminal functionality on Android using GPUI.

## Architecture Overview

The terminal feature consists of three main crates:

```
zedra-ssh (SSH client, defines TerminalSink trait)
    ↑
zedra-terminal (Terminal emulation + GPUI rendering)
    ↑
zedra (Android app, creates terminal with proper sizing)
```

## Components

### zedra-terminal

The terminal crate provides:

1. **TerminalState** (`lib.rs`) - Wraps `alacritty_terminal::Term` for terminal emulation
   - Processes VT100/ANSI escape sequences
   - Manages terminal grid state (cells, cursor, scrollback)
   - Converts GPUI keystrokes to terminal escape sequences

2. **TerminalElement** (`element.rs`) - GPUI Element for rendering the terminal grid
   - Implements text batching for efficient rendering
   - Handles cell colors, cursor shapes, and INVERSE flag
   - Uses embedded JetBrains Mono NL font for monospace rendering

3. **TerminalView** (`view.rs`) - GPUI Render implementation
   - Manages terminal state and SSH output buffer
   - Handles keyboard input and scroll events
   - Implements `TerminalSink` trait for SSH integration

### Embedded Font

The terminal uses JetBrains Mono NL (No Ligatures), an embedded monospace font optimized for terminal rendering:

```rust
pub static JETBRAINS_MONO_REGULAR: &[u8] = include_bytes!("../assets/JetBrainsMonoNL-Regular.ttf");
pub const TERMINAL_FONT_FAMILY: &str = "JetBrains Mono NL";
```

The font is loaded once via `load_terminal_font()` and registered with GPUI's text system.

## Rendering Approach

The terminal rendering follows Zed's `terminal_element.rs` approach:

### Text Batching

Adjacent cells with the same style are batched into `BatchedTextRun` structs:

```rust
struct BatchedTextRun {
    start_line: i32,
    start_col: i32,
    text: String,
    cell_count: usize,  // May differ from text.len() for wide chars
    color: Hsla,
}
```

Batching rules:
- Cells must be on the same line
- Cells must be consecutive (`start_col + cell_count == col`)
- Cells must have the same foreground color
- Blank cells break batches

### Monospace Grid Alignment

Text is shaped with a forced cell width to ensure monospace alignment:

```rust
let shaped = text_system.shape_line(
    text,
    font_size,
    &runs,
    Some(cell_width),  // Force monospace grid
);
```

### Pixel Positioning

Following Zed's approach:
- Background rectangles use `floor()` for position and `ceil()` for width to prevent gaps
- Text uses exact floating-point positions (no floor/ceil)
- Cursor uses `floor()` for position and `ceil()` for width

### Color Handling

The terminal supports:
- 16 named ANSI colors (One Dark theme)
- 216-color cube (indices 16-231)
- 24-level grayscale (indices 232-255)
- True color (RGB spec)
- INVERSE flag handling via `mem::swap(fg, bg)`

## Dynamic Terminal Sizing

Terminal dimensions are calculated based on actual screen size and font metrics:

```rust
// Get viewport size
let viewport = window.viewport_size();

// Measure cell width from font metrics
let cell_width = text_system
    .advance(font_id, font_size, 'm')
    .map(|size| size.width)
    .unwrap_or(px(9.0));

// Calculate available space (minus UI chrome)
let available_width = viewport.width - px(16.0);
let available_height = viewport.height - px(124.0);  // tab bar + header + status

// Calculate grid dimensions
let columns = (available_width / cell_width).floor() as usize;
let rows = (available_height / line_height).floor() as usize;
```

This ensures the PTY size sent to SSH matches the rendered terminal size, preventing overflow.

## SSH Integration

The terminal integrates with SSH via the `TerminalSink` trait:

```rust
pub trait TerminalSink: 'static {
    fn advance_bytes(&mut self, bytes: &[u8]);
    fn set_connected(&mut self, connected: bool);
    fn set_status(&mut self, status: String);
    fn set_send_bytes(&mut self, callback: Box<dyn Fn(Vec<u8>) + Send + 'static>);
    fn terminal_size_cells(&self) -> (u32, u32);
    fn output_buffer(&self) -> OutputBuffer;
}
```

Output flows: SSH I/O task -> OutputBuffer -> TerminalView.process_output() -> TerminalState.advance_bytes()

Input flows: Keyboard -> TerminalView.handle_keystroke() -> zedra_ssh::send_to_ssh()

## Configuration

Current terminal settings:
- Line height: 16px
- Font size: 12px (line_height * 0.75)
- Theme: One Dark
- Cursor color: #528bff (blue)

## Dependencies

- `alacritty_terminal` - Terminal emulation (VT100/ANSI parsing)
- `gpui` - UI framework (rendering, input handling)
- `itertools` - For `chunk_by` grouping of cells by line
- `zedra-ssh` - SSH client integration
