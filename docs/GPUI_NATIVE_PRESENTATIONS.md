# GPUI Native Presentations

Native presentation stays in UIKit.

- Rust asks through FFI.
- Swift presents native UI.
- GPUI does not own alert / action sheet / sheet gestures or animation.

## Current split

- `platform_bridge` is the Rust entry point for native presentation requests.
- `ios/Zedra/Presentations.swift` is the Swift presentation layer.
- `platform_bridge::show_custom_sheet(options, view)` takes the caller-owned GPUI view entity to host in the sheet.
- UIKit owns:
  - alert
  - selection / action sheet
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
- custom sheet: native shell + GPUI content
- QR scanner / dictation preview: separate native flows unless we intentionally generalize them
