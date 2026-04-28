# GPUI Input Dictation

Native dictation is part of the GPUI text-input path. On iOS, UIKit drives
dictation through the same `UITextInput` responder used for keyboard and IME
input, but the lifecycle is not the same as ordinary `insertText`.

## Contract

- `GPUIMetalView` remains the single native `UITextInput` responder.
- `gpui_ios` detects dictation lifecycle callbacks and forwards them through
  `InputHandler`.
- The app input handler owns the synthetic text store it exposes back to UIKit.
- Dictation hypothesis text is stored as marked text until it is committed or
  cancelled.
- Terminal commit sends the final transcript to the PTY once.
- The native preview overlay is display-only and must not be the source of text
  truth.

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

## Synthetic Text Store

Dictation requires a stable document model even for surfaces like the terminal
that do not represent an editable app document. `TerminalInputHandler` exposes a
small synthetic document:

- a single-space placeholder when no composition exists
- the current marked dictation hypothesis during active dictation
- the recently committed hypothesis briefly after commit for trailing UIKit
  reconciliation queries

The marked range must remain available while UIKit is replacing its previous
hypothesis. Clearing it too early can make `UIDictationController` cancel with
“Could not find the last hypothesis”.

## Duplicate Guard

When UIKit removes the dictation placeholder with `willInsertResult=false`, the
streamed marked text is treated as final and committed. Some stop paths can then
send a late final insert. The recent streamed-commit guard ignores that late
insert so the terminal does not receive duplicate transcript text.

Normal multi-character `insertText` must not be treated as dictation by length
alone. Dictation buffering requires an explicit native signal:

- `UITextInputContext.isDictationInputExpected`
- a recent wide context query that marks a pending dictation insert
- an already active insert-text dictation stream

## Preview Overlay

`TerminalEvent::DictationPreviewChanged` carries live hypothesis text from
`zedra-terminal` to the app layer. `WorkspaceTerminal` forwards it through
`platform_bridge` to `Presentations.swift`, where UIKit renders a compact native
glass/material overlay above the keyboard.

The overlay caches the last rendered text, bottom offset, and window bounds so
repeated identical updates do not relayout or replay animations.

## Key Files

| File | Purpose |
|------|---------|
| `vendor/zed/crates/gpui/src/platform.rs` | `InputHandler` dictation hooks |
| `vendor/zed/crates/gpui_ios/src/ios/window.rs` | UIKit dictation callback routing and duplicate guard |
| `crates/zedra-terminal/src/input.rs` | Terminal `InputHandler` bridge |
| `crates/zedra-terminal/src/terminal.rs` | Synthetic marked-text state and preview events |
| `crates/zedra/src/workspace_terminal.rs` | Active-terminal preview routing |
| `ios/Zedra/Presentations.swift` | Native dictation preview overlay |

## Validation

Use targeted checks:

```bash
cargo test -p zedra-terminal --lib dictation
cargo check --manifest-path vendor/zed/Cargo.toml -p gpui_ios -p gpui
./scripts/build-ios.sh --sim
```

Manual iOS validation lives in `docs/MANUAL_TEST.md` under
“Terminal Native Dictation On iOS”.
