# Terminal Selection Handoff

This documents the terminal native selection model used by the iOS terminal
surface.

## Goal

Terminal output selection must coexist with terminal keyboard input. The
terminal stays an editable text input for keyboard, IME, and dictation, but the
output text is read-only and selected through terminal-owned state.

## Ownership

- `TerminalView` owns terminal gestures: normal taps, long presses, scroll, and
  hyperlink routing.
- `TerminalInputHandler` owns the terminal `UITextInput` protocol surface. It
  switches between keyboard/IME context and output selection geometry.
- `TerminalSelectionDocument` builds a visible terminal text document in UTF-16
  coordinates for native selection callbacks.
- `gpui_ios` stays generic. It lets an editable input handler opt into native
  selection geometry, but it does not contain terminal-specific selection
  policy.
- The app platform bridge owns the custom native edit menu used for empty-cell
  paste.

## Gesture Policy

- Single tap uses the existing terminal focus and keyboard toggle.
- Double tap is ordinary terminal tap input. It does not start terminal output
  selection.
- Long press on output text starts terminal-owned output selection.
- Long press on an empty terminal cell shows a native edit menu with `Paste`
  when clipboard text exists.
- A tap outside active output selection dismisses that selection and must not
  also toggle terminal focus or keyboard visibility.
- Native selection callbacks clear only output selection. Keyboard visibility
  remains owned by the tap/focus path.

## Native Selection Flow

1. Paint registers `TerminalInputHandler` when terminal input is focused or when
   visible terminal output has selectable text.
2. `handles_native_selection()` lets iOS keep the editable text interaction
   available for long-press selection without switching terminal output to the
   noneditable read-only selection handler.
3. The handler builds `TerminalSelectionDocument` lazily when UIKit asks for hit
   testing, range adjustment, selected text, or selection rects.
4. Collapsed native ranges from UIKit are expanded to a terminal word before
   range geometry is cached.
5. Text input mutations clear terminal output selection first, then route input
   through the normal PTY keyboard/IME path.

## Paste Menu

Empty terminal cells do not have native selection text, so the terminal wrapper
requests a Zedra-owned native edit menu. The menu is anchored slightly above the
long-press point, triggers medium haptic feedback, and sends selected clipboard
text through `Terminal::paste_text(&str)`.

`Terminal::paste_text` preserves bracketed paste behavior, strips ESC bytes in
bracketed paste mode, and normalizes newlines to carriage returns outside
bracketed paste mode.

## Deferred Work

- Terminal output selection currently stores a UTF-16 range rather than routing
  through alacritty `Term::selection`. Mapping native ranges back to terminal
  grid selection can be done later if scrollback persistence or deeper terminal
  selection semantics become necessary.
- The native edit-menu FFI still uses parallel arrays for labels and image
  names. A JSON payload or shared array helper would reduce bridge boilerplate
  if more native menu types are added.

## Manual Verification

Use `docs/MANUAL_TEST.md`, section 11. The highest-value device checks are:

- first tap focuses terminal and shows the keyboard
- long press on output text starts selection on the first attempt
- selection handles drag to extend and shrink the range
- selected range shows native copy actions
- tap outside selection only dismisses selection
- long press on an empty cell shows the `Paste` menu
- paste reaches a running PTY command through terminal paste handling
