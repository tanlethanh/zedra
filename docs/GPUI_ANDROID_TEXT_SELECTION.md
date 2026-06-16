# GPUI Android Text Selection

## Status

Proposed architecture for native Android text selection in GPUI.

This design keeps the existing GPUI text-selection contract shared with iOS
while giving Android a native selection presenter. Android platform code owns
the visible selection interaction. GPUI owns the selectable document, range,
and geometry.

## Goal

Android text selection should behave like iOS text selection:

- long press starts selection
- native highlights, handles, magnifier, and toolbar appear
- dragging handles updates the GPUI selection range
- selection follows layout and scrolling changes
- copy and custom selection actions use the active GPUI handler
- read-only selection does not request the software keyboard

The Android implementation must support both:

- read-only `SelectionArea` documents through the window selection handler
- editable or manual-focus input handlers that opt into native selection, such
  as terminal output

## Terminology

### SelectionController

`SelectionController` is the native Android selection presenter.

It owns:

- long-press recognition
- selection highlights
- start and end handles
- handle dragging
- magnifier presentation
- contextual toolbar
- dismissal
- touch-stream coordination
- geometry refresh

It does not own the selectable document or independently calculate text
positions. It queries and updates the active GPUI selection handler.

### InputMethodAdapter

`InputMethodAdapter` is the Android editable-text and IME bridge.

It creates and implements the Android `InputConnection` returned by
`GpuiSurfaceView.onCreateInputConnection(...)`. It owns synchronization between
the Android input method and the active editable GPUI input handler.

It is not a text-selection presenter. It must not own long press, selection
handles, highlights, magnifier, or the contextual toolbar.

### Selection Handler

The selection handler is the `PlatformInputHandler` installed through
`PlatformWindow::set_selection_handler(...)`.

It exposes read-only selectable documents created from GPUI `SelectionArea`
content.

### Input Handler

The input handler is the `PlatformInputHandler` installed through
`PlatformWindow::set_input_handler(...)`.

It owns editable text and IME behavior. An input handler participates in native
touch selection only when `query_handles_native_selection()` is true.

## Shared Platform Contract

Android and iOS consume the same GPUI handler operations:

```text
selected_text_range
text_for_range
text_len_utf16
set_selected_text_range
adjusted_native_selection_range
character_index_for_point
nearest_character_index_for_point
bounds_for_range
rects_for_range
clear_selected_text_range
selection_action_presentations
perform_selection_action
```

The platform implementations differ only in presentation:

```text
GPUI PlatformInputHandler contract
├── iOS: UITextInput + UITextInteraction
└── Android: SelectionController
```

On iOS, UIKit queries the handler contract and presents the native interaction.
On Android, `SelectionController` queries the same contract through the Android
GPUI bridge and presents the native interaction.

The shared abstraction is the `InputHandler` trait, and each backend calls the
subset of hooks its native machinery needs — the same way iOS calls the
keyboard-accessory and IME-trait hooks that Android does not. The trait stays
generic with sane defaults; "iOS doesn't call it" is not a special case.

One hook follows from this split: `initial_native_selection_range` (widen a
long-press seed to its enclosing word). Android calls it because it synthesizes
the initial selection itself; iOS does not, because UIKit's `UITextInteraction`
supplies long-press granularity natively. Consequence to keep in mind: iOS word
boundaries come from the UIKit tokenizer while Android's come from GPUI's
`unicode-segmentation`, so the two platforms can pick slightly different word
edges. Unifying that would mean routing iOS through the same GPUI word policy —
deliberately not done, to avoid disturbing the working UIKit selection path.

## Ownership

### GPUI Owns

- selectable document identity
- document text
- UTF-16 document ranges
- active selected range and direction
- point-to-character hit testing
- range adjustment, including word expansion
- selection bounds and per-line rectangles
- clipping and occlusion decisions
- copy text
- custom selection actions

### Android Owns

- native selection interaction state
- gesture timing and touch slop
- highlight drawing
- native-styled handles
- magnifier
- contextual toolbar
- touch capture while dragging handles
- dismissing native presentation
- requesting refreshed geometry after layout changes

Android must not maintain a second text layout or infer character positions
from pixels. The GPUI handler remains the source of truth.

## Android Structure

```text
GpuiSurfaceView
├── InputMethodAdapter
└── SelectionController
    └── SelectionOverlayView

AndroidWindowState
├── input_handler
├── selection_handler
├── selectable_text_hit_regions
└── active_selection_source
```

`SelectionOverlayView` is a transparent native Android view above
`GpuiSurfaceView`. It draws highlights and handles and receives handle-drag
touches. It must not obscure ordinary GPUI touches when selection is inactive.

`SelectionController` coordinates the overlay, magnifier, toolbar, and bridge
calls. It should be framework-owned by `gpui_android`, not app-owned by Zedra.

## Active Selection Source

`SelectionController` resolves one active selection source before starting an
interaction:

1. Use the input handler when it exists, reports
   `query_handles_native_selection()`, and accepts the interaction point.
2. Otherwise use the read-only selection handler when the interaction point is
   inside a current selectable-text hit region.
3. Otherwise do not start selection.

Once selection starts, the selected source remains fixed until dismissal. A
new frame may refresh that handler and its geometry, but a drag must not switch
between input and selection handlers.

Normal terminal taps must continue through the existing focus and keyboard
path. Terminal native selection starts only after long press.

## Selection Lifecycle

### Start

1. `GpuiSurfaceView` receives `ACTION_DOWN`.
2. `SelectionController` records the pointer and schedules long-press
   recognition without consuming the ordinary GPUI touch stream.
3. Movement beyond touch slop, cancellation, or a claimed scroll cancels the
   pending long press.
4. When long press fires, `SelectionController` asks GPUI for
   `character_index_for_point`.
5. If GPUI returns an index, the controller sets a collapsed range.
6. GPUI returns the adjusted range, such as a terminal word selection.
7. The controller captures the touch stream and presents native selection UI.

If hit testing returns no index, selection does not start.

### Update

While a handle is dragged:

1. Convert the Android touch point into GPUI logical coordinates.
2. Query `nearest_character_index_for_point`.
3. Build a proposed range while preserving selection direction.
4. Ask GPUI to set and adjust the range.
5. Query the resulting selected range and geometry.
6. Refresh highlights, handles, magnifier, and toolbar position.

The controller must prevent handle crossing according to Android interaction
rules while preserving the GPUI selection direction.

### Refresh

After a GPUI frame that may affect active selection geometry:

1. Query the selected range.
2. Query `rects_for_range` and endpoint bounds.
3. Compare with the last native geometry snapshot.
4. Invalidate the overlay and contextual toolbar only when geometry changed.

The selection remains valid across scrolling and layout changes as long as the
active handler still exposes it. If the handler disappears, the controller
dismisses the native interaction.

### Dismiss

Dismiss selection when:

- the user taps outside active selection UI
- the active handler is cleared
- the selectable document disappears
- the active window or surface is destroyed
- a conflicting input interaction takes ownership

Dismissal clears native presentation and calls
`clear_selected_text_range()` when the interaction owns the active GPUI
selection.

## Native Presentation

### Highlights

`SelectionOverlayView` draws Android-native selection highlights from
`rects_for_range`.

The bridge must provide all visible selection rectangles in physical view
coordinates. The overlay applies the Android selection highlight color and
clips to the visible GPUI surface.

GPUI continues to own geometry and clipping decisions. Android owns the final
native highlight drawing.

### Handles

The overlay draws native-styled start and end handles anchored to the endpoint
bounds returned by GPUI.

Handles own their drag touch streams. Handle movement must not also scroll or
activate GPUI content underneath the overlay.

### Magnifier

Use Android `Magnifier` while dragging a handle when supported and useful.
Magnifier position follows the drag point; selection position still comes from
GPUI hit testing.

### Contextual Toolbar

Use `ActionMode.Callback2` for the native contextual toolbar.

The toolbar includes:

- `Copy` for any non-empty selection
- `Select all` when the active document exposes a larger selectable range
- custom actions from `selection_action_presentations`

`onGetContentRect(...)` uses the active selection bounds. Geometry changes
invalidate the action mode content rectangle.

Copy reads selected text from the active GPUI handler and writes it through the
Android platform clipboard implementation.

## InputMethodAdapter

`InputMethodAdapter` remains separate from `SelectionController`.

It is responsible for:

- creating the Android `InputConnection`
- IME composition
- committed text
- editable selection synchronization
- surrounding-text queries
- deletion semantics
- batch edits
- initial `EditorInfo` state
- `InputMethodManager.updateSelection(...)`
- extracted-text monitoring when supported

It talks only to the active editable input handler. It must not fall back to
the read-only selection handler.

An input handler may simultaneously support editable IME behavior and native
selection. In that case:

- `InputMethodAdapter` owns IME communication
- `SelectionController` owns native touch-selection presentation
- both read and mutate the same GPUI input handler
- GPUI remains the source of truth for the selected range

The adapter must notify Android when GPUI changes selection or marked text.
Android-originated `InputConnection.setSelection(...)` updates the editable
range but does not start or present native touch selection.

## Bridge API

The Android bridge should expose selection operations by behavior, not by IME
method names:

```text
selectionCanStartAt(x, y) -> boolean
selectionStartAt(x, y) -> boolean
selectionNearestIndexAt(x, y) -> int?
selectionSetRange(start, end, reversed) -> SelectionSnapshot?
selectionSnapshot() -> SelectionSnapshot?
selectionClear()
selectionCopy() -> boolean
selectionActions() -> SelectionAction[]
selectionPerformAction(index) -> boolean
```

`SelectionSnapshot` should include:

```text
range start and end
selection direction
visible highlight rectangles
start-handle bounds
end-handle bounds
overall content bounds
document length when available
```

Use one snapshot call where practical so range and geometry come from the same
GPUI state. Avoid separate JNI calls for every rectangle or range endpoint.

IME bridge methods should remain under `InputMethodAdapter` and use explicit
input-method naming.

## Threading

Android selection callbacks originate on the UI thread. GPUI handler queries
and mutations must execute through the existing Android platform/window path.

Do not hold a Kotlin or JNI callback across a GPUI update. Return immutable
selection snapshots to Kotlin, then update native views after the bridge call
completes.

Selection geometry refresh must be coalesced with GPUI frames. Do not poll
geometry continuously from Android.

## Coordinate Rules

- Android touch and overlay coordinates are physical view pixels.
- GPUI handler hit testing uses logical GPUI pixels.
- Convert physical touch positions to logical positions exactly once at the
  Android window boundary.
- Convert GPUI geometry snapshots to physical view pixels exactly once before
  returning them to Kotlin.
- Include surface/view offsets for embedded GPUI surfaces.

The overlay must use the same surface-local coordinate space as
`GpuiSurfaceView`.

## Current WIP Disposition

The current Android text-selection WIP exposes selection text and ranges through
the anonymous `BaseInputConnection` in `GpuiSurfaceView`.

Revise it as follows:

- remove selection presentation responsibilities from `InputConnection`
- rename and extract the editable bridge into `InputMethodAdapter`
- keep Android clipboard support as an independent platform capability
- add Android storage and lifecycle for `selection_handler`
- add `SelectionController` and `SelectionOverlayView`
- expose selection snapshots and mutations through a selection-specific bridge
- keep read-only selection independent from keyboard requests

The existing `InputConnection` text/range methods may be retained only when
they are required for correct editable IME behavior. They must not be used as
the native selection presentation protocol.

## Implementation Order

1. Extract and correct `InputMethodAdapter` without changing selection UI.
2. Store `selection_handler` and selectable hit regions in
   `AndroidWindowState`.
3. Add a selection-specific bridge and immutable `SelectionSnapshot`.
4. Implement long press and dismissal in `SelectionController`.
5. Implement native highlight and handle drawing in `SelectionOverlayView`.
6. Implement handle dragging and geometry refresh.
7. Add native contextual toolbar, copy, select-all, and custom actions.
8. Add magnifier support.
9. Validate read-only text, terminal output, editable input, scrolling,
   rotation, embedded surfaces, and lifecycle transitions.

## Magnifier Loupe (Deferred)

The drag-handle magnifier loupe is implemented but disabled
(`LOUPE_ENABLED = false` in `SelectionController.kt`). The wiring stays in place:
the loupe tracks the snapped handle and the floating toolbar hides during a drag.

It is off because it can only show GPUI text, never the selection highlight.
Android's `Magnifier` copies from a single surface. Our canvas is a `SurfaceView`
(an out-of-tree SurfaceFlinger layer) holding the GPUI text; the highlight and
handles are drawn in an Android overlay `View` in the app window surface. The
loupe anchored to the `SurfaceView` captures text only; anchored to the overlay
it would capture the highlight but not the text. No single-surface copy composites
both, and Flutter's own engine-drawn loupe (`RawMagnifier`/`BackdropFilter`) hits
the same wall for platform-view surfaces.

### Path to re-enable: unify on an in-tree GPU layer

The fix is to make Android match iOS at the compositor level:

- iOS renders GPUI into a `CAMetalLayer` that lives *in* the Core Animation view
  tree, so UIKit overlays and the `UITextInteraction` loupe snapshot text and
  selection together.
- Android's `SurfaceView` is out-of-tree, which is the only reason the loupe
  cannot see the overlay.

Migrating the canvas to a `TextureView` puts GPUI in-tree (HWUI-composited within
the window), so the overlay highlight and a system `Magnifier` (anchored to a
window view) capture both — unifying the iOS/Android selection/loupe contract and
removing the platform special-casing.

Feasibility notes (verified against the current pipeline):

- `gpui_wgpu` and the `ffi.rs`/`window.rs` surface path are unchanged: a
  `TextureView`'s `SurfaceTexture` yields a `Surface`/`ANativeWindow`, and Vulkan
  uses the same `VK_KHR_android_surface`. The work is a new Kotlin
  `TextureView` emitting the existing `nativeSurfaceCreated/Changed/Destroyed`
  calls; surface replacement is already supported.
- We already render on the UI thread via `Choreographer`, so TextureView's
  "render on the UI thread" constraint costs us nothing.
- The real cost is double compositing (HWUI then SurfaceFlinger) and ~1–3 frames
  of added latency plus extra buffer-queue memory — a global regression against
  the SurfaceView hardware-overlay path. Google recommends SurfaceView for
  API 24+.

Decision gate: build the `TextureView` canvas behind a flag and benchmark
input-to-photon latency and frame time on the Mali-G68 baseline against
SurfaceView. Migrate only if the editor scroll/caret/typing latency delta is
imperceptible; otherwise keep SurfaceView and leave the loupe disabled.

## Non-Goals

- using a hidden `TextView` or `EditText` as the selection document
- duplicating GPUI text layout in Android
- routing read-only selection through the software keyboard
- making `InputConnection` the shared iOS/Android selection abstraction
- moving platform-neutral selection state into Kotlin

## Validation

### Read-Only Selection

1. Long press selectable static or markdown text.
2. Confirm native highlights, handles, and toolbar appear (the magnifier loupe
   is deferred — see "Magnifier Loupe (Deferred)").
3. Drag both handles across lines.
4. Confirm `Copy` returns the exact GPUI document text.
5. Confirm the software keyboard never appears.

### Terminal Selection

1. Tap terminal output and confirm existing focus/keyboard behavior remains.
2. Long press terminal output and confirm native selection starts.
3. Drag handles through terminal rows and whitespace.
4. Confirm typing clears or preserves output selection according to terminal
   policy and still writes to the PTY.

### Editable Input

1. Focus an editable GPUI input.
2. Type with Latin and composing IMEs.
3. Move the editable selection through the IME.
4. Start native touch selection where the handler opts in.
5. Confirm IME state and native selection presentation remain synchronized.

### Lifecycle And Geometry

1. Scroll and resize while selection is active.
2. Rotate or recreate the Android surface.
3. Background and foreground the app.
4. Confirm geometry refreshes or selection dismisses cleanly without stale
   overlays.
