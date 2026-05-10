# Terminal Selection Review

This is the cleanup review for the terminal native selection branch.

## Current Shape

The implementation now follows the simpler terminal policy:

- terminal taps stay on the existing `on_press` focus/keyboard path
- terminal selection starts from `on_long_press`
- empty-cell paste uses a custom native edit menu instead of trying to force a
  selection menu
- terminal does not register output through the noneditable `Selectable` path
- `TerminalInputHandler` owns keyboard/IME/dictation and output selection
  queries through one native input handler

The GPUI framework changes are generic:

- `InputHandler::handles_native_selection()` lets an editable input handler opt
  into native touch selection geometry
- `InputHandler::adjusted_native_selection_range()` lets a handler expand
  collapsed native ranges before UIKit caches rects
- `Window::register_selectable()` remains available for passive custom painted
  read-only surfaces, but terminal output does not use it

## Resolved In This Branch

- Paint no longer builds a full `TerminalSelectionDocument` every frame.
  `TerminalElement` only computes a cheap selectable-text flag and the handler
  builds the document lazily.
- Long-press setup builds a one-off document only for that gesture.
- UTF-16 to byte lookup and word expansion use precomputed document character
  metadata instead of rebuilding temporary vectors.
- Selection range, text extraction, and rect generation share the same
  `TerminalSelectionDocument`.
- Dismissal taps are consumed so they do not also toggle terminal focus or
  keyboard visibility.
- Empty-cell paste no longer silently pastes or no-ops. It shows a native edit
  menu with a safe finite anchor rect and haptic feedback.
- Temporary `SelectionDebug` tracing and the experimental dictation action were
  removed.

## Remaining Tradeoffs

- Terminal output selection is stored as a UTF-16 range on `Terminal`. This is
  intentionally narrower than routing through alacritty `Term::selection`.
  Alacritty-backed selection would be a larger follow-up because native UIKit
  callbacks still require one stable UTF-16 document for text and rect queries.
- The iOS bridge has additional generic state for editable input handlers with
  native selection. Keep future changes at that boundary generic; terminal
  gesture policy belongs in `zedra-terminal`.
- Native menu FFI is still array-based. If more custom native menus are added,
  extract the repeated C string array plumbing or move to one structured payload.

## Unit Coverage

Root `zedra-terminal` tests cover:

- first native hit-test lazily builds output selection state
- selectable output advertises native selection, while empty output does not
- collapsed native ranges expand to a terminal word after hit-testing
- setting a native range without a hit-test candidate does not create output
  selection
- native clear removes output selection and the hit-test candidate
- selectable text detection for empty, blank, visible, and scrolled output
- paste newline normalization and bracketed-paste wrapping
- terminal tap and long-press wrapper behavior

Vendored GPUI tests cover:

- custom `Selectable` routing through the window selection handler
- reusable paint replay for registered selectables
- editable input handlers opting into native selection
- native selection dismissal touch consumption
- active selection geometry hit slop for handle dragging

## Device Checks Before Merge

Manual verification still matters because UIKit owns the visible handles and
menu presentation. Run the iOS terminal section in `docs/MANUAL_TEST.md`,
especially:

- first long press on visible output starts selection
- dragging either handle extends and shrinks the selection
- native copy menu appears for a selected range
- long press on an empty cell shows `Paste`
- dismissal tap does not toggle the keyboard
