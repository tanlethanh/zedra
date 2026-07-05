# GPUI Custom Render Effect

Design for a generic post-scene render effect seam in GPUI, and for its first
consumer: the Zedra water droplet, a draggable blob that refracts and recolors
the UI beneath it.

Status: implemented on iOS (`feat/gpui-mobile` in `vendor/zed`, `crates/zedra/src/vfx/`).
Android remains deferred; see the Android section.

## Overview

GPUI renders each frame's scene into the drawable, then presents. The seam adds
one step between those two: an app-provided effect object receives the frame's
command buffer and drawable texture and encodes extra GPU work. GPUI stays
neutral — it defines when the effect runs and what it receives, never what it
draws. The effect brings its own shaders and pipelines.

Zedra uses the seam to draw the water droplet. The same seam supports any
full-frame effect: blur, CRT, color grading.

Prior art shaping this design:

- Godot [`CompositorEffect`](https://docs.godotengine.org/en/stable/tutorials/rendering/compositor.html):
  engine-neutral effect objects registered on the compositor; the engine hands
  a render context, the effect records its own passes.
- egui_wgpu [`CallbackTrait`](https://docs.rs/egui-wgpu/latest/egui_wgpu/trait.CallbackTrait.html):
  app callback with device/encoder access and persistent cached resources.
- Bevy [custom post-processing](https://bevy.org/examples/shaders/custom-post-processing/):
  post pass samples the rendered view and writes the output target.

Upstream GPUI has no custom-render hook
([discussion #45996](https://github.com/zed-industries/zed/discussions/45996)),
and gpui-ce has no render-pass work. This seam is net-new in the
`feat/gpui-mobile` fork.

## GPUI core seam

The core carries an opaque object; backends define the real contract.

`vendor/zed`, branch `feat/gpui-mobile`:

- `crates/gpui/src/platform.rs` — add to `PlatformWindow`, next to the existing
  boxed-callback setters (`on_request_frame` at `platform.rs:693`):

  ```rust
  fn set_render_effect(&self, _effect: Option<Box<dyn Any>>) {}
  ```

  The default no-op body leaves macOS, Linux, and every other backend
  untouched.

- `crates/gpui/src/window.rs` — public forwarder:

  ```rust
  impl Window {
      pub fn set_render_effect(&mut self, effect: Option<Box<dyn Any>>) {
          self.platform_window.set_render_effect(effect);
      }
  }
  ```

One effect slot, one insertion point: after the scene, before present. A stage
enum (Godot-style pre/post stages) is additive later if a second insertion
point earns its keep.

## iOS backend contract

New `crates/gpui_ios/src/render_effect.rs`:

```rust
pub trait MetalRenderEffect: 'static {
    fn encode(&mut self, cx: &MetalEffectContext);
}

pub struct MetalEffectContext<'a> {
    pub device: &'a metal::DeviceRef,
    pub command_buffer: &'a metal::CommandBufferRef,
    pub drawable_texture: &'a metal::TextureRef,
    pub viewport_size: Size<DevicePixels>,
}

// Concrete wrapper: Box<dyn Any> cannot downcast trait-to-trait.
pub struct IosRenderEffect(pub Box<dyn MetalRenderEffect>);
```

`encode` is the only method. The effect builds pipelines lazily on first call
and rebuilds size-dependent resources when `viewport_size` changes. This keeps
the seam smaller than the Godot/egui lifecycles; caching is the effect's job.

Wiring:

- `IosWindow::set_render_effect` downcasts the `Box<dyn Any>` to
  `IosRenderEffect` and stores it in the renderer (new
  `effect: Option<Box<dyn MetalRenderEffect>>` field on `MetalRenderer`).
- `MetalRenderer::draw` calls `effect.encode(&ctx)` after `draw_primitives`
  returns (`metal_renderer.rs:428`) and before present (`:447`). Both the
  command buffer and the drawable are live at that point. The effect creates
  its own blit and render encoders on the command buffer.
- The drawable texture is readable: the layer sets `framebuffer_only(false)`
  (`metal_renderer.rs:198`).
- `render_to_image` skips the effect. Screenshots capture the raw scene.

## Zedra droplet effect

Lives in `crates/zedra/src/vfx/`: `mod.rs`, `droplet.rs`, `droplet.metal`
(embedded with `include_str!`, compiled at runtime with
`device.new_library_with_source` on first `encode`).

`DropletEffect` implements `MetalRenderEffect` (iOS-only via `cfg`). Per frame:

1. Read shared `DropletState` (center in pixels, radius, velocity, active).
   Inactive → return. No encoders, no cost.
2. Blit the padded droplet bounding box from `drawable_texture` into a small
   cached grab texture. Recreate the texture only when its size changes.
3. Encode one render pass (`Load` action) drawing a single quad:
   - SDF metaball: main blob plus a trailing blob scaled by velocity — the
     drip. Velocity also stretches the SDF along the motion vector.
   - Refraction: surface normal from the SDF gradient distorts the UV used to
     sample the grab texture.
   - Specular highlight, edge darkening, slight chromatic aberration.

Params flow through an `Arc<Mutex<DropletState>>` shared between the UI element
and the effect object. No per-frame GPUI API traffic.

UI side: an overlay element at the workspace root handles touch drag
(`.on_press`/touch handlers, not `on_click`). Spring physics drives the wobble
through an animation-frame loop. Droplet position is ephemeral gesture state
and stays element-local, like existing drag interactions — it is not
`WorkspaceState` display data.

## Settings toggle

The droplet ships as a user-facing setting, default on.

- `crates/zedra/src/settings.rs`: `read_droplet_enabled()` /
  `set_droplet_enabled()`, copying the `telemetry_enabled` pattern.
- `crates/zedra/src/settings_view.rs`: one toggle row.
- Registration at root-window creation (`crates/zedra/src/app.rs`):

  ```rust
  window.set_render_effect(Some(Box::new(IosRenderEffect(Box::new(
      DropletEffect::new(state),
  )))));
  ```

  Toggle off → `window.set_render_effect(None)`.

No telemetry event; the toggle carries no personal data either way.

## Performance

- No effect registered: one `Option` check per frame in `MetalRenderer::draw`.
  No extra passes, no textures. Renderer output is byte-identical to today.
- Effect registered, droplet inactive: `encode` locks the shared state, sees
  inactive, returns. One mutex lock and a branch per frame.
- Droplet active, per frame:
  - Bbox blit: ~250×250 BGRA ≈ 250 KB on the blit engine — microseconds.
  - The extra render pass with `Load` action is the dominant GPU cost. On
    Apple TBDR it reloads and re-stores the full drawable through tile memory
    (~12 MB each way at 1179×2556). This is the same cost class the renderer
    already pays whenever a scene contains paths — `draw_paths_to_intermediate`
    ends the encoder and reopens with `Load` (`metal_renderer.rs:602-634`) — so
    the budget is known-acceptable.
  - Fragment work is bounded to the droplet quad, not the screen.
  - The grab texture is small, cached, and recreated only on size change.
- The dominant system cost is CPU-side: the animation loop. A wobbling droplet
  forces continuous full redraws where the UI is otherwise on-demand. The
  element must request animation frames only while dragging or while the spring
  is above a settle threshold, and stop when settled. Toggle off removes the
  effect entirely — zero residual cost.

## Blending model

The droplet pass is not a translucent overlay. The fragment shader samples the
rendered scene from the grab texture and outputs an arbitrary function of it.
Refraction chooses where to sample — UV distortion via the SDF-gradient normal.
The color transform chooses what to output: tint, hue rotation, saturation
boost, brightness lift, inversion. Recoloring text under the droplet (glass-blue
tint, magnified and hue-shifted glyphs) needs no extra plumbing.

Fixed-function Metal blend states (multiply, screen, alpha) are also available
on the effect's pipeline, but sample-and-transform is strictly more expressive
and is the default.

Limits:

- The effect sees final composited pixels, not primitives. It recolors whatever
  is under it — glyphs, images, chrome alike. Per-primitive selective effects
  (text only) are a scene-level feature, out of scope for a post-scene seam.
- Full-frame color effects cannot sample the drawable while writing it. Two
  future options: a full-screen grab copy (extra ~12 MB blit), or restructuring
  the frame to render the scene into a sampleable intermediate first. The
  droplet needs neither; the seam precludes neither.

## Android

Deferred. The wgpu surface is created `RENDER_ATTACHMENT`-only
(`wgpu_renderer.rs:429`) and cannot be sampled. The future approach: render the
scene into a full-frame intermediate created with
`RENDER_ATTACHMENT | TEXTURE_BINDING` (the path intermediate at
`wgpu_renderer.rs:1064` is the template), let the effect sample it into the
surface, and port the shader to WGSL. The Android draw path's off-thread
present worker must be respected. The seam mirrors iOS: a `WgpuRenderEffect`
trait in `gpui_wgpu`.

## Implementation phases

1. GPUI seam: `gpui` + `gpui_ios` changes on `vendor/zed` `feat/gpui-mobile`,
   then the submodule pointer bump here.
2. `DropletEffect` + Metal shader in Zedra, driven by hardcoded test state.
3. Draggable overlay element, spring physics, shared state.
4. Settings toggle + registration.
5. Polish: shape tuning, aberration, trail. Android parity as separate work.

## Validation

```sh
cargo check --manifest-path vendor/zed/Cargo.toml -p gpui_ios -p gpui
cargo test -p zedra --features ios-platform
./scripts/run-ios.sh device
```

On device: drag the droplet over terminal and editor content; verify
refraction, shape stretch on fast drags, and no frame drops. With the toggle
off, rendering behavior must be unchanged. Add manual steps to
[MANUAL_TEST.md](./MANUAL_TEST.md): drag, toggle off/on, rotate/resize.
