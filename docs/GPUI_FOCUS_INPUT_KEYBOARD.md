# GPUI Focus, Input, and Keyboard Coordination

Zedra treats GPUI focus, platform text input, and software-keyboard presentation
as separate responsibilities. Normal text inputs can use GPUI's default focus and
keyboard behavior. Terminal surfaces opt out of default tap focus so a completed
tap can toggle focus and keyboard state intentionally.

## Layers

```
tap / key input
    -> GPUI Window event dispatch
    -> FocusHandle state
    -> window.handle_input(focus_handle, input_handler, cx)
    -> PlatformInputHandler
    -> gpui_ios IosWindow / UITextInput
    -> InputHandler
    -> application text sink
```

## Core Contract

`.track_focus(&focus_handle)` registers the handle in the focus tree, enables
focused styles and key context, and installs the default pointer-down focus
transfer. Suppress that default focus transfer with `.manual_focus()` when the
element owns focus changes itself. A lower-level pointer/mouse-down handler can
also suppress default focus by calling `Window::prevent_default()` before the
default focus listener runs.

`Window::handle_input(...)` should be registered for the currently focused text
surface. `InputHandler::accepts_text_input()` only answers whether platform
text and IME callbacks should route to that handler.

`manual_focus()` disables implicit software-keyboard presentation for that
focused surface. The terminal still needs `insertText`, `deleteBackward`, marked
text, and dictation, but only terminal tap logic may call
`window.show_soft_keyboard()`.

## Normal Input Flow

For normal editable text inputs:

```
focused handler accepts text
    -> handle_input registers PlatformInputHandler
    -> platform may auto request keyboard on a new handler session
    -> native editable text interaction can be enabled
```

## Terminal Flow

The terminal is a full-screen text surface with toggle semantics. Tapping a
focused terminal should dismiss the keyboard and blur focus, not immediately
refocus and reopen the keyboard.

Terminal uses `.track_focus(&focus_handle).manual_focus()`:

- `track_focus` keeps focus state, styles, key context, and input registration
- `manual_focus` prevents pointer-down from focusing before the press completes
- completed press handling is the only tap path that calls `focus()` or `blur()`

When a terminal tap should show the keyboard:

```
completed terminal press
    -> focus terminal if needed
    -> mark pending keyboard request
    -> next paint registers TerminalInputHandler with handle_input(...)
    -> deferred callback calls show_soft_keyboard() after the handler exists
```

When a terminal tap should hide the keyboard:

```
completed terminal press while focused and keyboard visible
    -> hide_soft_keyboard()
    -> window.blur()
```

The keyboard request is intentionally deferred. Calling `show_soft_keyboard()`
immediately after focus can run before the next paint installs the platform input
handler. The pending request is consumed from `TerminalElement::paint()` after
`handle_input(...)` has registered the handler, then the deferred callback asks
UIKit to become first responder.

Hyperlink taps are excluded from this toggle path. They keep their own press behavior and should not focus, blur, or request the keyboard.

Do not use `Window::on_next_frame()` for this keyboard request. The request is tied to input-handler registration, not just frame timing, and GPUI's test platform does not drive platform frame callbacks the same way the device runtime does. The paint-time deferred callback keeps the ordering explicit and unit-testable.

## iOS Text Interaction

`gpui_ios` maps the policies to UIKit behavior:

```
accepts_text_input=true, manual_focus=false
    -> editable text interaction mode
    -> implicit keyboard request allowed

accepts_text_input=true, manual_focus=true
    -> no editable text interaction mode
    -> explicit keyboard request still works
    -> UITextInput callbacks still route through the handler

selection handler present
    -> non-editable text interaction mode
```

`GPUIMetalView` remains the single native `UIView` / `UITextInput` responder for
the GPUI window. Editable input handlers and non-editable selection handlers are
separate logical systems. Non-editable selection must not create keyboard focus
or disturb the active input handler.

## Expected Terminal Behavior

```
[unfocused, keyboard hidden]
    tap terminal
        -> focused, keyboard visible

[focused, keyboard visible]
    tap terminal
        -> unfocused, keyboard hidden

[focused, keyboard hidden]
    tap terminal
        -> focused, keyboard visible
```

The third case matters for externally dismissed keyboard states. If focus remains on the terminal while UIKit reports the software keyboard hidden, the next terminal tap should reopen the keyboard rather than blurring the terminal again.

## Logging

Keep these paths quiet in normal builds. The focus/keyboard path runs during interaction and draw, so broad frame or text-input logs can mask the actual timing issue and add debug-build overhead. If this regresses, add short-lived targeted logs with a clear prefix, reproduce once, then remove them after the cause is known.

## Key Files

| File | Purpose |
|------|---------|
| `vendor/zed/crates/gpui/src/elements/div.rs` | `.track_focus` default focus and `.manual_focus()` opt-out |
| `vendor/zed/crates/gpui/src/platform.rs` | `InputHandler` text policy and soft-keyboard auto-request helper |
| `vendor/zed/crates/gpui/src/window.rs` | Focused input-handler registration |
| `vendor/zed/crates/gpui_ios/src/ios/window.rs` | iOS keyboard session and text interaction mode switching |
| `crates/zedra-terminal/src/view.rs` | Terminal tap-state focus and keyboard toggle |
| `crates/zedra-terminal/src/element.rs` | Paint-time handler registration and deferred keyboard request |
| `crates/zedra-terminal/src/input.rs` | Terminal text input routing |
