# GPUI Mobile Text Protocol

## Status

Proposed framework design for mobile text selection and mobile text input in GPUI.

This document defines the public GPUI API shape we should build toward for iOS and Android. It is intentionally framework-level, not UIKit-specific or Android-specific.

## Goals

- Support native-feeling long-press text selection for general app text, not only editable text inputs.
- Keep editable text input and read-only text selection as separate GPUI concepts.
- Support static text, markdown, logs, terminal output, and editors with one coherent model.
- Preserve a path to Android without forcing Android to mimic UIKit internals.
- Allow selection to span multiple text elements inside one logical document region.

## Non-Goals

- Exact implementation details for every platform callback.
- Rich text editing semantics for non-editable content.
- Arbitrary cross-view selection across unrelated UI surfaces.

## Problem Summary

Today GPUI only exposes `Window::handle_input(...)`, which is the editable text path. That path is appropriate for:

- keyboard text insertion
- IME composition
- editable cursor/caret behavior
- platform text replacement APIs

It is not the right public abstraction for:

- markdown preview selection
- static labels or article text
- non-editable terminal transcript selection
- log output selection

Using the editable input path for read-only content causes design problems:

- static text can appear focused like an editor
- the keyboard can appear when selection starts
- a caret can blink on non-editable content
- selection becomes artificially scoped to a single element

## Design Principles

### 1. Separate Editable Input From Selection

GPUI should expose two public capabilities:

- `InputHandler`
- `SelectionGroup`

`InputHandler` is for editable text.

`SelectionGroup` is for selecting text, regardless of whether the content is editable.

These concepts may share some platform machinery internally, but they should not share the same public API.

### 2. Selection Is Document-Centric, Not Element-Centric

Selection should be modeled in group document coordinates, not per-element coordinates.

That means a selection can start in one text element and end in another, as long as both belong to the same logical selection group.

### 3. Platform Backends May Differ

The GPUI API should express capabilities, not platform internals.

Expected platform mappings:

- iOS: `UITextInteraction` plus a read-only `UITextInput` adapter for selection groups
- Android: GPUI-managed selection gestures and handles, `ActionMode` for copy/select-all, magnifier where appropriate

The public GPUI API should not expose UIKit-only concepts such as `UITextRange` or Android-only concepts such as `ActionMode`.

### 4. Non-Editable Selection Must Not Behave Like Focused Input

A non-editable selection group must not imply:

- software keyboard presentation
- insertion caret visibility
- editable replacement behavior
- editor-style focus semantics

## Public API Overview

### Editable Text

Editable text continues to use `Window::handle_input(...)`.

That path remains responsible for:

- keyboard input
- IME
- marked text
- replacement
- editable selection synchronization

### Non-Editable Or Shared Selection

GPUI should add a first-class selection API with three layers:

- `selection_container()` for shared cross-element selection scope
- `.selectable()` on text elements for common usage
- low-level fragment registration for custom elements

The low-level API should still be paint-time, mirroring `handle_input(...)`, but ordinary app code should not need to call it directly.

## Core Types

### SelectionGroup

```rust
#[derive(Clone)]
pub struct SelectionGroup { /* opaque */ }
```

Purpose:

- identifies one logical selectable document region
- owns selection state for that region
- lets multiple elements participate in one shared selection

Expected creation:

```rust
let selection_group = cx.new_selection_group();
```

Expected storage:

- markdown view state
- article/document views
- terminal surface state
- log viewer state

Each logical document region should own one stable handle.

This should follow existing GPUI handle patterns like focus handles and scroll ids: the view owns a stable handle, while the window/backend owns transient interaction state for the current frame.

### Selection Container

The primary grouping API should be a wrapper element, not a callback-only helper:

```rust
selection_container()
    .child(StyledText::new(title).selectable())
    .child(StyledText::new(paragraph_1).selectable())
    .child(StyledText::new(paragraph_2).selectable())
```

This container creates an ambient `SelectionGroup` scope for descendants.

Behavior:

- descendant selectable elements register into the nearest parent selection container
- GPUI builds one logical selection document for that subtree
- selection can span multiple child text elements

This is the right default for markdown, article views, logs, and terminal transcript rendering.

### Selectable Text Opt-In

Primitive text should expose an explicit builder entry point:

```rust
text("Hello world").selectable()
```

This should use the existing primitive uniform-text rendering path, not force callers to switch to `StyledText` for ordinary labels and body copy.

Text elements should be able to opt into selection declaratively:

```rust
text(text).selectable()
StyledText::new(text).selectable()
InteractiveText::new(id, styled).selectable()
```

Optional configuration:

```rust
StyledText::new(text)
    .selectable()
    .selection_order(order)
    .separator_after("\n\n")
```

Expected behavior:

- if there is a parent `selection_container()`, the element joins that group
- if there is no parent selection container, the element gets an implicit single-element group

That means `.selectable()` should work for simple standalone text without requiring extra scaffolding, while `selection_container()` remains the way to opt into cross-element selection.

Raw string children should continue to work for display-only text:

```rust
div().child("Status")
```

But selectable primitive text should use the explicit builder form:

```rust
div().child(text("Status").selectable())
```

This keeps the API explicit and avoids trying to add builder-style selection methods directly onto raw string literals.

### SelectableTextFragment

```rust
pub trait SelectableTextFragment: 'static {
    fn group_fragment_id(&self) -> ElementId;

    fn group_order(&self) -> u64;

    fn text(
        &mut self,
        window: &mut Window,
        cx: &mut App,
    ) -> SharedString;

    fn separator_after(
        &mut self,
        window: &mut Window,
        cx: &mut App,
    ) -> SharedString {
        SharedString::default()
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<usize>;

    fn rects_for_range(
        &mut self,
        range_utf16: Range<usize>,
        window: &mut Window,
        cx: &mut App,
    ) -> SmallVec<[Bounds<Pixels>; 4]>;

    fn word_range_at_point(
        &mut self,
        point: Point<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Range<usize>>;
}
```

Purpose:

- contribute one selectable text fragment to a logical group
- expose local text and geometry
- let GPUI map between local fragment coordinates and group document coordinates

Notes:

- `group_order()` determines concatenation order inside the group
- `separator_after()` lets the group serialize text naturally across fragments
- `character_index_for_point()` and `rects_for_range()` are local to the fragment
- `word_range_at_point()` improves long-press selection behavior

### SelectionGroupState

This is internal GPUI state, not necessarily public API.

Suggested shape:

```rust
pub struct SelectionGroupState {
    pub active: bool,
    pub anchor: Option<GroupPosition>,
    pub focus: Option<GroupPosition>,
    pub selection: Option<UTF16Selection>,
    pub fragments: Vec<RegisteredSelectionFragment>,
}

pub struct GroupPosition {
    pub fragment_id: ElementId,
    pub offset_utf16: usize,
}
```

Responsibilities:

- track active group
- map between group ranges and fragment-local ranges
- compute copy text for the current selection
- drive selection handle placement
- support select-all
- clear selection when interaction ends

## Selection Semantics

### Group Coordinates

Each fragment contributes:

- local text
- local UTF-16 length
- optional separator after it

GPUI computes a flattened group document:

```text
fragment_0.text + fragment_0.separator_after
+ fragment_1.text + fragment_1.separator_after
+ ...
```

Selection is stored against that flattened UTF-16 document.

### Cross-Element Selection

Cross-element selection should be supported within a single `SelectionGroup`.

That means:

- paragraph to paragraph selection in markdown: yes
- heading into paragraph: yes
- code block into next paragraph: yes, if both are in the same group
- text in one panel into text in another unrelated panel: no

This is the right default for markdown, article views, and structured documents.

### Separator Policy

`separator_after()` is part of the protocol because visual elements do not always concatenate the way users expect copied text to read.

Examples:

- paragraph: `"\n\n"`
- heading: `"\n\n"`
- list item: `"\n"`
- table cell: `"\t"` or a table-aware export policy
- code block line container: `"\n"`
- inline span inside the same paragraph: `""`

Separator policy should be owned by the higher-level view constructing the fragments, not guessed by the platform backend.

## Usage Patterns

### Example 1: Standalone Selectable Text

```rust
fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    text("Zedra lets you access your desktop workspace from your phone.")
        .selectable()
}
```

Behavior:

- long press starts selection
- copy menu appears
- no keyboard
- no editable caret
- GPUI creates an implicit single-element selection group

### Example 2: Markdown As One Selection Group

```rust
selection_container()
    .id("markdown-selection")
    .flex()
    .flex_col()
    .child(text(title).selectable().separator_after("\n\n"))
    .child(text(paragraph_1).selectable().separator_after("\n\n"))
    .child(text(paragraph_2).selectable())
```

Result:

- a long press in one paragraph can drag into the next paragraph
- copy returns combined text in reading order
- links remain a separate tap interaction layered on top of the same text

For explicit ordering:

```rust
selection_container()
    .child(text(title).selectable().selection_order(0).separator_after("\n\n"))
    .child(text(paragraph_1).selectable().selection_order(1).separator_after("\n\n"))
    .child(text(paragraph_2).selectable().selection_order(2))
```

### Example 3: Inline Spans In One Paragraph

If a paragraph renders separate inline spans for styling:

- bold run
- normal run
- inline code run
- link run

these should usually not become separate top-level fragments unless needed.

Prefer one fragment per paragraph with one `TextLayout`, because:

- layout is already continuous
- wrapping geometry is easier
- copy text is simpler
- cross-span selection becomes natural

Use multiple fragments only when the rendered text is genuinely split across layout surfaces.

### Example 4: Terminal

Terminal should use both protocols:

- `handle_input(...)` for keyboard/IME
- `.selectable()` or low-level selection fragments for transcript selection inside a selection container

That lets the terminal remain keyboard-capable without pretending it is editable document text.

Suggested behavior:

- tap: focus terminal and show keyboard
- long press on transcript: start non-editable selection
- selection shows copy actions, not editable caret behavior

## Recommended Built-In GPUI Helpers

To make the protocol practical, GPUI should provide reusable helpers built on `TextLayout`.

### TextLayoutSelectionFragment

```rust
pub struct TextLayoutSelectionFragment {
    pub id: ElementId,
    pub order: u64,
    pub text: SharedString,
    pub layout: TextLayout,
    pub separator_after: SharedString,
}
```

This helper should:

- use `TextLayout::index_for_position(...)`
- use `TextLayout::position_for_index(...)`
- derive rects for wrapped selections
- expose text and separator

This should cover most `StyledText` and markdown block usage.

### `.selectable()` On Existing Text Elements

The preferred high-level API is not a separate `SelectableText` element.

Instead, existing text elements should grow selection support:

- `text(...).selectable()`
- `StyledText::selectable()`
- `InteractiveText::selectable()`

Those methods should:

- render exactly like the existing element
- register a `TextLayoutSelectionFragment` automatically
- inherit the nearest parent `SelectionGroup`
- fall back to an implicit single-element group when no parent group exists

This keeps the API aligned with GPUI’s existing element-builder style.

### Low-Level Fragment Registration

Custom elements still need an escape hatch:

```rust
impl Window {
    pub fn handle_selectable_text(
        &mut self,
        group: &SelectionGroup,
        fragment: impl SelectableTextFragment,
        cx: &App,
    );
}
```

This should be used by:

- custom markdown block renderers
- terminal transcript surfaces
- advanced rich text elements

It should not be the primary API used in ordinary app code.

## Platform Backend Responsibilities

### iOS

Use native text selection UI.

Suggested backend structure:

- one active non-editable selection interaction per window
- one active editable input interaction per window
- a read-only adapter that exposes the active group selection to UIKit

Responsibilities:

- start selection from long press
- map UIKit selection updates into group document ranges
- return selection geometry for handles and menus
- return selected text for copy
- avoid keyboard presentation for selection-only groups
- avoid caret display for non-editable groups

Important note:

UIKit selection still depends on a backing `UITextInput` object for custom-rendered text. That is an implementation detail of the iOS backend, not the public GPUI API.

### Android

Android should use the same GPUI group model but a different backend strategy.

Suggested responsibilities:

- detect long press and drag handles in GPUI/native bridge
- show contextual copy/select-all actions with `ActionMode`
- provide content rect updates with `ActionMode.Callback2`
- use magnifier support where needed for handle dragging
- manage handles without implying editable input

Important note:

Android does not offer a direct equivalent to `UITextInteraction` for arbitrary custom text surfaces. The GPUI public API should therefore be selection-centric, not UIKit-centric.

## Interaction Rules

### Non-Editable Selection Group

A non-editable selection group must:

- support long press to begin selection
- support drag expansion
- support copy
- support select-all if the group allows it
- not show software keyboard
- not show insertion caret

### Editable Input Surface

An editable input surface may:

- use keyboard and IME
- expose insertion caret
- support replacement
- support marked text
- optionally also participate in selection gestures

If an editable control also wants shared document-style selection, it should explicitly opt into both APIs.

## GPUI Pattern Fit

The intended developer experience should look like normal GPUI composition, not manual frame orchestration.

Preferred style:

```rust
selection_container()
    .flex()
    .flex_col()
    .gap_2()
    .child(text(title).selectable())
    .child(text(body).selectable())
```

Avoid making app code manually:

- construct raw selection fragments for simple text
- attach ad hoc `on_prepaint` callbacks
- concatenate copied text itself
- track group offsets itself

Those responsibilities belong inside GPUI.

## Geometry Requirements

For mobile selection to feel correct, fragment geometry must support wrapped text.

`rects_for_range(...)` should return all line rects for the selected range, not one bounding box. This matters for:

- proper selection handles
- menu placement
- loupe/magnifier placement
- visually correct highlight on wrapped lines

This is especially important for markdown paragraphs and terminal lines that wrap.

## Testing Expectations

Any implementation of this protocol should be tested for:

- long press to select in static text
- no keyboard on non-editable selection
- no blinking caret on non-editable selection
- copy output across fragment boundaries
- wrapped multi-line selection geometry
- link tapping in selectable markdown
- terminal keyboard input unaffected by transcript selection
- clearing selection on outside tap
- rotation and relayout with active selection

See also:

- [NATIVE_TEXT_SELECTION_TEST_PLAN.md](/Users/thomasle/projects/zedra/docs/NATIVE_TEXT_SELECTION_TEST_PLAN.md)

## Recommended Rollout

### Phase 1

- add `SelectionGroup`
- add `selection_container()`
- add `.selectable()` to `StyledText` and `InteractiveText`
- add `TextLayoutSelectionFragment`
- add `Window::handle_selectable_text(...)` as the low-level custom-element hook
- keep current editable `handle_input(...)` path unchanged

### Phase 2

- implement iOS non-editable selection backend
- remove markdown from the editable input path
- migrate markdown to `SelectionGroup`

### Phase 3

- migrate terminal transcript selection to `SelectionGroup`
- keep terminal keyboard input on `handle_input(...)`

### Phase 4

- implement Android backend with `ActionMode` and GPUI-managed handles

## Summary

The core design choice is:

- editable text input is one protocol
- selectable text is another protocol

Selection must be modeled at the logical document-group level, not the individual text-element level. That is what makes cross-paragraph markdown selection and future Android support feasible.
