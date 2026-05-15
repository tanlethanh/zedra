# GPUI Android

## Goal

Android is the secondary mobile platform for Zedra. The current renderer stack is:

```text
Android app (Kotlin) -> gpui_android -> gpui_wgpu -> wgpu/Vulkan
```

The app boots and renders, but physical-device scrolling and general FPS need a focused performance pass. Do not port Bevy or rewrite GPUI Android. The working strategy is to keep the existing GPUI Android architecture, preserve GPU state across Android surface lifecycle events, and collect enough timing data to decide the next patch from device evidence.

## Current Architecture

Root app Android code lives in:

- `android/app/src/main/kotlin/dev/zedra/app/MainActivity.kt`
- `crates/zedra/src/android/entry.rs`
- `crates/zedra/src/android/jni.rs`
- `crates/zedra/src/android/sheet.rs`

Framework Android code lives in the vendored GPUI checkout:

- `vendor/zed/crates/gpui_android/src/android/ffi.rs`
- `vendor/zed/crates/gpui_android/src/android/platform.rs`
- `vendor/zed/crates/gpui_android/src/android/window.rs`
- `vendor/zed/crates/gpui_wgpu/src/wgpu_renderer.rs`

In the current main checkout, Android surface lifecycle, touch, IME, fling, and Choreographer frame callbacks are owned by `gpui_android`, not by Zedra root `AndroidApp` or a root command queue. Do not reintroduce the older root-side `AndroidApp` / `command_queue` model from scratch worktrees.

`crates/zedra/src/android/jni.rs` is app-specific JNI only: sheets, deeplinks, QR scanner, alerts, native selection, native text input, floating button, dictation preview, notifications, and app metadata. Framework-level hot paths stay in `gpui_android`.

## Investigation Summary

The first successful Android revision proved that GPUI can boot and render on Android. The remaining issue is performance under real interaction, especially drawer movement, markdown scroll, code-editor scroll, fling, and background/foreground.

The likely performance risks were:

- Hot-path Java/Rust logging in touch, IME, key, fling, and inset callbacks.
- Rebuilding renderer/device/atlas on Android surface destroy, recreate, or resize.
- Blocking in `Surface::get_current_texture()` or present/acquire while using the default present mode.
- Lack of aggregated frame timing, making it hard to distinguish GPUI layout work, pending task backlog, surface acquire stalls, and GPU draw time.

The current checkout already has most of the renderer-lifecycle shape needed for Android:

- `WgpuSurfaceConfig::preferred_present_mode`
- `WgpuRenderer::new_with_atlas`
- `WgpuRenderer::update_drawable_size`
- `WgpuRenderer::unconfigure_surface`
- `WgpuRenderer::replace_surface`
- Android window code that preserves the renderer across native surface replacement

The follow-up patch added instrumentation and surface capability logging around that existing structure.

## Bevy Comparison

Bevy is useful as a reference, not as an implementation to copy.

Useful Bevy ideas:

- Treat the surface as lifecycle-managed window state.
- Reconfigure or replace the surface when the native window changes.
- Keep renderer/device resources alive when possible.
- Prefer measurement around acquire/present and app-frame scheduling before changing loop policy.

Ideas not applied yet:

- Parking or substantially changing the Choreographer loop. GPUI Android still needs the Java wakeup path to remain reliable before changing loop ownership.
- Hard-defaulting to `Mailbox`. `Fifo` is universal. `Mailbox` is only useful when the device surface reports support.

## Current Strategy

### Preserve Renderer State

On Android surface destroy:

- Keep the GPUI window and `WgpuRenderer`.
- Call `WgpuRenderer::unconfigure_surface()`.
- Drop surface-sized intermediate textures.
- Skip drawing while the surface is unconfigured.

On Android surface recreate:

- Create a new wgpu surface from the new `ANativeWindow`.
- Call `WgpuRenderer::replace_surface(...)`.
- Keep the same device, queue, pipelines, and atlas.

On Android resize:

- Call `WgpuRenderer::update_drawable_size(...)`.
- If the surface is temporarily unconfigured, update the stored size but avoid touching the stale surface.

This is the main Bevy-inspired part of the strategy: replace/reconfigure the surface, not the renderer.

### Present Mode

Android requests `wgpu::PresentMode::Mailbox` as a preference where supported. `gpui_wgpu` checks `surface_caps.present_modes` and falls back to `Fifo`.

The renderer logs:

- adapter name
- backend
- surface size
- format
- selected present mode
- requested present mode
- alpha mode
- supported present modes
- supported alpha modes
- maximum frame latency

Use this log before assuming the device is actually using `Mailbox`.

### Hot-Path Logs

Per-event logging should stay out of touch, IME, key, fling, inset, and keyboard height paths. Lifecycle logs such as surface created/changed/destroyed are fine. If debugging input routing, add temporary targeted logs with a clear prefix and remove them before finishing.

### AndroidPerf Logs

Use aggregated logs under the `AndroidPerf` target. They should report every 120 samples when there is activity. `samples=120` is the aggregation bucket size, not the FPS; use `sample_window_ms`, `callback_hz`, and `draw_hz` to judge whether callbacks or draw attempts are actually keeping up.

Main-loop instrumentation in `gpui_android::android::ffi` tracks:

- pending task count drained by `AndroidPlatform::process_pending_tasks`
- pending task drain time
- fling processing time
- frame request time
- explicit forced frame count
- fling-active frame count

Kotlin Choreographer instrumentation in `GpuiRuntimeController` tracks:

- posted-delay, frame-time lateness, Choreographer frame-time interval, and
  actual callback-start interval
- time spent reposting the next callback vs time inside the JNI/GPUI frame call
- display refresh/mode reported to the app process for the same callback

GPUI frame instrumentation in `gpui::window` tracks:

- dirty, present-only, idle, throttled, and forced frame counts
- total request-frame callback time
- next-frame callback time
- CPU time spent in `window.draw(cx)`
- platform present time and complete-frame time
- `draw_roots` root/deferred prepaint and paint time, scene items, hitboxes,
  selection fragments, deferred draw counts, and same-frame max-total phase
  decomposition
- `draw_roots stats` per-draw distributions for total draw-roots time, measured
  phase sum, unaccounted overhead, prepaint/paint phases, and nested text
  phases. These stats are computed from the actual draw samples in the bucket,
  not from multiplying an average by a call count.
- `draw_roots sample_row` metric-by-sample rows. Each `s00`..`s29` column is
  one draw cycle, and rows include top-level draw parts, accumulated text
  durations in that same draw, and text-run counts such as `text_runs`,
  `line_layout_runs`, and `backend_layout_runs`.
- `gpui frame slow`, `gpui window_draw slow`, `gpui draw_roots slow`, and
  `renderer slow` immediate rows. These are sparse thresholded rows used to
  match a single visible jank frame to the exact nested draw or renderer phase,
  instead of inferring from a later aggregate bucket.
- `draw_roots sample_row` text paint and glyph backend rows, including
  `text_paint_line_ms`, `text_paint_line_glyph_ms`, `paint_glyph_ms`,
  `paint_glyph_raster_bounds_ms`, `paint_glyph_rasterize_ms`,
  `paint_glyph_atlas_ms`, and glyph counts/atlas hit counts for the same draw.
  These rows are used to test whether Android text cost is in GPUI layout,
  cosmic/swash raster-bounds work, atlas misses, or scene insertion.
- `draw_roots sample_row` top-source rows for root/deferred prepaint and paint,
  such as `root_prepaint_top_source_ms`, `root_prepaint_top_source`,
  `root_prepaint_top_kind`, and `root_prepaint_top_source_samples`. These point
  the slow phase at the element source location and whether the exclusive cost
  came from root layout, element prepaint, or element paint in the same draw
  cycle.
- `draw_roots sample_row` layout-subphase rows for root/deferred prepaint,
  including `*_layout_total_ms`, `*_element_prepaint_ms`,
  `*_layout_request_ms`, `*_layout_compute_ms`, and `*_layout_bounds_ms`.
  These rows split the same draw cycle into GPUI request-layout recursion,
  Taffy compute, and layout-bounds lookup/application.
- `draw_roots sample_row` Taffy measurement source rows, including
  `taffy_measure_top1_source`, `taffy_measure_top1_source_calls`,
  `taffy_measure_top1_source_ms`, and the same top-2 rows. Text elements pass
  their original element source into the measured-layout node, so these rows
  point at concrete app text callsites rather than only the generic GPUI text
  measurement function.
- `draw_roots`-correlated text layout totals, so slow-frame buckets show how
  many text layout calls, relayouts, cache misses, and total text/shape time
  happened inside the same frame bucket instead of only in independent text
  buckets
- `draw_roots`-correlated text-stack totals for `shape_text`, GPUI line-layout
  cache work, and platform backend `layout_line` calls. Use these to separate
  normal repeated text measurement from Android-specific backend shaping cost.
  The line-layout totals also split current-frame cache lookup,
  previous-frame lookup, near-miss scanning, text materialization, cache
  insertion, wrap-boundary computation, and force-width adjustment so a large
  `draw_roots` bucket can be mapped to the specific inner cache or text phase.

GPUI list and text instrumentation tracks:

- Android drawer animation-element frames for `drawer-panel-snap` and
  `drawer-backdrop-snap`, including elapsed animation time, raw/clamped/eased
  delta, done state, and whether the animation requested another frame. This
  separates the visual animation timebase from the drawer snap commit task.
- virtual-list prepaint, layout, and paint time
- uniform-list max-frame decomposition across render, layout, child prepaint,
  visible item count, and slowest item layout/prepaint
- virtual-list layout vs child prepaint time inside `prepaint_items`
- virtual-list max-frame decomposition across measured/cached/visible item
  counts, render-item time, total item-layout time, and slowest item layout
- visible, measured, cached, backfilled, leading-overdraw, and focused list item counts
- slowest per-item `layout_as_root` time and item index
- generic element root layout split into request-layout, Taffy compute-layout, and layout-bounds lookup
- generic element paint timing, including hottest single element, aggregate
  elapsed by source, aggregate elapsed by known source, and aggregate scene
  item count by source
- deferred draw prepaint/paint counts, reuse counts, round count, and slowest deferred draw
- text element layout cache hits vs relayouts, shaped line counts, text length, and wrapping/truncation counts
- text element layout source attribution for the slowest relayout and shape operation
- text element layout identity attribution for Android, including distinct
  layout-instance count, distinct text-content count, most frequently measured
  text layout, and highest-elapsed text layout
- text layout paint timing split into background and foreground line paint,
  with source attribution for the hottest text element
- text element measurement mode, including normal vs nowrap whitespace,
  known/available width, wrap/truncate width use, per-measured-node callback
  index, cache miss reason, width measurement kind, hottest caller source,
  and per-source split for no-layout vs wrap-width-change misses
- text line-layout cache hits/misses for wrapped, plain, and hashed line layouts
- text shaping and wrap-boundary timing
- text line-layout near-miss attribution, separating cold cache misses from
  same-text misses caused by wrap width, font size, or font-run changes
- text line-wrapper truncation timing, including per-truncation character-width
  calls, width-cache hits/misses, and slowest single-character width lookup
- high-level text `shape_text` split into font-run construction and line-layout
  cache work
- Android cosmic-text backend `layout_line` split into attr construction,
  `ShapeLine` creation, `layout_to_buffer`, and glyph conversion
- Android cosmic-text backend single-character layout counts and timing, so
  truncation or wrapping width probes can be separated from full-line shaping
- Android text backend lock timing, including write-lock wait time, locked
  layout time, font database face count, loaded-font count, and family-cache
  size
- Android text backend font loading timing, including lock wait, load time,
  requested font batches, font database face-count growth, loaded-font count,
  and family-cache size
- Android `gpui_android` currently enables cosmic-text `shape-run-cache` for
  diagnostic comparison against repeated `ShapeLine::new` costs caused by
  GPUI/Taffy text measurement misses.
- text paint-line timing split into font bounding-box lookup and glyph paint
  calls, with glyph and visible-glyph counts
- renderer `sample_row` rows under `AndroidPerf renderer sample_row`, with
  `s000`..`s119` columns for the renderer draws in a bucket. These rows split
  WGPU/Vulkan-side work such as `renderer_acquire_ms`, `renderer_encode_ms`,
  `renderer_submit_ms`, `renderer_present_ms`, sprite/batch counts, and mono
  sprite batching density. The encode rows also split command encoder creation,
  render-pass begin/drop, batch iteration, per-batch draw calls, per-primitive
  draw calls, and instance-buffer byte counts so WGPU CPU encoding can be
  separated from surface present/backpressure.
- GPUI frame `sample_row` rows under `AndroidPerf gpui frame sample_row`, with
  `s000`..`s119` columns for callback samples. These rows show dirty vs
  present-only vs forced frames and split callback, update, window draw,
  present, and complete-frame timing.
- WGPU instance-batch reuse optimization: plain instance batches reuse one
  full instance-buffer bind group and draw from a nonzero first-instance range.
  On Android, monochrome text batches also reuse one texture+instance bind group
  per atlas texture within the frame when the shared instance binding covers the
  batch. Consecutive Android mono batches now also keep the mono pipeline and
  globals bind group active, so only the atlas texture binding changes between
  text batches. Compare high-batch buckets against `avg_encode_ms`,
  `renderer_draw_mono_ms`, `renderer_draw_quads_ms`, `renderer_present_ms`,
  `AndroidPerf renderer mono sprites avg_bind_group_ms`,
  `avg_set_pipeline_ms`, and `avg_set_globals_bind_group_ms`.
- Renderer slow rows include mono fragmentation probes:
  `mono_order_count`, `mono_order_texture_runs`, `mono_texture_switches`,
  `mono_after_non_mono`, `mono_after_mono_texture_switch`, and
  `mono_after_mono_same_texture`. Use these to distinguish text broken by
  draw-order/interleaved primitive barriers from text broken by atlas texture
  switching.

Use `scripts/android-perf-summary.py --draw-roots-matrix <log>` for per-draw
GPUI rows, `scripts/android-perf-summary.py --renderer-matrix <log>` for
per-renderer rows, and `scripts/android-perf-summary.py --frame-matrix <log>`
for frame callback rows. Pass `--matrix-all` to print all buckets instead of
only the latest bucket.
- per-glyph paint timing split into raster-bounds lookup, sprite-atlas
  lookup/insert, scene insertion, atlas hit/miss count, render mode, and
  subpixel variant distribution
- Android swash glyph image rendering timing, including image content kind,
  grayscale/subpixel/emoji counts, glyph pixel count, and rendered byte size
- WGPU sprite-atlas lookup timing, including hit/miss count, miss build time,
  allocation time, queued-upload bytes, pending-upload count, cached-tile count,
  and texture kind
- WGPU atlas upload flush timing, including per-frame upload count, byte count,
  max single upload, and total flush time
- Taffy measured-node callback source attribution for the slowest measured layout
  and same-compute max-total measurement count/time
- Taffy measured-node source distribution, including distinct measured sources,
  hottest source by call count, hottest source by elapsed time, and the
  definite/min/max constraint shape for measured callbacks
- `draw_roots`-correlated Taffy totals, including `taffy_compute_ms`,
  `taffy_measure_ms`, `taffy_non_measure_ms`, `taffy_compute_calls`,
  `taffy_measure_calls`, repeated root-compute count, and root child count.
  Use these with the layout-subphase rows to tell whether slow prepaint is
  Taffy traversal itself or measured-node callbacks such as text.

Zedra markdown instrumentation tracks:

- markdown preview render rate and top-level block count
- visible markdown row materialization time
- row kind counts for paragraph, heading, block quote, list, code, table, HTML, rule, spacer, and empty rows
- nested block count per row so large list/quote rows are visible in the aggregate

Zedra code-editor instrumentation tracks:

- editor render rate, line count, bottom spacer count, and line-cache rebuild time
- visible row callback time and row/spacer count per callback

Zedra drawer instrumentation tracks:

- `DrawerHost` render rate, overlay frames, drag frames, snap-animation frames, offset ratio, and host render time
- `WorkspaceDrawer` render rate, active drawer tab/mode, and drawer content render time

Zedra benchmark instrumentation tracks:

- active benchmark case markers with content type, row count, and uniform-list flag
- benchmark view render rate and render time, with the slowest bucket annotated
  by benchmark id, content type, row count, and uniform-list flag

Terminal instrumentation in `zedra-terminal` tracks when the terminal view is part of the repro:

- terminal render snapshot time, including content cloning and visible-link detection
- terminal paint layout time and visible cell/link/run counts
- text shaping/paint time for batched terminal runs
- background rectangle, underline, cursor, deferred bookkeeping, and bounds reconcile time

Renderer instrumentation in `gpui_wgpu::WgpuRenderer::draw` tracks:

- sample window time and draw attempt rate
- atlas `before_frame` time, which includes queued sprite-atlas upload flushing
- surface acquire time
- total draw time
- presented vs skipped frames
- last draw status such as `presented`, `unconfigured`, `suboptimal`, `lost_or_outdated`, `timeout_or_occluded`, `validation`, or `instance_buffer_full`

## 2026-05-09 Glyph Cache Finding

The current confirmed root cause for the 10k text benchmark jank is cold glyph
atlas miss work during GPUI root paint.

Captured logs:

- `/tmp/zedra-android-logcat-20260509-205102.log`
- `/tmp/zedra-android-logcat-20260509-205102-androidperf.log`
- `/tmp/zedra-androidperf-summary-20260509-205102.txt`
- `/tmp/zedra-gfxinfo-20260509-205102.txt`

Repro case:

- Settings -> Developer -> `Text / 10000 / Uniform List`
- Randomized text rows with random text style/color.

Key bucket:

```text
AndroidPerf gpui frame max_total_ms=1016.831 max_window_draw_ms=930.110
AndroidPerf gpui draw_roots max_total_ms=928.680 max_total_root_paint_ms=885.878
AndroidPerf paint glyph samples=4096 atlas_misses=587 avg_total_ms=0.217 avg_raster_bounds_ms=0.109 avg_atlas_ms=0.106 avg_scene_insert_ms=0.001
```

The arithmetic lines up:

```text
4096 glyph paint calls * 0.217ms average = 888.832ms
```

That matches the `885.878ms` root paint stall closely enough to identify the
root bottleneck. The split shows the time is not scene insertion:

- raster-bounds lookup/render work: about half the stall
- atlas lookup/miss/rasterization work: about half the stall
- scene insertion: negligible

Renderer and GPU upload cost are secondary in this trace:

```text
AndroidPerf wgpu atlas flush max_flush_ms=21.821
AndroidPerf renderer max_atlas_before_frame_ms=21.827
AndroidPerf renderer max_draw_ms=84.405
```

Those numbers can still cause visible hitching, but they do not explain the
900ms root-paint stall.

The issue is cold-cache behavior, not steady-state text paint. Later buckets
drop to `atlas_misses=0..3`, `avg_total_ms=0.004..0.006`, and
`wgpu atlas lookup hits=4096 misses=0`.

First fix target:

- first reduce double swash work on a glyph miss. Android computes raster
  bounds via `render_glyph_image`, then atlas miss rasterization calls
  `render_glyph_image` again for the same `RenderGlyphParams`.
- then measure whether one render per new glyph is still too expensive before
  adding any miss budget, prewarm path, or prepare-phase queue.
- do not spend more time on uniform-list rendering, markdown parsing, text
  layout, scene insertion, or surface acquire for this specific stall unless a
  new trace contradicts this finding.

First fix applied:

- `gpui_android::android::text_system` now keeps a bounded, same-params swash
  image handoff cache. `raster_bounds` stores the rendered image, and
  `rasterize_glyph` consumes it for the immediate atlas-miss bitmap path. This
  preserves the existing GPUI paint/atlas contract while removing the confirmed
  duplicate swash render for cold glyph misses.

## 2026-05-09 Text Rasterizer Follow-up

The handoff cache fixed the duplicate swash render on an atlas miss. It did
not make Android glyph generation cheap enough.

Post-fix captured logs:

- `/tmp/zedra-android-logcat-20260509-210137.log`
- `/tmp/zedra-android-logcat-20260509-210137-androidperf.log`
- `/tmp/zedra-androidperf-summary-20260509-210137.txt`
- `/tmp/zedra-gfxinfo-20260509-210137.txt`

Key post-fix buckets:

```text
AndroidPerf paint glyph samples=4096 atlas_misses=587 avg_total_ms=0.116 avg_raster_bounds_ms=0.111 avg_atlas_ms=0.004 avg_scene_insert_ms=0.001
AndroidPerf glyph render image samples=512 avg_pixels=384.7 avg_total_ms=0.593 max_total_ms=2.935
AndroidPerf glyph render image samples=512 avg_pixels=448.7 avg_total_ms=0.807 max_total_ms=1.916
AndroidPerf gpui draw_roots max_total_root_paint_ms=476.862
```

The important shift is:

- `avg_atlas_ms` dropped from about `0.106ms` to about `0.004ms`, so the
  duplicate render inside the atlas miss path is gone.
- `avg_raster_bounds_ms` is still about `0.111ms`, so one swash render per cold
  glyph remains expensive.
- `4096 * 0.116ms = 475.136ms`, which matches the remaining root-paint stall.

That means a queue, budget, or prewarm path can protect frame latency, but it is
not the root fix. The root fix must reduce cold glyph generation cost or replace
the raster backend.

Current backend facts:

- Android GPUI shapes with `cosmic-text`, rasterizes with `swash`, and inserts
  the resulting bitmap into GPUI's sprite atlas:
  `vendor/zed/crates/gpui_android/src/android/text_system.rs`.
- `Window::paint_glyph` synchronously asks for raster bounds, then inserts the
  glyph bitmap into the atlas on a miss:
  `vendor/zed/crates/gpui/src/window.rs`.
- Zedra Android currently keeps `minSdk 21`: `android/build.gradle`.
- The comparable iOS path uses the platform text stack, CoreText/CoreGraphics,
  in `vendor/zed/crates/gpui_ios/src/ios/text_system.rs`.

Candidate paths:

1. FreeType A/B raster backend.
   Keep `cosmic-text` shaping, font matching, glyph IDs, GPUI scene insertion,
   and the existing WGPU atlas. Replace only the grayscale glyph bounds/raster
   backend for non-emoji glyphs, then measure the same AndroidPerf buckets.
   This is the narrowest root-cause experiment.
2. Android native raster backend.
   On API 31+, `android.graphics.fonts.Font#getGlyphBounds` and
   `Canvas#drawGlyphs` can consume glyph IDs. That can preserve
   `cosmic-text` shaping if the Android `Font` object is built from the same
   font bytes and the glyph IDs match. It cannot be the only backend while
   `minSdk` remains 21.
3. GPU/path/SDF text architecture.
   This could reduce CPU rasterization more fundamentally, but it is a larger
   renderer change because GPUI currently paints bitmap glyph sprites into an
   atlas. Treat this as research unless the narrower rasterizer A/B still shows
   unacceptable per-glyph cost.

Research notes:

- Prefer direct `freetype-sys` for the first FreeType A/B. It can consume the
  exact font bytes already loaded for `cosmic-text`, use the shaped glyph ID,
  and avoid fontconfig. `freetype-sys` skips `pkg-config` for Android targets
  and has a bundled-source build path.
- Do not enable GPUI's full desktop `font-kit` feature for Android as the first
  attempt. In the current dependency shape it pulls `zed-font-kit` and then
  `yeslogic-fontconfig-sys` for non-macOS/iOS/Windows targets, which is exactly
  the cross-compile surface to avoid.
- Keep swash as the fallback and emoji path during the A/B. FreeType can handle
  basic bitmap/color cases, but modern color emoji and COLR/SVG details are
  higher regression risk than the normal text path being measured.
- `glyphon` is useful reading, but it is mostly lateral for this root cause:
  it is also `wgpu` + `cosmic-text` + a cached rasterized glyph atlas.
- SDF/MSDF is the closest incremental renderer change that could reduce
  per-size glyph generation for monochrome editor/terminal text while keeping
  GPUI's shaped glyph positions and atlas/sprite model. The risk is small-text
  quality, hinting loss, large CJK/Unicode sets, and color emoji.
- Vello/Slug-style GPU outline rendering is the bigger architectural direction.
  It removes precomputed bitmap/field atlas generation, but requires new
  renderer primitives or passes rather than a small Android text-system patch.

Implementation status:

- `gpui_android` now depends directly on `freetype-sys = "0.20.1"`.
- `gpui_android::android::text_system` creates a FreeType library and a
  memory-backed FreeType face for each loaded `cosmic-text` font.
- FreeType is used only for non-emoji grayscale glyphs. Swash remains the
  fallback and the emoji/color path.
- FreeType glyph output is still inserted through the existing GPUI sprite
  atlas contract. Shaping, fallback selection, scene insertion, and WGPU atlas
  upload are unchanged.
- `AndroidPerf glyph render image` now includes `swash=` and `freetype=`
  counts so the device logs show which backend produced the sampled glyphs.
- Validation passed after this change:
  `cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 check --manifest-path vendor/zed/Cargo.toml -p gpui_android`,
  `./scripts/build-android.sh --target=arm64-v8a`, and
  `cd android && ./gradlew assembleDebug`.

Research questions for the next patch:

- Can FreeType build and link for `cargo ndk` without pulling fontconfig?
  `zed-font-kit` has separate `loader-freetype` and `source-fontconfig`
  features; enabling GPUI's full `font-kit` feature currently pulls fontconfig,
  which is not the Android path we want.
- Can `cosmic_text::Font::data()` provide the exact font bytes needed to create
  a FreeType face for the selected fallback font?
- Can FreeType compute bounds from `FT_Load_Glyph` metrics before
  `FT_Render_Glyph`, avoiding a bitmap render just to answer
  `glyph_raster_bounds`?
- Does FreeType grayscale/hinting output match current visual quality enough on
  Android? Measure sharpness and placement, not only time.
- How should emoji and color glyphs behave? A likely first pass is FreeType for
  normal mask glyphs and swash/native fallback for color emoji.
- Does subpixel positioning matter? Android currently logs only one y variant
  and grayscale text, so the first A/B can start with the existing mask path.

Reading list:

- Android `Canvas#drawGlyphs`:
  <https://developer.android.com/reference/android/graphics/Canvas#drawGlyphs(int%5B%5D,%20int,%20float%5B%5D,%20int,%20int,%20android.graphics.fonts.Font,%20android.graphics.Paint)>
- Android `Font#getGlyphBounds`:
  <https://developer.android.com/reference/android/graphics/fonts/Font#getGlyphBounds(int,%20android.graphics.Paint,%20android.graphics.RectF)>
- Android JNI performance guidance:
  <https://developer.android.com/training/articles/perf-jni>
- FreeType glyph loading and rendering:
  <https://freetype.org/freetype2/docs/reference/ft2-glyph_retrieval.html>
- FreeType memory-backed faces:
  <https://freetype.org/freetype2/docs/reference/ft2-face_creation.html>
- FreeType LCD/subpixel rendering:
  <https://freetype.org/freetype2/docs/reference/ft2-lcd_rendering.html>
- FreeType tutorial, glyph images and metrics:
  <https://freetype.org/freetype2/docs/tutorial/step2.html>
- `freetype-sys` build script:
  <https://docs.rs/crate/freetype-sys/latest/source/build.rs>
- Swash scaling/rasterization docs:
  <https://docs.rs/swash/latest/swash/scale/index.html>
- Glyphon, a wgpu/cosmic-text text renderer:
  <https://docs.rs/glyphon/latest/glyphon/>
- Vello, a GPU-compute 2D renderer:
  <https://skia.googlesource.com/external/github.com/linebender/vello/+/refs/heads/main/README.md>
- MSDF generator and atlas references:
  <https://github.com/Chlumsky/msdfgen>
- Valve's signed-distance-field paper:
  <https://steamcdn-a.akamaihd.net/apps/valve/2007/SIGGRAPH2007_AlphaTestedMagnification.pdf>
- Slug GPU outline text paper:
  <https://jcgt.org/published/0006/02/02/paper.pdf>
- Slug library overview:
  <https://sluglibrary.com/>
- Android hardware-accelerated Canvas model:
  <https://developer.android.com/topic/performance/hardware-accel>
- Flutter Impeller renderer notes:
  <https://docs.flutter.dev/perf/impeller>
- Parley rich text layout stack:
  <https://github.com/linebender/parley>

## Summarizing Logs

Use `scripts/android-perf-summary.py` to collapse raw log output into
per-instrument bucket counts and numeric field statistics. The helper reports
`count`, `min`, `p25`, `avg`, `mean`, `p50`, `p75`, and `max` for every
selected numeric field, plus the slowest raw buckets for the preferred timing
field. It parses both `AndroidPerf` and `IosPerf` so Android and iOS traces can
be compared with the same output shape.

From an existing AndroidPerf extract:

```sh
scripts/android-perf-summary.py /tmp/zedra-android-logcat-androidperf.log --top 3
```

From raw logcat:

```sh
adb logcat -d -v threadtime | scripts/android-perf-summary.py -
```

To print the latest `draw_roots sample_row` bucket as a matrix where each
column is one draw cycle. The matrix includes top-source rows for root/deferred
prepaint and paint, so the source owning the slow phase can be compared against
text and scene metrics in the same sample column:

```sh
scripts/android-perf-summary.py /tmp/zedra-android-logcat-androidperf.log --draw-roots-matrix
```

To print GPUI frame callback samples and see whether present cost is happening
during dirty, present-only, forced, idle, or throttled callbacks:

```sh
scripts/android-perf-summary.py /tmp/zedra-android-logcat-androidperf.log --frame-matrix
```

From iOS `IosPerf` logs:

```sh
./scripts/log-ios.sh --filter IosPerf | tee /tmp/zedra-iosperf.log
scripts/android-perf-summary.py --prefix IosPerf /tmp/zedra-iosperf.log --top 3
```

Use `--all-fields` when the default timing/count field filter hides a field you
need for a one-off investigation.

## iOS Comparison Buckets

iOS now emits `IosPerf` buckets for the shared GPUI hot paths that Android logs
as `AndroidPerf`:

- `gpui frame`, `gpui draw`, and `gpui draw_roots`
- `element root layout` and `element paint`
- `taffy compute`
- `list`, `uniform list`, and deferred draw buckets
- `text element layout`, `text layout paint`, `text paint line`,
  `text shape_text`, `text line layout`, `text line wrapper truncate`, and
  `paint glyph`
- iOS-specific `renderer` timing from the Metal renderer

The Metal `renderer` bucket measures CPU-side drawable acquire, primitive
encoding, and present/commit scheduling. It does not include Android WGPU
surface status fields or GPU completion time, so compare it against Android's
renderer encode/draw directionally, not as identical backend telemetry.

## 2026-05-10 Root Dev Profile Finding

The current Android repro builds through the root Zedra workspace. That matters
because the root workspace excludes `vendor/zed`, so profile overrides in
`vendor/zed/Cargo.toml` do not apply to Android app builds.

Zed's vendored workspace optimizes `taffy` in dev builds. Before the root
profile update, `scripts/run-android.sh` defaulted to the Android debug APK and
also passed `--debug` to `scripts/build-android.sh`, so hot layout/text
dependencies were built with normal Rust dev-codegen settings while the app
kept debug assertions and developer benchmark views.

This matches the 2026-05-10 Android logs:

- `AndroidPerf taffy compute` showed very high total layout time.
- The measured text callback time was much smaller than total Taffy time in the
  worst buckets, so the gap was inside pure layout work, not only text shaping.
- iOS simulator logs using the same shared GPUI buckets showed low Taffy and
  text-layout cost. That comparison is directional only because simulator and
  Android physical-device hardware differ, but it confirms the shared probes are
  able to show the same bucket names at much lower cost.

Root `Cargo.toml` now keeps hot GPUI layout/text dependencies optimized in the
dev profile:

- `taffy`
- `cosmic-text`, `swash`, `fontdb`, `harfrust`, `rustybuzz`, `skrifa`,
  `ttf-parser`
- Unicode line/shaping support crates used by that text stack

This keeps `debug_assertions` enabled for developer-only benchmark views while
removing a major debug-build performance amplifier. The next Android install
should be a normal debug install, then the key comparison is whether
`AndroidPerf taffy compute`, `text backend layout_line`, and `gpui draw_roots`
drop without changing app behavior.

## 2026-05-10 WGPU Batch Encoding Finding

Per-batch WGPU bind group churn was a real Android backend bottleneck, separate
from the text rasterizer cost.

Relevant post-optimization logs:

- `/tmp/zedra-android-logcat-20260510-175644-pid18254-androidperf.log`
- `/tmp/zedra-android-summary-20260510-175644-pid18254.txt`
- `/tmp/zedra-android-renderer-matrix-20260510-175644-pid18254.tsv`
- `/tmp/zedra-android-draw-matrix-20260510-175644-pid18254.tsv`

Applied optimization:

- Plain instance batches share one buffer-wide instance bind group and use
  `first_instance` to select the batch range.
- Android monochrome text batches share one texture+instance bind group per
  atlas texture within a frame when the batch is covered by the shared instance
  binding.
- Consecutive Android monochrome text batches keep the same mono pipeline and
  globals bind group bound; the renderer only changes the texture+instance bind
  group between batches.
- Batches fall back to the old per-batch bind group path when the offset cannot
  be represented as a typed instance index or the batch exceeds the shared
  binding range.

Validated result:

- Before: `avg_batches=61`, `max_batches=174`, `avg_encode_ms=6.782`,
  `max_encode_ms=24.037`.
- After: the worst high-batch bucket was `avg_batches=21.9`,
  `max_batches=105`, `avg_encode_ms=1.390`, `max_encode_ms=5.814`.
- The remaining high-encode samples are still dominated by monochrome text
  batch count. One sample had `renderer_encode_ms=5.814`,
  `renderer_mono_batches=99`, and `renderer_mono_sprites=1022`.

Remaining signals:

- Some slow renderer samples are now present/backpressure dominated. Example:
  `renderer_draw_ms=18.323`, `renderer_encode_ms=1.521`, and
  `renderer_present_ms=13.316`.
- GPUI draw-root spikes still exist above the renderer. One sample had
  `draw_total_ms=35.907`, `root_prepaint_ms=22.818`,
  `taffy_compute_ms=10.123`, `text_layout_ms=4.623`, and
  `text_runs=1261`.
- A text-backend layout spike remains possible:
  `backend_layout_ms=6.444`, `shape_text_ms=7.736`,
  `line_layout_ms=7.046`.

Next useful probes:

- Separate present/backpressure from CPU encoding in the Android WGPU surface
  path.
- Reduce or explain monochrome text tiny-batch fragmentation.
- Keep draw-root sample rows tied to a single render cycle so GPUI layout, text
  layout, renderer encode, and present timing can be compared directly.
- Check `mono_order_texture_runs` and `mono_after_non_mono` in renderer slow
  rows before changing batching semantics.

## 2026-05-11 Android Monochrome Atlas Texture Churn Finding

The next renderer probes showed that remaining mono text encode cost was not
primarily from quads or other primitives interrupting text order. Slow renderer
rows had `mono_after_non_mono=2..4`, but `mono_after_mono_texture_switch` was
often `56..74` and `mono_after_mono_same_texture=0`. A single code-editor draw
could have two mono atlas textures and `mono_order_texture_runs=75..96`.

That means many text orders are being split because glyphs for the same visible
rows live on two monochrome atlas textures. The WGPU atlas previously searched
existing textures from newest to oldest, so once a second monochrome atlas
existed, newly seen glyphs tended to land there while common glyphs stayed in
the first atlas.

Applied optimization:

- Android monochrome atlas allocation now searches oldest to newest, keeping new
  glyphs in the primary mono atlas while it still has room.
- Android monochrome atlases use a `2048x2048` default size, clamped by the
  device max texture size. R8 mono atlas memory grows from about 1 MB to about
  4 MB per atlas, which is a small tradeoff compared with repeated CPU-side WGPU
  texture bind churn during text-heavy frames.

The expected validation signal is fewer mono atlas textures in normal editor
use, lower `mono_order_texture_runs`, and fewer
`mono_after_mono_texture_switch` counts in `AndroidPerf renderer slow` rows.

## 2026-05-10 Android Present-Only Frame Pacing Finding

Later immediate slow rows showed a second Android-backend bottleneck: GPUI could
submit full WGPU presents for unchanged scene content after high-rate input.

Relevant logs:

- `/tmp/zedra-android-logcat-20260510-183908-pid24696-androidperf.log`
- `/tmp/zedra-android-summary-20260510-183908-pid24696.txt`
- `/tmp/zedra-android-frame-matrix-20260510-183908-pid24696.tsv`
- `/tmp/zedra-android-renderer-matrix-20260510-183908-pid24696.tsv`

Finding:

- Kotlin drives `gpuiRequestFrame()` from `Choreographer` every vsync.
- GPUI's shared high-rate input tracker sustained presentation for one second
  after >=60 Hz input, even if the window was not dirty.
- On Android, that meant repeated `present_only` frames with `window_draw=0`,
  while WGPU/SurfaceView `present()` still blocked for roughly 13-18 ms on the
  physical Mali/Vulkan device.
- These present-only stalls happened before and around file-open/editor
  transitions, so they could consume the UI thread independently of editor text
  layout or row construction.

Applied optimization:

- Android now disables only the high-rate-input sustain-presentation path.
- Dirty frames, forced redraws, explicit `require_presentation`, and real
  content-changing scroll/fling frames still present normally.
- The expected validation signal is that `gpui frame slow kind=present_only`
  rows largely disappear, while remaining jank should concentrate in dirty
  `draw_roots`, renderer encode, or renderer present rows.

## 2026-05-11 Initial Dirty Draw Investigation

After disabling Android present-only sustain, slow samples are expected dirty
draws. Treat `kind=dirty` as the trigger, not the diagnosis.

Current interpretation:

- File open and editor replacement legitimately require a cold draw.
- A code file naturally creates many text runs and many monochrome text batches.
- The remaining question is why that first required draw is too expensive on
  Android compared with iOS.

Active probe:

- `gpui draw_roots slow` is paired with `gpui draw_roots layout_slow`.
- The companion row splits the same draw into root/deferred prepaint layout,
  element prepaint, layout request, Taffy compute, layout-bounds lookup, Taffy
  text-measure callbacks, and their top source locations.
- `renderer slow` now includes per-sample monochrome tiny-batch counts so encode
  cost can be separated from expected text-run count.
- Periodic aggregate bucket logs are currently gated off during file-open
  repros. They can perturb the UI thread when several large summaries flush on
  the same frame; rely on immediate slow rows for clean cold-draw samples.

## 2026-05-11 File-Open Query Compilation Fix

The repeated file-open jank had one confirmed app-side cause before the next
GPUI backend pass: `EditorView::set_content` synchronously created a Rust
highlighter, and that compiled the tree-sitter highlight query on the UI thread.

Pre-fix evidence:

- Logs:
  - `/tmp/zedra-android-logcat-20260511-133948-pid25726-androidperf.log`
  - `/tmp/zedra-android-summary-20260511-133948-pid25726.txt`
- Rust files spent roughly `159-178ms` in `editor set_content`, almost all in
  `highlighter_ms`.
- `Highlighter::new` showed `query_ms` around `159-176ms` for Rust, including a
  tiny `427` byte / `24` line Rust file. The cost was therefore query
  compilation, not file size, buffer replacement, or editor cache rebuild.
- Plain text files stayed near `0.1ms` in `editor set_content`, confirming the
  tree-sitter query boundary.

Zed reference:

- Upstream Zed stores compiled `tree_sitter::Query` values on
  `language_core::Grammar`, then holds those grammars behind cached
  `Arc<Language>` values in `LanguageRegistry`.
- `SyntaxMap` and buffer highlighting borrow the already-compiled grammar query
  rather than recompiling queries for each file open.
- Zedra's simplified mobile editor had been treating `Highlighter` as
  parser/query/tree per file open, which was the wrong ownership shape for
  Android.

Applied optimization:

- `crates/zedra/src/editor/syntax_highlighter.rs` now keeps a per-language
  `Arc<Query>` cache and gives each highlighter a parser/tree plus a shared
  compiled query.
- `Highlighter::pending_for_filename(...)` creates a parser-free and query-free
  placeholder for UI-thread file replacement.
- `crates/zedra/src/editor/code_editor.rs` now installs that pending highlighter
  from `EditorView::set_content`. The background `ParsedEditorSyntax::build`
  task still creates/parses the real highlighter and `apply_parsed_syntax`
  installs the parsed result.
- Do not reintroduce `Highlighter::from_filename(...)` or `Query::new` on the
  UI-thread `set_content` path. The pending highlighter exists specifically to
  keep file-open/drawer transitions free of tree-sitter query compilation.

Post-fix evidence:

- Logs:
  - `/tmp/zedra-android-logcat-20260511-135611-pid28015-androidperf.log`
  - `/tmp/zedra-android-summary-20260511-135611-pid28015.txt`
  - `/tmp/zedra-gfxinfo-20260511-135611-pid28015.txt`
- `editor set_content`: avg `4.409ms`, p75 `5.621ms`, max `12.805ms`.
- The first Rust `Query::new` still cost `197.898ms`, but it ran on a
  background thread. Later Rust highlighter creation hit the cache:
  `query_ms=0.013-0.077ms`.
- `syntax_apply_apply_done` stayed tiny, usually `0.024-0.069ms`, so applying
  parsed syntax back to the editor is not the UI-thread stall.
- Background syntax parse/highlight can still take tens to hundreds of
  milliseconds for large files. That work is no longer the drawer-close UI
  blocker, but it can trigger the next full syntax draw when it completes.

Remaining bottleneck after this fix:

- Slow dirty frames now point at GPUI editor row layout/prepaint, especially
  after syntax apply or full editor redraw.
- Worst post-fix samples:
  - `gpui frame slow total_ms=101.082`, `window_draw_ms=74.443`.
  - `gpui draw_roots slow total_ms=70.851`, `root_prepaint_ms=58.621`,
    `root_prepaint_layout_ms=42.997`.
  - `taffy_compute_ms=21.251`, `taffy_measure_ms=10.964`.
  - top source: `crates/zedra/src/editor/code_editor.rs:447`, the per-row
    `div()` in the code editor uniform list.
- Renderer work during those same samples is lower and not the primary cause:
  `renderer draw_ms=16.686`, with mono draw around `1.756ms`.

Current conclusion:

- The old file-open `Query::new` UI-thread stall is fixed.
- The next investigation boundary is GPUI/editor row layout and prepaint on
  Android, not tree-sitter query compilation.

## 2026-05-11 Periodic HostInfo Redraw Finding

After the query-compilation fix, the app still produced slow dirty frames even
when no new file was opened. The repeated samples lined up with the host stats
period, not with editor content replacement.

Evidence:

- Host info is sampled by the host every `5s`.
- Android logs after the last file open still showed `gpui draw_roots
  layout_slow` roughly every `5s`.
- The dirty source was `workspace.rs` handling host info, followed by broad
  `WorkspaceState` observers such as the file explorer and workspace content.
- `cx.notify()` only invalidates views tracking that entity, but `WorkspaceState`
  is a high-level display entity. Marking it dirty can invalidate the
  editor/explorer/drawer path and defeat nested prepaint reuse for that frame.

Applied optimization:

- `HostInfoState` now owns the transient host resource snapshot.
- The host-info listener updates `HostInfoState`, not `WorkspaceState`.
- The Session panel observes `HostInfoState` directly, so the stats row still
  updates while editor, explorer, and workspace content avoid the periodic broad
  redraw.

Expected validation:

- The recurring `5s` dirty `draw_roots/layout_slow` samples should disappear or
  shrink when the visible Session panel is not the active source of work.
- This is not expected to solve all Android jank. It removes a confirmed
  periodic redraw amplifier; file-open cold draws, Android text/layout backend
  cost, and renderer batching/present timing remain separate suspects.

## 2026-05-11 Two-Track Performance Strategy

Keep the investigation split into two active tracks.

Track 1: app and GPUI scene cost.

- Optimize the editor, list-heavy panels, file explorer, and git diff views so
  they do not create avoidable layout or text-measure work.
- Treat code editor, file explorer, git diff, markdown, and benchmark views as
  repro surfaces for the same GPUI list/text/layout pipeline.
- The current evidence after the HostInfo split points at virtualized editor
  rows and text elements: repeated dirty draws still show `code_editor.rs` row
  layout/prepaint and styled-text measurement/paint as the largest required-draw
  costs.

Track 2: Android rendering backend smoothness.

- Keep probing `gpui_android`, `gpui_wgpu`, Android text/glyph rasterization,
  atlas upload, WGPU/Vulkan present/backpressure, and monochrome text batching.
- A GPU renderer should feel smoother than a WebView once scene construction is
  cheap enough. If it does not, the backend path still has work to do.
- Use cached-scene and text-heavy benchmark views to separate backend smoothness
  from app/editor row construction.

The latest validation signal:

- No periodic `workspace.rs` HostInfo dirty source remained after the split.
- After the last editor slow frame in the 2026-05-11 `14:55` log, the main loop
  stayed near `61Hz` with tiny frame-request time.
- Remaining slow rows were file-open/editor/drawer dirty draws, not idle
  periodic telemetry redraws.

## 2026-05-11 Drawer Present-Path Probe

The drawer toggle animation is the current minimal repro for Android backend
smoothness. It keeps app-level work small enough that slow frames are easier to
attribute below GPUI.

Latest captured logs:

- `/tmp/zedra-android-logcat-20260511-172734.log`
- `/tmp/zedra-android-logcat-20260511-172734-androidperf.log`
- `/tmp/zedra-gfxinfo-20260511-172734.txt`
- `/tmp/zedra-display-20260511-172734.txt`
- `/tmp/zedra-surfaceflinger-20260511-172734.txt`

Signal from that capture:

- Drawer open target `160ms` stretched to roughly `278..355ms`.
- Drawer close target `100ms` stretched to roughly `234..298ms`.
- Drawer render and action-handler costs were tiny; this was not a drawer
  content render bottleneck.
- GPUI text/layout was not the dominant cost in this repro. `draw_roots
  text_layout` stayed around `1ms` average, with only small spikes.
- WGPU CPU work before present was small: acquire, encode, finish, and submit
  were usually a few milliseconds total.
- The dominant renderer phase was `frame.present()`: `present_ms` averaged about
  `15..16ms`, with p75 around `20ms` and max around `30ms`.
- App-side Choreographer logs reported `120Hz`, while SurfaceFlinger dumpsys at
  capture time reported an active `60Hz` composition mode. Treat this as an
  active suspect, not a conclusion, until per-layer latency confirms it.

Use `scripts/android-render-capture.sh` to capture this next boundary:

```sh
scripts/android-render-capture.sh prepare
# manually toggle the drawer several times
scripts/android-render-capture.sh pull
```

The helper saves logcat, AndroidPerf summaries/matrices, `gfxinfo`,
`dumpsys display`, full SurfaceFlinger state, and `SurfaceFlinger --latency`
for matching app layers. The layer latency summary is the key next artifact for
checking whether `frame.present()` is blocking because the app is pacing at
120Hz while the SurfaceView/BLAST/SurfaceFlinger path presents at 60Hz.

The next A/B should use the debug-only render pacing controls. Apply one mode,
rerun the same drawer toggle capture, then compare `renderer present_ms`,
`gpui frame present_ms`, Choreographer `callback_hz`, and SurfaceFlinger layer
latency:

```sh
scripts/android-render-pacing.sh --frame-rate none --present-mode mailbox
scripts/android-render-capture.sh prepare
# manually toggle the drawer several times
scripts/android-render-capture.sh pull

scripts/android-render-pacing.sh --frame-rate 60 --present-mode mailbox
scripts/android-render-capture.sh prepare
# manually toggle the drawer several times
scripts/android-render-capture.sh pull

scripts/android-render-pacing.sh --frame-rate 120 --present-mode mailbox
scripts/android-render-capture.sh prepare
# manually toggle the drawer several times
scripts/android-render-capture.sh pull

scripts/android-render-pacing.sh --frame-rate 60 --present-mode fifo
scripts/android-render-capture.sh prepare
# manually toggle the drawer several times
scripts/android-render-capture.sh pull

scripts/android-render-pacing.sh --frame-rate 120 --present-mode fifo --frame-driver end
scripts/android-render-capture.sh prepare
# manually toggle the drawer several times
scripts/android-render-capture.sh pull

scripts/android-render-pacing.sh --frame-rate 120 --present-mode fifo --frame-driver start --frame-latency 3
scripts/android-render-capture.sh prepare
# manually toggle the drawer several times
scripts/android-render-capture.sh pull

scripts/android-render-pacing.sh --frame-rate 120 --frame-compat fixed --frame-change always --present-mode fifo --frame-driver start --frame-latency 3
scripts/android-render-capture.sh prepare
# manually toggle the drawer several times
scripts/android-render-capture.sh pull

scripts/android-render-pacing.sh --present-strategy offload
scripts/android-render-capture.sh prepare
# manually toggle the drawer several times
scripts/android-render-capture.sh pull

scripts/android-render-pacing.sh --present-strategy inline
```

These controls are diagnostic only:

- `--frame-rate` calls Android `Surface.setFrameRate(...)` on the GPUI
  `SurfaceView` in debug builds. `none` clears the SurfaceView vote.
- `--frame-compat` selects the debug `Surface.setFrameRate` compatibility:
  `default` is the UI/game path; `fixed` is the fixed-source/video-style path.
- `--frame-change` selects the debug refresh-rate change strategy:
  `seamless` keeps Android's default behavior; `always` lets Android use a
  non-seamless display-mode change if the platform decides it is worthwhile.
- `--present-mode` updates the WGPU surface present mode in debug Rust builds.
  `default` is the current Android preference, `Mailbox`.
- `--frame-driver` switches the debug Choreographer repost point. `start`
  preserves the current loop and posts the next callback before
  `gpuiRequestFrame(...)`; `end` posts after the GPUI frame returns. Use this
  only as an A/B probe for catch-up callbacks after slow draws.
- `--frame-latency` updates WGPU
  `SurfaceConfiguration::desired_maximum_frame_latency` in debug Rust builds.
  Keep it at the default `2` unless a capture is explicitly testing Vulkan /
  SurfaceFlinger backpressure.
- `--present-strategy` is a backend-only probe for the WGPU handoff point:
  `inline` is the normal path, `offload` moves `SurfaceTexture::present()` to a
  single worker thread and emits `AndroidPerf renderer async_present`, and
  `discard` drops the acquired surface texture after submit. Use it only to
  isolate whether UI-thread jank follows WGPU/Vulkan present blocking.
- Each change emits `AndroidPerf render_pacing_control` / `render_pacing` rows,
  so captures show which pacing mode was active.

Captured A/B results:

- `none + Mailbox`:
  - Drawer snap target `100/160ms` completed by the commit task in
    `218..358ms`, average `282.6ms`.
  - GPUI frame `present_ms` averaged `21.1ms`; renderer `present_ms` averaged
    `16.0ms`.
  - SurfaceFlinger BLAST layer active intervals were unstable: filtered
    `p50=37.0ms`, `p75=63.9ms`.
- `60Hz vote + Mailbox`:
  - SurfaceFlinger intervals improved but remained unstable: filtered
    `p50=25.9ms`, `p75=42.6ms`.
  - Drawer snap commit did not improve: average `303.4ms`.
  - GPUI frame total stayed effectively unchanged at about `35ms`.
- `60Hz vote + Fifo`:
  - Renderer present improved: average `14.5ms -> 11.9ms`, max
    `29.7ms -> 20.9ms`.
  - GPUI frame total improved: average `35.4ms -> 30.0ms`, max
    `51.1ms -> 37.7ms`.
  - Drawer snap commit still did not improve: average `306.5ms`.
- `60Hz vote + Fifo`, later drawer-tap capture:
  - SurfaceFlinger/layer output was still effectively `60fps`, but app-side
    Choreographer continued to report `120Hz`.
  - GPUI frame total improved only modestly versus the prior `120/Fifo`
    capture: p50 `41.7ms -> 34.9ms`, p75 `46.1ms -> 39.7ms`.
  - Renderer present improved only modestly: p50 `13.6ms -> 11.9ms`, p75
    `17.8ms -> 17.1ms`.
  - This rules out `60Hz` as a real fix. A GPU UI backend should not need to
    cap to 60Hz to be smooth, and this A/B still left visible jank.
  - The same run exposed a separate Android text/font cold path:
    `text backend layout_line` hit about `41ms`, with `android_backend_ms`
    around `60ms` in the correlated `draw_roots` row.
- `120Hz vote + Fifo + frame_driver=start + frame_latency=3`:
  - WGPU `desired_maximum_frame_latency=3` did not fix the backend jank.
  - GPUI frame `present_ms` stayed effectively unchanged versus the default
    latency runs: p50 stayed around `18ms` and p75 around `22ms`.
  - Renderer `present_ms` also stayed in the same band: p50 around `13ms`,
    p75 around `17ms`.
  - The bad frames are mixed: many frames spend about `10..15ms` in GPUI
    window drawing plus `18..30ms` in WGPU present, while cold frames can spend
    much more in window drawing.
  - SurfaceFlinger still shows irregular app-layer presentation, with the
    active BLAST layer around p50 `33ms` and p75 near `99ms` actual interval.
- `120Hz vote + Fifo + frame_driver=start + frame_latency=3`, joined-frame
  follow-up:
  - Text backend was not the bottleneck in this drawer capture. Slow backend
    `layout_line` max was only `0.079ms`, and steady drawer frames had
    `android_backend_ms=0`.
  - Steady bad frames were split between draw and present. Example
    `frame_seq=1529`: GPUI frame `56.2ms`, window draw `19.4ms`, GPUI present
    `35.0ms`; matching `draw_roots` was `18.0ms`, mostly root prepaint/layout
    and root paint, while renderer `present_ms` was `29.8ms`.
  - `div_request` is cumulative/nested; use it as a cost signal, not direct
    wall-clock. It can exceed the same frame's `draw_roots total_ms`.
  - The drawer content was terminal-heavy in this run: about `1896` scene items
    and `1442` mono sprites.

Next backend probe:

- Keep the display/present mode at `120/Fifo`, default the frame driver to
  `start`, and keep `frame_latency=3` for one more capture only because that is
  the active A/B mode. The next isolated question is whether Android is treating
  the GPUI `SurfaceView` as flexible UI content and therefore not selecting a
  useful display mode for the app layer. Test `--frame-compat fixed
  --frame-change always` as a probe only; this is not the expected final policy
  for a UI renderer.
  - The SurfaceFlinger layer latency capture only had two rows in an earlier
    run because the surface was recreated near the end of that run, so do not
    use that specific layer summary as evidence.

Current interpretation:

- `Fifo` is a real backend pacing improvement for Android WGPU present
  blocking, but not the complete drawer-smoothness fix.
- The drawer repro is no longer "render path is cheap" in terminal-heavy app
  state. Steady frames can spend about `16..19ms` in `draw_roots` before the
  additional WGPU present wait.
- The current backend boundary is the combination of scene build/paint cost and
  Android surface presentation pacing. Keep separating those two in the same
  frame with `frame_seq`.

## Reading Results

If `avg_acquire_ms` or `max_acquire_ms` is high:

- Check selected present mode and surface caps.
- Compare `Fifo` vs `Mailbox` on the same physical device.
- Check background/foreground and surface replacement logs for stale surface behavior.

If `avg_draw_ms` is high:

- Inspect scene complexity, markdown/editor row work, drawer content rendering, terminal row count when a terminal is visible, atlas churn, and instance buffer growth.
- Look for repeated `instance_buffer_full`.

If `avg_mono_batches` is high while `avg_mono_sprites` is modest:

- Compare `avg_encode_ms` against `avg_mono_batches`. Hundreds of tiny
  monochrome sprite batches can dominate Android CPU time even when shaping,
  rasterization, atlas lookup, and the actual GPU draw calls are cheap.
- Check `avg_mono_unique_textures`. If it is only a few textures but
  `avg_mono_tiny_batches_4` or `avg_mono_tiny_batches_8` is high, suspect scene
  sprite ordering/batching rather than atlas pressure.
- `AtlasTile::tile_id` is unique only within its atlas texture. Sorting sprites
  by tile id without texture id can interleave textures inside the same draw
  order and force a bind group per tiny run.

If `AndroidPerf paint glyph` is high:

- Check `atlas_misses` first. Hundreds of misses in a bucket mean paint is
  synchronously rasterizing cold glyphs.
- Compare `avg_raster_bounds_ms`, `avg_atlas_ms`, and `avg_scene_insert_ms`.
  If raster bounds and atlas dominate while scene insertion is near zero, the
  fix belongs in glyph raster/cache preparation, not scene construction.
- If the handoff cache is active and `avg_atlas_ms` is low but
  `avg_raster_bounds_ms` remains high, the remaining suspect is the rasterizer
  backend itself, not atlas insertion.
- Compare `AndroidPerf wgpu atlas flush` and renderer
  `max_atlas_before_frame_ms`. Upload flush cost can add a smaller second hitch
  after the CPU-side paint miss work.

If text backend `max_shape_new_ms` is high:

- Compare it with `AndroidPerf text backend lock`. High `max_wait_ms` points at
  text-system lock contention, often from concurrent font loading or glyph
  rasterization. Low wait but high `max_locked_layout_ms` keeps the suspect
  inside cosmic-text shaping/layout itself.
- Check `AndroidPerf text backend font_load` around the same repro. Font
  database growth during scroll can invalidate or perturb backend font matching
  even when upper-layer text layout caches are behaving correctly.
- If embedded app fonts disappear after Android rotation, check whether the
  Activity recreated GPUI's platform/text system in the same process. Zedra's
  embedded font loader must dedupe per text-system instance, not with a
  process-global once guard, or the new Android text system will keep only
  platform-loaded system fonts.
- Rotation should not recreate `MainActivity` for the normal GPUI root surface.
  The Android host declares rotation-related `configChanges` and forwards
  configuration changes into the existing `GpuiRuntimeController`; otherwise
  `onDestroy` clears the platform and the app restarts at Home.
- Check non-ASCII, missing-font-run, font-id-miss, loaded-font, and face-count
  fields before assuming the slow path is caused by markdown, uniform lists, or
  persistent row content.
- Slow `layout_line` rows include only numeric identifiers (`first_cp`,
  `first_run_font_id`, `first_glyph_font_id`, `first_glyph_id`) so cold
  fallback/font-cache paths can be correlated without logging file content.
- Check `single_char_layouts`, `avg_single_char_ms`, and
  `AndroidPerf text line wrapper truncate`. High single-character layout counts
  with width misses point at truncation/wrapping width probes repeatedly shaping
  characters before the final line layout.

If `AndroidPerf gpui draw_roots` has high line-layout time but backend
`layout_line` is much lower:

- Compare `shape_text_font_run_ms`, `shape_text_resolve_font_ms`, and
  `shape_text_font_run_non_resolve_ms` in the draw matrix. If resolve-font time
  is small, the cost is not Android font selection; continue into GPUI text-run
  preparation, line-layout cache lookup, layout compute, or renderer batching.
- Compare `max_total_line_current_lookup_ms`,
  `max_total_line_previous_lookup_ms`, `max_total_line_near_miss_ms`,
  `max_total_line_materialize_ms`, `max_total_line_cache_insert_ms`,
  `max_total_line_wrap_ms`, and `max_total_line_force_width_ms` in the same
  bucket. This separates cache-map work and GPUI text preparation from the
  actual platform text backend call.
- Use the matching `avg_line_*` fields to distinguish one cold spike from
  sustained per-frame overhead during scroll or text-editor interaction.

If `AndroidPerf taffy compute` has high `max_measure_calls` or
`max_total_measure_calls`:

- Check `max_source_calls_source` and `max_source_calls`. If most calls in a
  compute come from one source, the next suspect is repeated measurement of
  that element path rather than broad tree size.
- Check `max_source_elapsed_source` and `max_source_elapsed_ms`. If elapsed
  time follows the same source as call count, the source is both frequently
  measured and individually expensive.
- Check `avg_distinct_measure_sources`. A low value with high measure calls
  points at repeated passes over a small set of measured nodes; a high value
  points at broad visible-tree measurement.
- Check `avg_width_definite_calls`, `avg_width_min_calls`, and
  `avg_width_max_calls`. Text cache misses that follow min/max-content probes
  usually mean layout constraints are forcing intrinsic measurement before the
  final definite-width layout.
- Then check `AndroidPerf text element layout` identity fields. High
  `distinct_layouts` with a low `top_layout_samples` means many visible text
  nodes are participating. Low `distinct_layouts` or a high
  `top_layout_samples`/`top_layout_max_call_index` points at one or a few text
  layouts being remeasured repeatedly.

If `avg_pending_tasks_ms` or `pending_tasks` is high:

- Inspect foreground executor work being drained during frame callbacks.
- Move non-visual or session/network work off the frame-critical path.

If `avg_frame_request_ms` is high:

- Inspect GPUI request-frame callbacks and window scheduling.
- Confirm fling is not routed through GPUI's forced-refresh path. During a
  normal fling, `fling_frames` may increase but `forced_frames` should not climb
  in lockstep.
- Compare it with the `AndroidPerf gpui frame` line. High
  `avg_window_draw_ms` points at CPU scene construction/layout, while high
  `avg_present_ms` points back toward the platform renderer path.
- Then compare app-level logs for the active view:
  `AndroidPerf markdown render`, `AndroidPerf markdown rows`,
  `AndroidPerf editor render`, `AndroidPerf editor rows`,
  `AndroidPerf drawer host`, and `AndroidPerf workspace drawer`.
  High markdown/editor row time points at visible row construction. High drawer
  host time points at overlay/animation wrapping, while high workspace drawer
  time points at the active drawer tab content.
  If the terminal is visible, also compare `AndroidPerf terminal render` and
  `AndroidPerf terminal paint`.

If `framestats` shows sustained jank but AndroidPerf numbers are low:

- Inspect Java/Kotlin work, system UI overlap, Android resource/layout work, and GPU driver behavior outside Rust timing.

If `samples` is always `120`:

- That only means the log emitted after a full aggregation bucket.
- Check `callback_hz` on the main-loop line and `draw_hz` on the renderer line. On a 60 Hz device, a healthy continuous scroll should be near 60. Lower rates or large `sample_window_ms` indicate missed callback or draw opportunities even though the bucket still says `samples=120`.

## Physical Device Smoke Test

Use a physical Android device. Emulator numbers are not enough for this investigation.

```sh
./scripts/run-android.sh --target arm64-v8a
```

Clear frame stats:

```sh
adb shell dumpsys gfxinfo dev.zedra.app.debug reset
```

Test flow:

1. Start host daemon: `zedra start --workdir .`
2. Open the debug Android app and connect to a workspace.
3. Open a large markdown file from the drawer.
4. Scroll for 30 seconds, fling several times, then open a large source file and repeat in the code editor.
5. With each file visible behind it, drag the workspace drawer and switch drawer tabs.
6. Background and foreground the app.
7. Confirm rendering resumes without reconnecting.
8. Capture frame stats:

```sh
adb shell dumpsys gfxinfo dev.zedra.app.debug framestats
```

Expected:

- No per-touch, per-IME, per-key, per-fling, or per-inset log spam.
- Periodic `AndroidPerf` summaries during scroll or fling.
- Surface replacement logs show preserved renderer behavior after background/foreground.
- No sustained jank in `framestats` on the target physical device.

For release app id, use `dev.zedra.app` instead of `dev.zedra.app.debug`.

## Build Checks

Prefer targeted checks first:

```sh
cargo fmt --manifest-path vendor/zed/Cargo.toml -p gpui_android -p gpui_wgpu
cargo ndk -t arm64-v8a check --manifest-path vendor/zed/Cargo.toml -p gpui_android
./scripts/build-android.sh --target=arm64-v8a
cd android && ./gradlew assembleDebug
```

The root workspace excludes `vendor/zed`, so vendored GPUI checks must use `vendor/zed/Cargo.toml`.

## Current Handoff State

The Android performance work was handed off into:

- root project: `/Users/thomasle/projects/zedra`
- vendored GPUI checkout: `/Users/thomasle/projects/zedra/vendor/zed`

The requested path `/Users/thomasle/projects/zedra/vendor/main` was not present on disk during handoff. Use `vendor/zed` unless a separate vendor worktree is created later.

At handoff time, the main checkout had:

- `docs/MANUAL_TEST.md` updated with `0c-Android-WGPU. Performance Smoke`
- `vendor/zed/crates/gpui_android/src/android/ffi.rs` updated with aggregated main-loop `AndroidPerf` logging
- `vendor/zed/crates/gpui_android/src/android/platform.rs` updated so `process_pending_tasks` returns a drained task count
- `vendor/zed/crates/gpui_wgpu/src/wgpu_renderer.rs` updated with surface cap logging and renderer acquire/draw `AndroidPerf` logging

The following checks passed in `/Users/thomasle/projects/zedra`:

```sh
cargo fmt --manifest-path vendor/zed/Cargo.toml -p gpui_android -p gpui_wgpu
cargo ndk -t arm64-v8a check --manifest-path vendor/zed/Cargo.toml -p gpui_android
./scripts/build-android.sh --target=arm64-v8a
```

## Current Next Plan

### 2026-05-11 Editor Layout Cache Follow-Up

The row-ID probe showed `root_layout_without_id=0`, but
`root_layout_reused=0` remained. That is expected: `root_layout_reused` only
means the same `Drawable` was measured more than once before paint in one draw
cycle. Virtual-list rows are recreated each draw, so stable ids help
attribution but do not create cross-frame layout reuse.

Useful steady editor samples showed roughly:

- `root_prepaint_layout_ms`: about `8-10ms`
- `root_layout_samples`: about `57-87`
- `taffy_request_nodes`: about `329-511`
- `taffy_request_measured_nodes`: about `100-132`
- `taffy_measure_calls`: about `302-372`

Attempted and then reverted:

- `UniformList` no longer measures one item during `request_layout` when using
  default `ListSizingBehavior::Auto`; that size is unused there and the real
  item height is measured in prepaint.
- `StyledText` can accept a caller-owned `TextLayout` cache.
- Code editor and git diff cached lines now keep a stable `TextLayout` per
  cached line and reset those layouts when syntax/highlight runs change.
- stable ids on recreated editor and git-diff virtual rows.

Result:

- The cache attempt did increase `text_cache_hits` and reduced Android backend
  shaping in steady frames, but visible jank remained dominated by GPUI
  prepaint/layout and paint.
- The added row state and ids were not clearly beneficial, and may distort
  pure immediate rendering behavior.
- Keep this as a later app/framework-layout optimization track. It is not the
  active fix direction now.
- Steady `editor rows detail` logs are disabled so platform/backend runs are
  less perturbed by app-level row diagnostics.

1. Keep physical-device logs as the source of truth. The app-level markdown,
   editor, drawer, and debug benchmark views are repro surfaces, not the current
   fix boundary.
2. Treat synchronous tree-sitter query compilation during file open as fixed.
   If `editor set_content` regresses above low single-digit milliseconds, first
   check that it is still using `Highlighter::pending_for_filename(...)`.
3. Continue on platform/backend optimization first:
   - glyph raster and atlas miss behavior in `gpui_android`
   - WGPU/Vulkan encode, submit, finish, and present behavior in `gpui_wgpu`
   - Android-specific text backend stack costs after GPUI cache effects are
     separated out
4. Keep isolating Android backend costs with cached-scene/text-heavy
   benchmark views and renderer rows. Text/glyph raster, atlas upload,
   monochrome batching, and WGPU present/backpressure are still valid suspects.
5. Keep Android native glyph APIs as an API 31+ experiment, not the main fix for
   the current `minSdk 21` app.

### 2026-05-14 Drawer Tap Jank Follow-Up

The current clean repro is tapping the drawer toggle button repeatedly. The
first few tap-open animations can look smooth, while later taps can become
visibly worse; dragging the drawer is not the same repro.

Latest evidence from the filtered process logs:

- `AndroidPerf drawer toggle` is tiny, so the tap handler itself is not the
  bottleneck.
- `AndroidPerf animation frame` shows the visual snap mostly finishing near
  the expected duration, but frame gaps are irregular.
- `gpui draw_roots` and text/layout do not consistently grow in the later bad
  taps.
- Later bad frames correlate more with `renderer present_ms`,
  `gpui frame present_ms`, and SurfaceFlinger late/irregular presentation than
  with app-level drawer render cost.

Current probe:

- Kotlin Choreographer assigns a monotonic `frame_seq`.
- `gpui_android` stores that sequence in `gpui::mobile_perf` before processing
  the JNI frame callback.
- `gpui frame slow`, `gpui frame sample_row`, `renderer slow`, and
  `renderer sample_row` include the same sequence as `frame_seq` or
  `renderer_frame_seq`.
- Use this to correlate a single bad visual frame across
  Choreographer -> GPUI frame -> WGPU renderer, instead of aligning rows only by
  timestamp.

Frame-pacing A/B:

- `60/Fifo` showed large present/backpressure stalls and irregular
  SurfaceFlinger presentation.
- Forcing the SurfaceView frame-rate vote to `120` while keeping `Fifo`
  improved the worst stalls, but still left many `30-45ms` drawer-tap frames.
- Forcing `120/Mailbox` selected `Mailbox` successfully on the device, but did
  not improve this repro; renderer and GPUI present averages were worse than
  `120/Fifo`.

Next probe:

- `AndroidPerf main loop slow` logs the same `frame_seq` from the Rust JNI
  frame boundary, split into pending-task drain, fling processing, whole
  GPUI-frame request time, live window count, request callback count, and
  callback elapsed time.
- `AndroidPerf choreographer slow` also logs the idle gap since the previous
  callback ended and the previous callback duration. Use these fields to tell
  whether slow drawer-tap frames are caused by GPUI work inside the callback,
  Android compositor/present backpressure, or callback scheduling drift after a
  blocked frame.
- `AndroidPerf renderer slow` splits renderer pre-present cost into known
  phases and unaccounted time, plus per-`queue.write_buffer` uniform updates
  and gaps since the previous renderer draw/submit. Use this after tiny-scene
  drawer frames show `16ms+` renderer work with only a handful of batches.

Next manual run:

```sh
./scripts/android-render-capture.sh prepare
```

Then test only the drawer toggle tap repro for 15-20 taps and capture with:

```sh
./scripts/android-render-capture.sh pull
```

### 2026-05-14 Present Backpressure Probe

Correct adb repro:

```sh
adb shell input tap 500 1405 # open saved workspace from Home
adb shell input tap 900 600  # close the initially visible drawer if needed
adb logcat -c
adb shell input tap 56 158   # hamburger, not drawer home icon
sleep 0.65
adb shell input tap 900 600  # backdrop close
sleep 1.25
adb logcat -d -v threadtime | rg "AndroidPerf|drawer snap|draw_roots slow|window_draw slow|gpui frame slow|renderer slow|async_present|present_backpressure_skip|choreographer slow"
```

Important correction: when the drawer is already open, the same top-left
region is the drawer home icon and returns to Home. Always verify the drawer is
closed before treating `(56, 158)` as the hamburger.

Current device surface caps:

- adapter: `Mali-G68 MC4`
- backend: `Vulkan`
- supported present modes: `[Mailbox, Fifo]`
- `Immediate` and `FifoRelaxed` debug choices fall back to `Fifo` on this
  device.

Useful finding:

- Inline `Fifo` repeats UI-thread stalls because `SurfaceTexture::present()`
  commonly blocks for `16-30ms`.
- Offloading `present()` alone moves that stall into the next
  `get_current_texture()` acquire.
- Android now defaults to the offloaded present path. The Choreographer frame
  loop drains GPUI foreground tasks first, then skips only draw/acquire while an
  older async present is still pending. This keeps dirty work queued and avoids
  moving the same swapchain stall back onto the UI thread.

Latest offload-gated drawer sample:

- no `renderer slow` rows in the filtered drawer open/close capture
- no `draw_roots slow` rows in the same capture
- worst main-loop rows were around `16.8ms` and `18.2ms`, down from earlier
  `30-40ms` callback frames
- async present worker still reports `present_ms` around `8-13ms`, confirming
  the surface/compositor wait still exists but no longer dominates the UI
  callback

Interpretation:

- This is a real Android backend direction, not an app-level drawer fix:
  synchronous WGPU acquire/present on the Android UI thread is the largest
  repeated stall in this repro.
- The offload gate is the current minimal backend fix. It reduces UI-thread
  blocking, but the durable architecture should still move Android GPUI
  rendering/presentation to a dedicated render/runtime thread or equivalent
  backend scheduler so UI events, GPUI foreground tasks, and swapchain waits
  cannot block each other.

## 2026-05-14 draw_roots Cost, Instrumentation A/B, iOS Comparison, Cache Defeat

This session continued the drawer-tap repro on a physical device (Mali-G68,
120Hz, `dev.zedra.app.debug`). It confirmed the offload-present fix works,
isolated the remaining cost, ruled out instrumentation self-inflation, compared
against iOS, and found why the existing view caching does not engage.

### Offload present: confirmed working

Drawer-tap repro (15 open/close): zero `renderer slow` rows. `renderer
async_present` runs on a worker thread (`present_ms` 9-14ms, off the UI
thread). `present_backpressure_skip` gate is active. The UI-thread stall is no
longer the synchronous present.

### Remaining cost is GPUI `draw_roots`, not the backend

Drawer-tap slow frames: `window_draw slow total_ms` 21-27ms, almost entirely in
`draw_roots_ms`. Decomposition (terminal-heavy scene, ~660 scene items):

- prepaint dominates: `root_layout_request` 3.5-5.7ms, `deferred_layout_request`
  2.3-3.4ms, `element_prepaint` 3-5ms. `layout_request` (the recursive
  request-layout walk plus Taffy node construction) beats `layout_compute`
  (Taffy solve, 0.4-0.6ms) by ~8-10x.
- paint: `root_paint` 5-10ms, `deferred_paint` 1.4-2.5ms.
- not the cost: Taffy solve, text layout (<1ms), Android text backend (0ms),
  glyph raster (0 atlas misses — codex's text caching holds), renderer, present.
- top exclusive contributor in `request_layout_exclusive` is
  `gpui::view::AnyView` — 10 calls, 2.7-4.9ms exclusive.

### Instrumentation is not inflating the numbers

Added a runtime gate `gpui::set_instrumentation_enabled` /
`instrumentation_enabled` in `gpui::mobile_perf`, wired through
`setDebugInstrumentation` JNI -> `GpuiRuntimeController` -> `DebugRenderPacing`
-> `scripts/android-render-pacing.sh --instrumentation on|off`. It early-returns
every `record_*` sink in `mobile_perf` (12 fns) and `crates/zedra/src/
android_perf.rs` (17 fns), removing the `Mutex`/aggregation/per-source
attribution and the per-anim-frame `info!` calls.

A/B over the same repro: instrumentation on vs off moved the slow-frame
`request_callback_ms` by ~0.7-1.3ms (within run-to-run noise); only the max tail
shrank (48ms -> 39ms). Verdict: the 16-22ms `draw_roots` cost is real GPUI work.
The toggle defaults on and is a reusable A/B tool; it is not gated by a build
feature so the inline `Instant::now()` pairs still run, but call-count math
bounds those well under 1ms/frame.

### iOS comparison (IosPerf, same terminal-heavy scene)

iOS log capture required two fixes: `crates/zedra/src/ios/logger.rs` now also
writes to stderr (idevicesyslog cannot decode the app dylib's os_log on recent
iOS; `devicectl device process launch --console` captures stderr cleanly), and
`gpui::mobile_perf::android_frame_sequence` is now available on iOS (returns 0)
so `gpui` compiles for `aarch64-apple-ios` — three unconditional call sites in
`window.rs` were previously android-only and broke the iOS build.

Findings:

- iOS `draw_roots` is NOT cheap on the same scene — it also hits 17-18ms.
- The difference is frequency: iOS produced ~5 slow rows in 15 taps (mostly
  transitional: home view, cold first-paint, cold text); Android produced ~13+,
  all steady drawer-anim frames. iOS steady frames mostly stay under threshold.
- Per phase, Android pays a broad ~1.5-2x tax (`layout_request`,
  `element_prepaint`, `paint` all ~2x) — a uniform per-operation cost, not one
  hot path. It is NOT an optimization gap: `gpui` is already `opt-level = 3` in
  the root dev profile.
- iOS has its own different slow spots: `gpui frame slow` is `present_ms`
  12-18ms (Metal present); `terminal_card.rs` `deferred_paint` 7-9ms.
- Reframe: the `cx.notify`-per-frame full-redraw pattern costs ~17ms on both
  platforms for this scene. iOS is over budget less often. Reducing `draw_roots`
  cost helps both.

### Why the existing `.cached()` view caching does not engage

`DrawerHost::new` already wraps `content` and `drawer` in `AnyView::cached(...)`
(`drawer_host.rs:113-114`). The Android `draw_roots` still re-renders
`workspace.rs:2602` (`WorkspaceContent::render`) every animation frame, so the
cache misses every frame. A device probe in the cached-view prepaint path
(`gpui/src/view.rs`) and in `Window::refresh` logged the reason:

- 60-74% of misses have `window.refreshing == true` as the ONLY failing
  condition (bounds/content_mask/text_style/dirty all match).
- `window.refreshing` is set by `window.refresh()`. The dominant caller is
  GPUI's built-in `:active` press feedback: `div.rs:3227` (pointer up) and
  `div.rs:3288` (pointer down) call `window.refresh()` for any pressable div.
  Each drawer-toggle tap therefore fires a full-window `refresh()` on pointer
  down and up, which sets `refreshing = true` and forces every `.cached()` view
  to miss that draw. ~100+ `refresh()` calls per 20 toggles.
- `window.rs:4956` also calls `window.refresh()` whenever `force_render` is set
  (comment scopes it to GPU device recovery, but it fires for generic forced
  frames too).
- The right-side "Workspaces" drawer's panel content cannot be cached at all:
  it lives inside the sliding panel, so its `bounds`/`content_mask` change every
  frame (`bounds_ok=false mask_ok=false` misses). Only the left drawer's
  fixed-position workspace `content` can benefit from caching.

`drawer snap` logic itself is clean: stable 100/160ms durations, every `start`
has a matching `done`, generation counter increments cleanly. Repeated-tap
degradation ("later opens jankier") was not cleanly reproduced as monotonic
growth in automated taps (`main loop slow` is noisy 17-36ms with one 36ms
spike); it likely needs a frame-interval/pacing capture or is compositor-side.

### To make caching actually work (not yet done)

- Change GPUI's `:active` press feedback (`div.rs:3227/3288`) from a full
  `window.refresh()` to a targeted invalidation of the affected view, and
  decouple `force_render` from cache-bypass (`window.rs:4956`). Both are core
  GPUI changes affecting every pressable div / forced frame app-wide.
- Even fixed, only the left drawer's fixed-bounds `content` benefits; the right
  drawer's sliding panel content needs a different approach (snapshot, or cache
  the panel content at a stable local origin and translate the container).

### Instrumentation currently in the tree (uncommitted)

Kept: the `set_instrumentation_enabled` toggle (gpui `mobile_perf` + gpui_android
JNI + Kotlin `DebugRenderPacing` + `android-render-pacing.sh`); the iOS
`android_frame_sequence` build fix; the iOS `logger.rs` stderr write;
`nslog_bridge.m` reverted to `%s`. Removed: the ad-hoc `cached_view miss` probe
in `view.rs` and the `window_refresh caller` probe in `window.rs` (findings
captured above).

Known unrelated bug: `scripts/run-ios.sh` installs from a stale DerivedData
path; the build succeeds but install fails with "not a valid bundle" — worked
around by installing the freshest `Zedra.app` manually.

## 2026-05-15 Cleanup Checkpoint, Feature-Gated Re-Introduction Plan

Decision to reset the Android-backend track to a clean baseline before
re-introducing each win behind its own Cargo feature so we can measure each
change against upstream. The full investigation (offload-present, atlas
oldest-first, batch caching, FreeType A/B, swash handoff cache, debug pacing
controls, instrumentation toggle, iOS comparison, `:active`-defeats-cache
finding) is recorded in the dated sections above and remains the source of
truth for what to re-implement.

### What now lives in the tree

Vendor/Zed (`vendor/zed`):

- `crates/gpui_wgpu/Cargo.toml` — single experimental feature
  `atlas-oldest-first` declared (off by default).
- `crates/gpui_wgpu/src/wgpu_atlas.rs` — `atlas-oldest-first` is implemented:
  on Android + monochrome, the allocator scans textures oldest→newest so common
  glyphs stay in the primary atlas, and the default mono atlas size is bumped
  to 2048×2048. Both changes are gated by
  `#[cfg(all(target_os = "android", feature = "atlas-oldest-first"))]`.
- `crates/gpui_android/Cargo.toml` — pass-through feature
  `atlas-oldest-first = ["gpui_wgpu/atlas-oldest-first"]`.

Everything else under `vendor/zed` is at upstream HEAD. The diagnostic
instrumentation that was added during the investigation (`mobile_perf` module,
per-element/draw/text/glyph `AndroidPerf`/`IosPerf` hooks, sample structures,
slow-row logging) is gone.

Root (`zedra`):

- `Cargo.toml` — `[profile.dev.package]` keeps hot crates optimised
  (`gpui`/`gpui_android`/`gpui_wgpu`/`taffy`/`cosmic-text`/`swash`/`fontdb`/
  `harfrust`/`rustybuzz`/`skrifa`/`ttf-parser`/`unicode-*`/`yazi`/`zeno`).
  Real measured Android benefit; kept.

Everything else under the zedra app is at HEAD. The app-level perf hooks,
benchmark views, `HostInfoState` split, drawer `.cached()` experiment, and the
syntax-highlighter cache rework are gone.

Tooling kept on disk but currently dormant (untracked):

- `scripts/android-{render-pacing,render-capture,perf-summary}.{sh,py}` —
  designed to drive debug pacing broadcasts and parse `AndroidPerf` log
  buckets. With instrumentation removed and `DebugRenderPacingReceiver`
  deleted, the broadcasts now have no receiver and the parser has no input.
  Kept so the interface is documented; will be re-wired alongside the
  re-introduced features.
- `android/app/proguard-rules.pro` — release-build artifact, left in place.

### What is intentionally deferred for the next session

The following wins were validated during the investigation but are not yet
re-introduced under feature flags. Each is its own self-contained piece of
work, planned as a separate commit:

1. **`offload-present`** — async WGPU `SurfaceTexture::present()` worker plus
   `android_pending_async_presents_for_frame_skip` API. Belongs in
   `gpui_wgpu/src/wgpu_renderer.rs` with a consumer in
   `gpui_android/src/android/ffi.rs` (`process_android_frame` skip-while-pending
   check). Confirmed working in the prior captures — biggest single backend
   win.
2. **`batch-bind-group-cache`** — shared instance bind group with `first_instance`
   offsets, plus consecutive mono text batches keep pipeline/globals bound.
   Reduced encode time from ~6.8 ms (max 24 ms) avg to ~1.4 ms (max 5.8 ms)
   in the 2026-05-10 capture.
3. **Choreographer driver with debug pacing controls** — `gpui_android`
   `GpuiRuntimeController` Choreographer-driven `gpuiRequestFrame(frameSeq)`,
   `setDebugPresentMode/Strategy/MaxFrameLatency/FrameDriverMode`, and the
   matching JNI exports + `set_debug_*_override` setters in
   `gpui_android::android::window`. With this comes the
   `DebugRenderPacingReceiver.kt` broadcast wiring, manifest entry, and the
   `scripts/android-render-pacing.sh --frame-rate/--present-mode/...`
   interface for runtime A/B.
4. **FreeType A/B rasterizer + swash handoff cache** — `gpui_android`
   text_system path that uses FreeType for grayscale non-emoji glyphs and a
   bounded same-params swash image handoff cache to remove the duplicate
   render on atlas miss. Likely behind a feature too (TBD).
5. **`AndroidFrameRequestStats` return + sheet glue** — `request_frame_*`
   returning `{ live_windows, callbacks }`, `process_pending_tasks → usize`,
   and the `Java_dev_zedra_app_SheetHostView_nativeSheetProcessSurfaceCommands`
   JNI in `crates/zedra/src/android/jni.rs` that drains tasks + forces a
   frame when a sheet surface is created.

Each will get a small feature-gated commit, default off, A/B-testable via
`cargo ndk ... --features gpui_android/<name>`.

### Known active findings to address with the re-introduction

When `offload-present` lands again it must keep the present-backpressure-skip
gate that was confirmed to remove the UI-thread present stall. Independent of
that, the dated 2026-05-14 entry above ("Cache Defeat") documents the more
fundamental finding that GPUI's built-in `:active` press feedback in
`div.rs:3227/3288` calls full `window.refresh()` on every pointer down/up,
which sets `window.refreshing = true` and defeats every `.cached()` view in
the same draw. That is a separate upstream `gpui` change and intentionally
not part of the Android-backend track being re-introduced here; it is filed
for a future investigation.
