# Pinch-to-Zoom Design (#106)

## Goal

User pinches the editor or markdown view; content scales like a canvas. Text stays sharp at any zoom factor. Pan around the larger zoomed content. Layout does not reflow.

## Foundation (already landed in tanlethanh/zed#4 + tanlethanh/zedra#128)

- `UIPinchGestureRecognizer` on iOS and `ScaleGestureDetector` on Android dispatch `PlatformInput::Pinch` to GPUI views.
- Single-finger pan/scroll is suppressed while a pinch is in progress.

This is platform-layer only. No view yet consumes `PinchEvent`.

## Architecture choice

Three honest options.

### Option A — Scale-multiplier stack on `Window` (recommended)

Add a parallel stack alongside `element_offset_stack` and `content_mask_stack`:

```rust
struct PaintTransform {
    scale: f32,
    offset: Point<Pixels>, // applied AFTER scale
    pivot: Point<Pixels>,  // scale anchor, in pre-transform logical space
}

// Window
pub(crate) paint_transform_stack: Vec<PaintTransform>,

pub fn with_paint_transform<R>(
    &mut self,
    transform: PaintTransform,
    f: impl FnOnce(&mut Self) -> R,
) -> R;
```

`current_paint_transform()` composes the stack into a single affine `T`.

Paint methods (`paint_quad`, `paint_path`, `paint_underline`, `paint_strikethrough`, `paint_glyph`, `paint_emoji`, `paint_shadows`, `paint_layer`) apply `T` to incoming positions and sizes, and use `paint_scale_factor()` (= `self.scale_factor() * T.scale`) in place of `self.scale_factor()` for glyph atlas keying, corner radii, and stroke snapping.

**Sharp text comes for free**: `RenderGlyphParams.scale_factor` keys the glyph atlas. Bumping it re-rasterizes at the on-screen pixel size, no extra atlas plumbing.

Hit-test integrates by storing `T` on each `Hitbox` at insert time; the dispatch tree applies `T^-1` to the pointer before testing.

**Pros:** smallest realistic diff (~400 LOC); reuses existing scale_factor → atlas plumbing; sharp text; works on iOS Metal + Android wgpu unchanged.
**Cons:** every existing paint method needs a few-line patch; hit-test path needs careful inverse-transform threading.

### Option B — Render-to-texture

`with_offscreen_render(bounds, scale, f)` renders the subtree into a separate GPU texture, then composites that texture scaled into the main scene. Conceptually pure; isolates the transform to the composite step.

**Pros:** zero changes to existing paint methods.
**Cons:** requires render-target infrastructure that mobile GPUI lacks. Roughly equal LOC to add the offscreen pass on both Metal and wgpu. Memory cost per zoomed view. Text sharpness still requires re-rasterizing at the texture scale, so we still touch the atlas key.

### Option C — Per-primitive transformation matrix

Sprite primitives already carry a `transformation: TransformationMatrix` field honored by the vertex shaders. Extend that to Quad/Path/Underline.

**Pros:** smallest renderer-side diff.
**Cons:** geometry scales but text rasterizes at base resolution → blurry text. To fix, must also re-rasterize at effective scale, which means we end up with Option A's atlas work plus shader changes on both renderers.

## Recommendation

**Option A.** Cleanest cost/value ratio; integrates with the existing `with_element_offset`/`with_content_mask` style; sharp text is automatic.

## Hit-test plan (Option A detail)

Each `Hitbox` insertion captures `current_paint_transform()`. At dispatch:

```
fn pointer_in_hitbox(mouse: Point<Pixels>, hitbox: &Hitbox) -> bool {
    let local = hitbox.transform.inverse().apply(mouse);
    hitbox.bounds.contains(local)
}
```

Pointer events that bubble into a transformed view also expose the local coords so `.on_pinch` callbacks can anchor scaling at the pinch focus correctly.

## Scroll geometry under zoom

When `zoom_factor != 1.0`, the zoomed content extends beyond its layout bounds. Single-finger pan inside the transformed subtree must translate `pan_offset` (state on the view), not scroll the underlying `UniformListScrollHandle`.

The view's `.on_scroll_wheel`/pan handler chooses between the two based on `zoom_factor`:

- `zoom_factor == 1.0`: forward to existing scroll handle (current behavior).
- `zoom_factor != 1.0`: translate `pan_offset` clamped to `[0, content_size * (zoom - 1)]` per axis.

## Editor + markdown wiring

```rust
struct EditorView {
    // ...
    zoom_factor: f32,
    pan_offset: Point<f32>,
    pinch_focus: Option<Point<Pixels>>,
}

fn render(&mut self, ...) {
    let t = PaintTransform {
        scale: self.zoom_factor,
        offset: self.pan_offset,
        pivot: self.pinch_focus.unwrap_or(viewport_center),
    };
    div().with_paint_transform(t, |...| { existing tree })
        .on_pinch(...)
}
```

Pinch handler updates `zoom_factor` (clamped to `[0.5, 4.0]`), updates `pan_offset` to keep the pinch focus stationary on screen (canvas-zoom invariant), and `cx.notify()`.

## Phasing

1. **Vendor PR 1**: `Window::with_paint_transform` + paint method patches + hit-test transform threading. Ships with a small `examples/zoom_demo` view in `vendor/zed/crates/gpui/examples/` that pinches a static panel. ~400 LOC.
2. **Zedra PR 1**: `EditorView` + `MarkdownView` consume the new API; pinch handler, pan, focus anchoring, scroll-geometry switch. ~150 LOC. Manual test entry updated.
3. **Polish**: clamp behavior, reset action, telemetry, edge cases (drawer interaction, native selection under zoom).

## Out of scope (initial PRs)

- Zooming `git_diff_view`, `terminal_view`, or any non-editor surface.
- Persisting `zoom_factor` across sessions or per-file.
- Two-finger double-tap reset gesture.
- Programmatic zoom-to-fit actions.
