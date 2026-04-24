# zedra-terminal

Reusable terminal emulator and GPUI renderer. Owns alacritty terminal state, terminal input handling, OSC parsing, selection/IME behavior, and viewport-driven rendering.

## What This Crate Owns

- terminal model and event stream in `src/terminal.rs`
- GPUI terminal view and sizing behavior in `src/view.rs`
- terminal painting in `src/element.rs`
- iOS text-input bridge and selection handling in `src/input.rs`
- key conversion helpers in `src/keys.rs`

## Core Rules

- Keep the split between model and view: `Terminal` owns emulator state, `TerminalView` owns GPUI interaction and viewport sizing, and `TerminalElement` owns rendering.
- Preserve packet-safe parsing. `Processor` and `OscScanner` are intentionally stateful so escape sequences can span network chunks.
- Keep replay semantics intact. The terminal model expects ordered output bytes and tracks state such as `last_seq`, selection, title, and OSC-derived metadata.

## Rendering And Sizing

- `TerminalView` is responsible for converting viewport size into terminal grid size and emitting `TerminalEvent::RequestResize` when the remote PTY should change size.
- Be careful around keyboard insets and bounds reconciliation. The fallback resize path in `view.rs` exists to avoid bad rows/cols when the visible viewport and measured element bounds diverge.
- Rendering changes in `element.rs` should preserve monospace alignment, batching behavior, and selection/cursor correctness.

## Input, IME, And Selection

- `input.rs` contains non-trivial UIKit text-input glue. Treat it as behavior-critical, especially around marked text, dictation, and selection ranges.
- Do not casually collapse IME, dictation, and terminal-selection behavior into one path; the current handlers intentionally distinguish them.
- `Terminal::handle_keystroke`, IME text insertion, and mouse/touch selection logic must stay consistent with GPUI event routing in `view.rs`.

## Events And Integration

- `TerminalEvent` is the public integration surface for host crates. If you add a new terminal-side behavior that the app must react to, expose it through `TerminalEvent`.
- Keep OSC-derived events flowing through the scanner and event channel rather than special-casing them only in the host app.
- `attach_channel()` and channel reattach behavior are part of reconnect support. Preserve task replacement semantics and avoid leaking prior channel tasks.

## Logging And Change Discipline

- Use `tracing` for attach, resize, and input/debug logs.
- Comments should explain lifecycle or rendering constraints, not restate obvious drawing code.
- This crate is reused by the main app; avoid importing app-level concepts or platform-bridge logic here.

## Good Change Shapes

- Input changes often need coordinated updates across `input.rs`, `terminal.rs`, and `view.rs`.
- Rendering changes usually need updates in both `element.rs` and `view.rs` if sizing or cursor/selection math changes.
- If you change hyperlink or OSC behavior, verify the contract with the consuming app crate.

## Validation

- `cargo check -p zedra-terminal`

## Key Files

- `src/terminal.rs` — emulator state, OSC scanning, input/output channel handling
- `src/view.rs` — GPUI interaction, sizing, focus, touch and keyboard behavior
- `src/element.rs` — painting and color/style batching
- `src/input.rs` — native text input and selection bridge
