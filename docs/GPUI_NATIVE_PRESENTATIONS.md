# GPUI Native Presentations

Native presentation stays in UIKit.

- Rust asks through FFI.
- Swift presents native UI.
- GPUI does not own alert / action sheet / sheet gestures or animation.

## Current split

- `platform_bridge` is the Rust entry point for native presentation requests.
- `ios/Zedra/Presentations.swift` is the Swift presentation layer.
- `platform_bridge::show_custom_sheet(options, view)` takes the caller-owned GPUI view entity to host in the sheet.
- `native_floating_button(...)` is a GPUI element wrapper that owns position,
  lifecycle, and the caller callback while Swift draws the native glass button
  at the wrapper bounds.
- UIKit owns:
  - alert
  - selection / action sheet
  - floating icon button chrome and glass effect
  - sheet detents
  - sheet gestures
  - sheet animation

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
- floating icon buttons: GPUI wrapper for position/lifecycle/callback, native
  chrome
- custom sheet: native shell + GPUI content
- QR scanner / dictation preview: separate native flows unless we intentionally generalize them
