# GPUI Focus, Input, and Keyboard Coordination

This documents the mobile input contract used by Zedra terminals. The key rule is that text input delivery, focus changes, and software-keyboard requests are separate responsibilities.

## Layers

```
tap / key input
    -> GPUI Window event dispatch
    -> FocusHandle state
    -> window.handle_input(focus_handle, input_handler, cx)
    -> PlatformInputHandler
    -> gpui_ios IosWindow / UITextInput
    -> TerminalInputHandler
    -> terminal PTY
```

The terminal is unusual because it is a full-screen text surface. A normal text input can use default focus and keyboard behavior. The terminal cannot, because tapping the focused terminal must dismiss it instead of refocusing and reopening the keyboard.

## Input Handler Policies

`InputHandler::accepts_text_input()` answers whether the handler wants platform text and IME callbacks. The terminal returns `true` because it still needs `insertText`, `deleteBackward`, marked text, and dictation callbacks.

`InputHandler::disable_default_keyboard_behavior()` answers whether GPUI/platform code should avoid implicit keyboard requests for this handler. The terminal returns `true`. It still accepts text, but only explicit terminal tap logic may call `window.show_soft_keyboard()`.

`InputHandler::disable_default_focus_behavior()` answers whether GPUI focusable elements should avoid default tap-to-focus behavior for this handler. The terminal returns `true`. Terminal taps implement their own toggle state instead of relying on `.track_focus`.

Default text inputs should keep both disable flags as `false`.

## Default Keyboard Behavior

For normal editable text inputs:

```
focused handler accepts text
    -> handle_input registers PlatformInputHandler
    -> platform may auto request keyboard on a new handler session
    -> native editable text interaction can be enabled
```

For terminal-style handlers:

```
focused handler accepts text
    -> handle_input registers PlatformInputHandler
    -> disable_default_keyboard_behavior=true
    -> platform does not auto request keyboard
    -> explicit window.show_soft_keyboard() is required
```

This prevents a focused terminal from re-showing the keyboard during unrelated renders, drawer drags, or first-frame platform refreshes.

## Default Focus Behavior

`.track_focus(&focus_handle)` normally installs default mouse and pointer down handlers that focus the element during bubble dispatch. This is correct for normal controls.

When a focused input handler returns `disable_default_focus_behavior=true`, GPUI marks that focus handle as owning focus behavior for the rendered frame. The default `.track_focus` handlers skip automatic focus for that handle. The terminal also calls `window.prevent_default()` during tap-start handling so parent focusable elements do not take the event.

## Terminal Tap State

Terminal tap behavior is owned by `TerminalView` at tap start:

```
unfocused terminal tap
    -> prevent default focus behavior
    -> focus terminal
    -> mark a pending keyboard request
    -> next terminal paint registers the input handler
    -> TerminalElement defers show_soft_keyboard() until after paint
    -> keyboard request runs with a valid platform handler

focused terminal tap
    -> prevent default focus behavior
    -> hide_soft_keyboard()
    -> window.blur()
```

The keyboard request is intentionally asynchronous. Calling `show_soft_keyboard()` immediately after focus can run before the next frame has installed the platform input handler. On iOS, that means `refresh_text_input_state()` sees no handler and clears the request. The terminal stores a pending keyboard request, then `TerminalElement::paint()` registers the input handler and schedules a deferred callback. That callback runs after paint, when the platform handler is available, before asking UIKit to become first responder.

Hyperlink taps are excluded from this toggle path. They keep their own press behavior and should not focus, blur, or request the keyboard.

Do not use `Window::on_next_frame()` for this keyboard request. The request is tied to input-handler registration, not just frame timing, and GPUI's test platform does not drive platform frame callbacks the same way the device runtime does. The paint-time deferred callback keeps the ordering explicit and unit-testable.

## iOS Text Interaction

`gpui_ios` maps the policies to UIKit behavior:

```
accepts_text_input=true
disable_default_keyboard_behavior=false
    -> editable text interaction mode
    -> implicit keyboard request allowed

accepts_text_input=true
disable_default_keyboard_behavior=true
    -> no editable text interaction mode
    -> explicit keyboard request still works
    -> UITextInput callbacks still route through the handler

selection handler present
    -> non-editable text interaction mode
```

This keeps the terminal from showing native UIKit caret or selection handles while still allowing IME and software-keyboard text delivery.

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
        -> unfocused, keyboard hidden
```

The third case matters for hardware-keyboard or externally dismissed keyboard states. Focused terminal taps are always dismiss/unfocus, not "show keyboard".

## Logging

Keep these paths quiet in normal builds. The focus/keyboard path runs during interaction and draw, so broad frame or text-input logs can mask the actual timing issue and add debug-build overhead. If this regresses, add short-lived targeted logs with a clear prefix, reproduce once, then remove them after the cause is known.

## Key Files

| File | Purpose |
|------|---------|
| `vendor/zed/crates/gpui/src/platform.rs` | `InputHandler` policy methods and `PlatformInputHandler` policy storage |
| `vendor/zed/crates/gpui/src/window.rs` | Per-frame input registration and focus-policy tracking |
| `vendor/zed/crates/gpui/src/elements/div.rs` | `.track_focus` default focus behavior |
| `vendor/zed/crates/gpui_ios/src/ios/window.rs` | iOS keyboard session and text interaction mode |
| `crates/zedra-terminal/src/view.rs` | Terminal tap-state focus and keyboard toggle |
| `crates/zedra-terminal/src/input.rs` | Terminal `InputHandler` policy and UITextInput routing |
