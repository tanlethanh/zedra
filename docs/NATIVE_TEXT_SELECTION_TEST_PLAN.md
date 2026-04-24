# Native Text Selection Test Plan

This plan covers the rollout of native text selection for GPUI-backed mobile views,
starting with iOS non-editable text selection and expanding to editable surfaces.

## Goals

- Long press on non-editable text starts native selection.
- Native copy action appears and copies the selected text.
- Non-editable selection does not show the software keyboard.
- Editable text continues to use the existing keyboard and IME path.
- Terminal/editor regressions are caught before broader rollout.

## Scope

- GPUI framework support for native UIKit text interactions.
- iOS bridge behavior for `UITextInput`, `UITextInteraction`, and edit menu.
- GPUI text layout hit-testing and selection geometry.
- Existing editable input handlers, especially terminal and input fields.

## Out Of Scope

- Android native selection parity in this first slice.
- Full multi-region selection registration in one pass.
- Rich actions beyond copy, such as lookup/share/translate.

## Test Matrix

### 1. Non-Editable Static Text

- Render a plain text block in a GPUI view.
- Long press inside the text.
- Confirm selection handles appear.
- Drag both handles.
- Tap `Copy`.
- Verify clipboard text matches the selected substring.
- Tap outside the selection.
- Confirm selection dismisses cleanly.
- Confirm the software keyboard never appears.

### 2. Wrapped / Multi-Line Text

- Render a text block that soft-wraps across multiple lines.
- Long press near the middle of a wrapped line.
- Extend the selection across line boundaries.
- Copy the selection.
- Verify newline and wrapped-line behavior matches the logical text model.
- Confirm geometry stays aligned after scrolling or resize.

### 3. Editable Input Regression

- Focus a standard text input.
- Type with the software keyboard.
- Use IME composition and marked text.
- Select text and copy it.
- Confirm keyboard behavior is unchanged.
- Confirm selection still tracks caret and marked text correctly.

### 4. Terminal Regression

- Focus terminal and type normally.
- Confirm keyboard still appears.
- Confirm existing IME/dictation path still works.
- Activate text selection.
- Confirm copy action uses selected terminal text.
- Confirm long press does not emit stray keystrokes or clear selection unexpectedly.

### 5. Focus / Keyboard Coordination

- Switch from editable input to non-editable selectable text.
- Confirm keyboard dismisses when the active handler becomes non-editable.
- Switch back to editable input.
- Confirm keyboard can reopen normally.
- Repeatedly toggle between the two states.
- Confirm no flicker or stale responder state.

### 6. Native Edit Menu

- With a non-empty selection, confirm `Copy` is enabled.
- With an empty selection, confirm `Copy` is hidden/disabled.
- Confirm menu anchor tracks the selected range.
- Confirm dismissing the menu does not corrupt selection state.

### 7. Lifecycle / Layout Changes

- Rotate device during active selection.
- Resize embedded GPUI sheet while selection is active.
- Background and foreground the app with an active selection.
- Confirm no crashes and selection geometry refreshes on the next frame.

## Verification Strategy

- Add targeted Rust unit tests for UTF-16 offset conversion and selection range mapping.
- Use iOS simulator/device manual verification for `UITextInteraction` behavior.
- Run `cargo check --manifest-path vendor/zed/Cargo.toml -p gpui_ios -p gpui` after each framework patch.
- Run targeted app checks covering `zedra-terminal` and `crates/zedra/src/ui/input.rs`.

## Rollout Order

1. Land iOS framework support for native editable vs non-editable text interaction modes.
2. Validate with a focused GPUI selectable text fixture.
3. Apply to general app text surfaces.
4. Revisit terminal once the non-editable path is stable.
