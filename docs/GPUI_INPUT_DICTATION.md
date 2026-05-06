# GPUI Input Dictation

Native dictation is part of the GPUI text-input path. On iOS, UIKit drives
dictation through the same `UITextInput` responder used for keyboard and IME
input, but the lifecycle is not the same as ordinary `insertText`.

## Contract

- `GPUIMetalView` remains the single native `UITextInput` responder.
- `gpui_ios` forwards UIKit selector-shaped text and dictation callbacks through
  `InputHandler`; it does not synthesize dictation start/end/cancel state.
- The app input handler owns the synthetic text store it exposes back to UIKit.
- Dictation hypothesis text is stored as marked text until it is committed or
  cancelled.
- Terminal input may preview `insertText` / `replaceRange` text that arrives
  without a terminal-observed `shouldChangeTextInRange` preflight while
  preserving a marked range for UIKit reconciliation, then commit the stream to
  the PTY once.
- The native preview overlay is display-only when used and must not be the
  source of text truth.

## iOS Flow

UIKit can deliver dictation through two shapes:

```text
insertDictationResultPlaceholder
    -> setMarkedText / insertText hypothesis updates
    -> removeDictationResultPlaceholder(willInsertResult=false)
    -> commit streamed hypothesis
```

or:

```text
insertDictationResultPlaceholder
    -> setMarkedText / insertText hypothesis updates
    -> removeDictationResultPlaceholder(willInsertResult=true)
    -> insertDictationResult
    -> commit final result
```

`dictationRecordingDidEnd` only means the microphone stopped recording. UIKit
may still query `markedTextRange` and `textInRange` after that callback, so it
must not clear the marked-text store.

On iOS 16 and newer, UIKit may stream dictated words through `insertText` or
`replaceRange` before exposing a useful dictation-start signal. `gpui_ios`
forwards UIKit selector-shaped calls, including `shouldChangeTextInRange`,
`insertText`, and `replaceRange`, without deciding whether they are dictation.
Terminal input may then stage a safe selector sequence as a live preview backed
by marked text, not as terminal output. The first unconfirmed selector must not
be sent to the PTY and "promoted" later.

## Synthetic Text Store

Dictation requires a stable document model even for surfaces like the terminal
that do not represent an editable app document. `TerminalInputHandler` exposes a
small synthetic document:

- a single-space placeholder when no composition exists
- the current marked dictation hypothesis during active dictation
- the recently committed hypothesis briefly after commit for trailing UIKit
  reconciliation queries
- preview text derived from an `insertText` / `replaceRange` sequence that still
  needs a marked range so UIKit can find the previous hypothesis

The marked range must remain available while UIKit is replacing its previous
hypothesis. Clearing it too early can make `UIDictationController` cancel with
“Could not find the last hypothesis”.

## Terminal Routing

`gpui_ios` exposes raw callbacks only. `TerminalInputHandler` owns the routing:

- confirmed `shouldChangeTextInRange` flows update the terminal keyboard
  context and send minimal PTY diffs
- raw `setMarkedText` flows stay in the marked-text store until committed
- unconfirmed `insertText` / `replaceRange` can stage a preview only from the
  empty anchor or from an already active preview/dictation flow
- live preview text is committed only when UIKit finalizes the dictation stream

Normal multi-character `insertText` is not treated as dictation by length. The
bridge must not infer preview or lifecycle state from phrase shape. Vietnamese
Telex, Japanese IME, suggestions, and dictation all share `insertText` and
`replaceRange`.

When UIKit removes the dictation placeholder with `willInsertResult=false`, the
terminal treats the streamed marked text as final and commits it. If UIKit later
sends a final `insertDictationResult`, the terminal reconciles it as a correction
against the already committed transcript instead of inserting a duplicate.

Vietnamese Telex delete/rewrite can replay unconfirmed text after the confirmed
correction. A terminal-owned rewrite guard keeps that replay on the ordinary
keyboard path instead of bootstrapping dictation preview. If confirmed keyboard
input follows a staged preview, the preview is flushed as ordinary PTY text
before the confirmed edit applies.

## Preview Overlay

`TerminalEvent::DictationPreviewChanged` carries live hypothesis text from
`zedra-terminal` to the app layer for preview-backed dictation flows. The
terminal stream path emits this event for live hypotheses and hides it when the
stream commits or cancels.
`WorkspaceTerminal` forwards preview events through `platform_bridge` to
`Presentations.swift`, where UIKit renders a compact native glass/material
overlay above the keyboard.

The overlay caches the last rendered text, bottom offset, and window bounds so
repeated identical updates do not relayout or replay animations. Tapping the
overlay hides it and asks the owning terminal view to cancel its pending preview
state, so a false-positive preview has an explicit escape hatch.

## Key Files

| File | Purpose |
|------|---------|
| `vendor/zed/crates/gpui/src/platform.rs` | Raw selector-shaped `InputHandler` hooks |
| `vendor/zed/crates/gpui_ios/src/ios/window.rs` | UIKit text-input callback forwarding |
| `crates/zedra-terminal/src/input.rs` | Terminal `InputHandler` bridge |
| `crates/zedra-terminal/src/terminal.rs` | Synthetic marked-text state and preview events |
| `crates/zedra/src/workspace_terminal.rs` | Active-terminal preview routing |
| `crates/zedra/src/platform_bridge.rs` | Preview id allocation and tap-dismiss callback registry |
| `ios/Zedra/Presentations.swift` | Native dictation preview overlay |

## Validation

Use targeted checks:

```sh
cargo test -p zedra-terminal --lib dictation
cargo test -p zedra-terminal --lib streamed_text_input
cargo test -p zedra-terminal --lib unconfirmed_text_input
cargo test -p zedra-terminal --lib text_input_rewrite_guard
cargo check --manifest-path vendor/zed/Cargo.toml -p gpui_ios -p gpui
./scripts/build-ios.sh --sim
```

Manual iOS validation lives in `docs/MANUAL_TEST.md` under
“Terminal Native Dictation On iOS”.
