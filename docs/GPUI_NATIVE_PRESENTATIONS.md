# GPUI Native Presentations

Native presentation stays in UIKit.

- Rust asks through FFI.
- Swift presents native UI.
- GPUI does not own alert / action sheet / sheet gestures or animation.

## Current split

- `platform_bridge` is the Rust entry point for native presentation requests.
- `ios/Zedra/Presentations.swift` is the Swift presentation layer.
- `platform_bridge::show_custom_sheet(options, view)` takes the caller-owned GPUI view entity to host in the sheet.
- `platform_bridge::show_native_notification(options)` presents a transient
  native in-app notification.
- `platform_bridge::show_native_notification_with_action(options, callback)`
  presents the same notification and calls the Rust callback when the user taps
  the front notification bubble.
- `native_floating_button(...)` is a GPUI element wrapper that owns position,
  lifecycle, and the caller callback while Swift draws the native glass button
  at the wrapper bounds.
- UIKit owns:
  - alert
  - selection / action sheet
  - in-app notification bubble stack
  - floating icon button chrome and glass effect
  - sheet detents
  - sheet gestures
  - sheet animation

## Native notification

Use native notifications for non-blocking, app-local status events: terminal
creation, host-published events, agent completion, background refresh status,
or other state changes that should not interrupt the current workflow.

Do not use native notifications for confirmations, destructive choices, or
errors that require a decision. Use alerts or selection sheets for those.

Basic usage:

```rust
use crate::platform_bridge::{self, NativeNotificationKind, NativeNotificationOptions};

platform_bridge::show_native_notification(
    NativeNotificationOptions::new("Terminal created")
        .message("Host opened a new shell.")
        .system_image("terminal")
        .kind(NativeNotificationKind::Success),
);
```

Usage with a tap action:

```rust
platform_bridge::show_native_notification_with_action(
    NativeNotificationOptions::new("Agent completed")
        .message("Tap to inspect the result.")
        .image("AgentCodex")
        .kind(NativeNotificationKind::Success)
        .duration_secs(4.0),
    || {
        platform_bridge::show_native_notification(
            NativeNotificationOptions::new("Opening result")
                .system_image("hand.tap"),
        );
    },
);
```

Configuration:

- `title`: required by `NativeNotificationOptions::new(title)`.
- `message`: optional secondary text. Keep it short; the native view truncates
  before becoming tall.
- `image(...)`: optional icon name. iOS resolves an asset-catalog image first,
  then falls back to an SF Symbol with the same name. Notification icons are
  rendered as template images and tinted with the same light/dark color as the
  notification title, so SVG assets should use `currentColor` and asset-catalog
  template rendering.
- `system_image(...)`: alias for `image(...)` when the caller intends an SF
  Symbol.
- `kind`: `Info`, `Success`, `Warning`, or `Error`. The kind controls the
  default icon only; native notifications do not trigger haptic feedback.
- `duration_secs`: optional display duration. Default is `3.2`. iOS clamps
  positive durations to a practical transient range.
- `auto_close`: optional. Default is `true`. Set to `false` only when the
  notification should remain until the user taps or swipes it away.

Stack behavior:

- Each call creates an independent notification item.
- New items fade in quickly, enter from above with a near-zero scale, and keep
  settling into full size after opacity has completed.
- Dismissed items fade out quickly, then finish scaling back toward zero while
  translating upward offscreen.
- In the collapsed stack, the newest item is the expanded front bubble.
- Older items peek above it as smaller glass bubbles, so the collapsed stack
  grows upward.
- Only the front bubble receives tap and swipe gestures while collapsed.
- Swipe the front bubble downward to expand the stack downward.
- In the expanded stack, all pending items render as full bubbles from oldest
  at the top to newest at the bottom, and each visible bubble can be tapped or
  swiped.
- When a visible bubble dismisses, the remaining items close the gap smoothly.
- Each item owns its own auto-close timer.

Callback behavior:

- `show_native_notification(...)` has no action callback; tapping still
  dismisses the front bubble.
- `show_native_notification_with_action(...)` invokes its callback at most once
  when the user taps the front bubble.
- Auto-close and swipe dismissal drop the pending callback without invoking it.
- App background cleanup calls `platform_bridge::clear_pending_alerts()`, which
  also clears pending native notification callbacks.

Call site rules:

- Call from event handlers, subscriptions, or async tasks. Do not trigger a
  notification from `render()`.
- Route all callers through `platform_bridge`; app UI code should not call
  UIKit or Swift presentation APIs directly.
- The current concrete renderer is iOS. Other platform bridges may treat this
  as a no-op until they add their own native presentation.

## Custom sheet rule

Custom sheet is a native shell, not UIKit content.

- Swift creates the sheet container and canvas view.
- GPUI renders into the canvas view.
- The sheet body should not be mocked with UIKit labels/buttons.

## Embedded GPUI view

To render inside a native sheet, GPUI needs an embeddable iOS surface.

- root mode: GPUI owns a `UIWindow`
- embedded mode: GPUI attaches `GPUIMetalView` into a provided parent `UIView`

Required pieces:

- FFI to pass a parent `UIView`
- `gpui_ios` path to create an embedded window/view
- resize + frame driving for the embedded surface
- attach / detach when the native container is dismissed or reopened

## Sheet scrolling contract

Sheet-hosted GPUI content does not receive UIKit `touchesMoved` reliably once the
native sheet starts competing for the same drag gesture.

For custom sheet content that needs inner scrolling:

- Swift owns the outer gesture source through the custom sheet content pan recognizer
- Swift forwards two-dimensional pan deltas into the embedded GPUI window as
  synthetic `ScrollWheel` input
- GPUI content reports whether it is currently at the top edge for vertical
  sheet handoff
- Swift rejects only downward vertical drags when content is already at top,
  allowing the native sheet gesture to take over for dismiss / detent motion

Current minimal bridge:

- `gpui_ios_inject_scroll(...)` forwards native deltas into the embedded iOS GPUI window
- `zedra_ios_sheet_content_is_at_top()` exposes the active sheet content boundary

Current content participation:

- markdown preview reports `is_at_top` from its `ScrollHandle`

GPUI layout requirement:

- the hosted GPUI viewport must be explicitly height-constrained
- use `size_full()` on the sheet body viewport wrapper
- use `min_h_0()` on intermediate flex children between the sheet host and the scrollable GPUI node
- keep a stable `.id(...)` on the GPUI scroll node

Without that layout chain, GPUI can measure the scroll node at content height, which makes the native scroll bridge appear wired up while inner scrolling still does not move.

This contract is intentionally minimal:

- native -> GPUI: two-dimensional scroll delta + phase
- GPUI -> native: top-edge boolean

Do not add a broader bidirectional gesture abstraction unless more sheet content
types need it.

## Runtime model

The custom sheet should feel instant.

- do not create a fresh GPUI sheet window on every open
- keep one embedded GPUI sheet window alive
- detach and reattach its native `GPUIMetalView` between sheet presentations
- force the first frame immediately after attach, then continue normal frame driving

Reusing only the renderer context is not enough. Reuse the embedded GPUI window/view too.

## Shared state

The main app and the sheet content run inside the same GPUI app runtime.

- shared app state should live in an app-owned `Entity<T>`
- the main window and sheet window should both receive that same entity handle
- sheet content should not fork its own duplicate state if it needs to reflect live app state

Current custom sheet content reads shared state from the main app, then renders that state inside the native sheet host.

## Practical rule

Use native components for native behavior.

- alerts and selections: fully native
- in-app notifications: native bubble stack
- floating icon buttons: GPUI wrapper for position/lifecycle/callback, native
  chrome
- custom sheet: native shell + GPUI content
- QR scanner / dictation preview: separate native flows unless we intentionally generalize them
