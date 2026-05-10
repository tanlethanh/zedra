# GPUI Focus, Input, and Keyboard Coordination

Zedra treats GPUI focus, platform text input, and software-keyboard presentation
as separate responsibilities. Normal text inputs can use GPUI's default focus and
keyboard behavior. Terminal surfaces opt out of default tap focus so completed
text taps can activate input without racing native text-selection gestures.

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

The terminal is a full-screen text surface with native text-selection gestures.
Tapping terminal text uses the existing focus/keyboard toggle: unfocused taps
focus and show the keyboard, focused taps with a visible keyboard hide it and
blur terminal focus, and focused taps with a hidden keyboard show it again.
Terminal native selection starts from long press, not double tap.

Terminal uses `.track_focus(&focus_handle).manual_focus()`:

- `track_focus` keeps focus state, styles, key context, and input registration
- `manual_focus` prevents pointer-down from focusing before a tap is confirmed
- the terminal wrapper uses GPUI's default `on_press` for keyboard/focus toggling
  and `on_long_press` for terminal selection/menu setup
- terminal press handling owns `focus()`, `blur()`, `show_soft_keyboard()`, and
  `hide_soft_keyboard()`

When a terminal tap should show the keyboard:

```
completed terminal text tap
    -> focus terminal if needed
    -> call show_soft_keyboard() if the terminal was not focused or keyboard is hidden
```

When a focused terminal tap should hide the keyboard:

```
completed terminal text tap
    -> if terminal is focused and keyboard is visible, call hide_soft_keyboard()
    -> blur terminal focus
```

Long press is intentionally not a tap. The terminal ignores long-press release
when deciding whether to request the keyboard, so selection or the paste menu
does not immediately collapse back into tap activation.

Hyperlink taps are excluded from this activation path. They keep their own tap behavior and should not focus, blur, or request the keyboard.

Do not add a double-tap delay to this keyboard request. Double tap is ordinary
tap input from the terminal perspective; native terminal selection is a long
press path.

## iOS Text Interaction

`gpui_ios` maps the policies to UIKit behavior:

```
accepts_text_input=true, manual_focus=false
    -> editable text interaction mode
    -> implicit keyboard request allowed

accepts_text_input=true, manual_focus=true
    -> editable text interaction mode once focused, or earlier if the handler
       explicitly owns native selection
    -> explicit keyboard request controls software-keyboard presentation
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
        -> focused cleared, keyboard hidden

[focused, keyboard hidden]
    tap terminal
        -> focused, keyboard visible
```

Long press owns terminal output selection. Double tap is ordinary tap input from
the terminal perspective and must not add a delay or a separate selection state
machine to the keyboard toggle path.

When native terminal output selection is active, a tap outside the selected
range is consumed by the iOS bridge to dismiss selection. That dismiss tap must
not also become a GPUI terminal press, because it would toggle focus or keyboard
state as a second side effect.

## Logging

Keep these paths quiet in normal builds. The focus/keyboard path runs during interaction and draw, so broad frame or text-input logs can mask the actual timing issue and add debug-build overhead. If this regresses, add short-lived targeted logs with a clear prefix, reproduce once, then remove them after the cause is known.

## Key Files

| File | Purpose |
|------|---------|
| `vendor/zed/crates/gpui/src/elements/div.rs` | `.track_focus` default focus and `.manual_focus()` opt-out |
| `vendor/zed/crates/gpui/src/platform.rs` | `InputHandler` text policy and soft-keyboard auto-request helper |
| `vendor/zed/crates/gpui/src/window.rs` | Focused input-handler registration |
| `vendor/zed/crates/gpui_ios/src/ios/window.rs` | iOS keyboard session and text interaction mode switching |
| `crates/zedra-terminal/src/view.rs` | Terminal tap-state input activation |
| `crates/zedra-terminal/src/element.rs` | Paint-time handler registration and terminal-grid hit coordinates |
| `crates/zedra-terminal/src/input.rs` | Terminal text input routing |
