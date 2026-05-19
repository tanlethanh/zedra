# GPUI Android Performance Optimizations

Catalog of every Android-targeted performance change carried in `vendor/zed`
relative to upstream GPUI. Each entry: **what** it does, **case** it
addresses, **fix** in code, **related system**, and **status** (validated,
plausible, or *reaudit*).

Code sites are linked by `[topic-slug]` markers in source comments —
e.g. `// See: GPUI_ANDROID_PERFORMANCE.md § atlas-oldest-first`.

Audit precedent and per-spike evidence: `docs/AUDIT_P50_OPTS_SPIKES.md`,
`docs/AUDIT_GPUI_DRAW_ROOTS.md`, `docs/AUDIT_GPUI_P99.md`,
`docs/AUDIT_WGPU_SURFACE_ATLAS.md`.

> **Status legend:**
> *validated* — A/B'd with frame timing on Mali-G68 device. Keep.
> *plausible* — mechanism sound, not yet A/B'd. Keep, flag for reaudit.
> *reaudit* — known mechanism-level risk per audit doc; kept for now but
> needs a dedicated simulation case before declaring done.

---

## Surface and presentation

### wgpu-surface-config

**What.** Android wgpu surface uses `desired_maximum_frame_latency: 2` and
`PresentMode::Fifo` (with the renderer's preferred-mode fallback).

**Case.** Android Vulkan swapchain size is `latency + 1`. Latency = 2 yields
triple buffering — the only configuration that absorbs CPU spikes (GC, system
work) without GPU starvation and matches SurfaceFlinger's compositor
expectations. Latency = 1 collapses the pipeline; a single CPU stall drops a
frame visibly. FIFO is the only present mode guaranteed available on Android.

**Fix.** `vendor/zed/crates/gpui_wgpu/src/wgpu_renderer.rs::new_internal`
configures `desired_maximum_frame_latency: 2` unconditionally and falls back
to FIFO if the renderer's preferred present mode is unsupported.

**Related system.** Vulkan swapchain, Android `BufferQueue`, SurfaceFlinger
pacing.

**Status.** *validated.* Removed the `frame-latency-1=1` experiment because
it inverts p99 (audit §2).

### present-fifo

**What.** Explicitly force `PresentMode::Fifo` on Android surfaces.

**Case.** Mailbox can return `get_current_texture()` immediately, but pairs
poorly with a present-worker pipeline: when the previous frame's `present()`
is still blocked, acquire returns a stale buffer or stalls anyway, but with
unpredictable timing. FIFO guarantees a known acquire-stall boundary that
fits the rest of the pacing.

**Fix.** `gpui_android/src/android/window.rs` sets the renderer's preferred
present mode to FIFO so the surface config fallback path keeps FIFO even on
devices that report Mailbox available.

**Related system.** [wgpu-surface-config].

**Status.** *reaudit.* Audit §1 flags interaction with present-worker
threading. Keep for clarity but revisit alongside [pipelined-render-thread]
and [offload-present].

### opaque-window-default

**What.** Android window surfaces request an opaque composite alpha mode by
default.

**Case.** A transparent surface composite costs an extra blend pass in
SurfaceFlinger. Zedra's window background is fully opaque, so transparency is
unused.

**Fix.** `gpui_android/src/android/window.rs` passes
`transparent: false` to `WgpuSurfaceConfig`, which selects
`CompositeAlphaMode::Opaque` in the renderer's alpha-mode preference list.

**Related system.** SurfaceFlinger composition path.

**Status.** *reaudit.* Audit §13 reports no measurable effect on Mali —
likely already the default. Keep.

---

## Atlas

### atlas-oldest-first

**What.** Mono atlas allocator scans textures oldest-first instead of MRU.
Android default atlas size raised to 2048×2048.

**Case.** Mali tile-based renderers stall on texture binding switches. The
upstream MRU scan rotates between freshly-created mono atlas textures every
few glyphs, multiplying texture-switch count per frame (counter:
`mono_after_mono_texture_switch`).

**Fix.**
`vendor/zed/crates/gpui_wgpu/src/wgpu_atlas.rs::allocate_mono` walks the
mono atlas free list oldest-first on `target_os = "android"`. Atlas pool size
default raised to 2048² on Android.

**Related system.** Mali Valhall tile binning, `WgpuAtlas` LRU, glyph
rasterization in `gpui_android::text_system`.

**Status.** *validated.* Repro: drawer-tap with 15-20 taps. Without:
0/10 smooth seconds. With: 4/10 smooth seconds at 95 fps, max-frame 16.9 ms.
Side-by-side capture in this branch's A/B run (2026-05-19).

---

## Render pipeline (wgpu_renderer)

### bind-group-cache

**What.** Cache `wgpu::BindGroup` instances across draws keyed by
`(offset, size, atlas_id, ...)`.

**Case.** Bind-group creation costs ~50-500 µs on Mali. A typical scene has
10-20 batches; recreating them per frame is 1-5 ms of overhead.

**Fix.** `wgpu_renderer.rs` maintains a `HashMap` of bind groups; reused on
identical `(offset, size, atlas_id)` triples.

**Related system.** `wgpu::Device::create_bind_group`, `WgpuAtlas`.

**Status.** *reaudit.* Audit §7 identifies two pathologies: (a) cache wiped
on every `grow_instance_buffer` → recreate cascade = 5-10 ms spike at scroll
entry; (b) unbounded growth without LRU eviction. Mitigation suggested:
pre-size instance buffer to high-water mark, or evict by recency. Keep, but
spike-prone until fixed.

### single-write-buffer

**What.** Coalesce all per-frame `queue.write_buffer` calls for instance data
into one staging `Vec` and one upload at end of frame.

**Case.** `wgpu::Queue::write_buffer` per batch is a syscall-heavy path on
Android — each call serializes through the queue's internal staging.

**Fix.** `wgpu_renderer.rs` accumulates instance data into
`instance_staging: RefCell<Vec<u8>>` and emits a single `write_buffer` near
the end of `draw()`.

**Related system.** wgpu queue staging, [bind-group-cache] (offsets shift
when staging packs densely).

**Status.** *reaudit.* Audit §8 notes `staging.resize(end, 0)` zeros across
the entire gap (4 MB memset worst case) and the single end-of-frame copy can
amount to a full-buffer write. Keep, revisit memset cost.

### persistent-globals

**What.** Cache the per-frame `GlobalParams` UBO write across frames when
viewport, scale, and offsets are unchanged.

**Case.** A globals UBO write every frame costs a small but constant queue
upload + bind. Steady-state viewport doesn't need it.

**Fix.** `wgpu_renderer.rs` stores last `GlobalParams` in
`Cell<Option<GlobalParams>>`; skips write when equal. Invalidated on
`unconfigure_surface`.

**Related system.** Uniform buffer ring, [bind-group-cache].

**Status.** *plausible.* Audit §9 marks low risk, low magnitude. Keep.

### once-per-pass-globals

**What.** Within one render pass, write the globals UBO at most once even if
multiple bind events would normally re-bind it.

**Case.** Re-binding the same UBO is cheap but not free; eliminating
redundant binds shaves a few microseconds per pass.

**Fix.** `wgpu_renderer.rs` tracks a per-pass `globals_written: bool` flag.

**Related system.** [persistent-globals].

**Status.** *plausible.* Audit §12 — low risk. Keep.

### opaque-quads-pipeline

**What.** Separate pipeline-state-object (PSO) for opaque vs alpha-blended
quad batches, with depth/blend state pre-configured.

**Case.** Branch-free shader path for opaque quads avoids per-fragment
discard logic.

**Fix.** `wgpu_renderer.rs` creates two PSOs (`quad_pipeline_opaque`,
`quad_pipeline_alpha`) and routes batches by opacity.

**Related system.** Quad shader, Mali tile binning.

**Status.** *reaudit.* Audit §3 flags PSO switches as Mali tile-cache
flushes; spike scales with alternating batch sequences. Keep but verify on
heavy-mixed scenes.

### clear-bottom-quad

**What.** When the bottom-most full-cover opaque quad would clear the
viewport, promote it to `LoadOp::Clear` and skip the draw.

**Case.** Markdown documents often paint a full-bleed background quad first.
Drawing it explicitly costs a fragment rate write across the whole viewport;
promoting to clear lets the GPU skip the load.

**Fix.** `wgpu_renderer.rs` scans `scene.quads` for a full-cover non-rounded
opaque candidate; if found, stores its draw order in
`clear_skip_order: Cell<Option<DrawOrder>>` and clears via render pass
`LoadOp::Clear` instead of drawing.

**Related system.** Render pass setup, Mali tile clear-on-load.

**Status.** *reaudit.* Audit §5: O(N) scan over `scene.quads` each frame,
uncapped on heavy markdown (1000+ quads). Keep, profile scan cost in
isolation.

### skip-transparent-quads

**What.** Cull quads with `Hsla.a == 0.0` before encode.

**Case.** Some UI states leave fully-transparent quads in the scene; they
produce no pixels but still cost a vertex pass.

**Fix.** `wgpu_renderer.rs` skips the batch entry for transparent quads.

**Related system.** Scene building, [clear-bottom-quad].

**Status.** *plausible.* Audit §4 notes low risk. Keep.

### quad-varyings

**What.** Move per-fragment storage-buffer fetches into vertex-shader
varyings.

**Case.** Per-fragment storage-buffer indexing causes the rasterizer to
serialize through scalar memory ops on some Mali drivers. Passing values via
varyings lets the fragment shader stay vectorized.

**Fix.** `gpui_wgpu/src/shaders.wgsl` quad pipeline interpolates background,
border, corner data as varyings instead of indexing a storage buffer in the
fragment shader.

**Related system.** Quad fragment shader.

**Status.** *plausible.* Audit §6 — steady-state only, no spike risk. Keep.

### offload-present

**What.** `queue.submit + frame.present()` runs on a worker thread; UI
thread queues encoded command buffers via `sync_channel(1)`.

**Case.** `frame.present()` under FIFO blocks for ~vsync until the previous
buffer releases. Moving it off the UI thread lets the next frame's encode
overlap with display latency.

**Fix.** `wgpu_renderer.rs` spawns a present worker; `draw()` `try_send`s
the submission; on `Full`, the UI thread runs submit+present inline.

**Related system.** wgpu queue, [present-fifo],
[pipelined-render-thread].

**Status.** *reaudit.* Audit §10 identifies the inline-fallback as
spike-prone: any time the worker is still blocked on present, the UI thread
runs the full ~16.6 ms blocking submit+present itself. Mitigation: drop the
frame on Full instead of falling back, or grow the channel and add explicit
backpressure. Keep, redesign required.

### pipelined-render-thread

**What.** UI thread builds and clones the `Scene`, then handoffs to a render
worker that owns the `WgpuRenderer` mutex and runs encode+submit.

**Case.** Decouples scene assembly from GPU encode so each can run on its
own vsync slot.

**Fix.** `gpui_android/src/android/pipelined_renderer.rs` wraps
`Arc<Mutex<WgpuRenderer>>` and a `sync_channel<Scene>(1)`.

**Related system.** [offload-present], `gpui::scene::Scene` (must implement
`Clone`).

**Status.** *reaudit.* Audit §15: top spike source. Three mechanisms:
(a) `Scene::clone` is 0.5-3 ms per frame including Path Vec heap clones;
(b) inline fallback on channel-Full doubles the work on UI thread;
(c) mutex held across full draw blocks `replace_surface`/resize. Mitigation:
drop frame on Full (queue-of-1 already implies a latency cap), or
move scene clone to a pool, or rework to a copy-on-write scene. Keep
pending redesign.

---

## Frame loop and refresh

### skip-high-rate-present

**What.** Remove the high-input-rate fallback that schedules an extra
present at the end of a touch sequence.

**Case.** The fallback was upstream's mitigation for displays that downclock
the refresh rate after idle frames. On Android, the panel handles 60↔120 Hz
transitions in hardware; the extra present can fight the transition.

**Fix.** `gpui/src/window.rs` removes the `input_rate_tracker.is_high_rate()`
branch from the `needs_present` decision.

**Related system.** Android refresh-rate switching, `SurfaceFlinger` mode
changes, `InputRateTracker`.

**Status.** *reaudit.* Audit §11: removing this triggers visible jank
during 60→120 Hz transitions after scroll ends. Spikes 30-80 ms during
transitions. Keep, but verify on devices with aggressive panel downclocking.

### skip-active-refresh

**What.** Drop the implicit `window.refresh()` chain attached to
`:active` styles on `Div` elements.

**Case.** Every press-and-hold on a tappable element triggered a window
refresh, costing a full paint pass on touch-down.

**Fix.** `gpui/src/elements/div.rs` removes the ~14 `window.refresh()` calls
inside `:active`-state setup.

**Related system.** `Div` interactivity, `InteractiveElement`.

**Status.** *reaudit.* Audit §14: tiny risk that touch-down visual feedback
is delayed by one frame. Tap-down currently looks fine on device. Keep.

### force-render-no-refresh

**What.** `Window::force_render` no longer calls `window.refresh()`.

**Case.** A `force_render` (e.g. from GPU device-lost recovery) was paired
with a refresh that re-laid-out the whole window. The redraw doesn't need a
re-layout.

**Fix.** `gpui/src/window.rs::force_render` skips the refresh call.

**Related system.** GPU surface-lost recovery, layout invalidation.

**Status.** *plausible.* No measured win recorded. Keep, verify against a
device-lost reproducer.

### fling-no-force

**What.** Fling animation frames don't call `force_render`.

**Case.** Fling already schedules frames via the animation tick; the extra
`force_render` doubled the frame request rate during inertia scroll.

**Fix.** `gpui_android` fling tick emits `window.refresh()` only.

**Related system.** Touch fling integrator, animation frame scheduling.

**Status.** *plausible.* Keep.

### skip-inspector-in-debug

**What.** Skip the inspector-overlay branch in `Element::prepaint` when
running a debug build without `--features inspector`.

**Case.** The inspector branch is dead code in release; in debug it ran a
`HashMap` lookup per element. Long element trees (markdown 700+ elements)
paid the lookup unnecessarily.

**Fix.** `gpui/src/element.rs` short-circuits the inspector branch when the
feature isn't compiled in.

**Related system.** GPUI inspector, debug builds.

**Status.** *plausible.* Win is debug-only; release was already optimal.
Keep for dev-build sanity.

---

## Text rendering

### sticky-line-cache

**What.** Extend the `LineLayout` cache lifetime so cosmic-text shape
results survive across multiple frames instead of being evicted aggressively.

**Case.** During fling/scroll, the same text lines flow on and off screen
repeatedly. Default cache lifetime causes them to re-shape every cycle —
each re-shape is 100-300 µs.

**Fix.** `gpui/src/text_system/line_layout.rs` extends the cache eviction
window. Soft-cap on entries to bound memory.

**Related system.** cosmic-text shape buffer, `LineLayout` cache, fling
scrolling.

**Status.** *reaudit.* Eviction policy not yet sized; can grow unbounded on
varied content. Keep, define soft-cap.

### integer-x-glyphs

**What.** Snap glyph X positions to integer pixels before atlas lookup.

**Case.** Sub-pixel X positions cause each glyph variant to occupy a
distinct atlas slot. Snapping reduces atlas variants by ~4× and dramatically
reduces atlas misses.

**Fix.** `gpui/src/text_system/line_layout.rs` rounds glyph X to integer
during layout.

**Related system.** [atlas-oldest-first], glyph rasterization,
`WgpuAtlas` mono pool.

**Status.** *reaudit.* Affects typography quality on hi-DPI displays —
verify visual diff. Keep pending visual check.

---

## Removed and rejected

The following appeared in the experiment matrix but did not survive
consolidation:

- **`frame-latency-1`** — collapses pipeline to 1 in-flight frame; helps p50,
  inverts p99. See [wgpu-surface-config].
- **`skip-{subpixel,mono,poly}-sprites`, `skip-quads`, `skip-shadows`,
  `skip-underlines`** — A/B attribution knobs only. Visually break the app;
  never shipped.
- **`perdebug`, `perdebug-logs`** — instrumentation features (atomic
  counters + `log::info!` lines). Used to produce the audit docs. Removed
  now that consolidation is in progress; can be reintroduced ad-hoc when a
  new spike investigation begins.

---

## Reaudit backlog

Optimizations marked *reaudit* above need a dedicated simulation case
before declaring done. Suggested order, worst risk first:

1. **[pipelined-render-thread]** — Scene clone + inline-fallback + mutex
   contention. Run sustained-scroll trace; count `TrySendError::Full`
   events; time `scene.clone()` per frame.
2. **[bind-group-cache]** — Grow-cascade spike + unbounded growth. Trigger
   `grow_instance_buffer` mid-scroll; measure cache-rebuild cost; sample
   long-running map size.
3. **[offload-present]** — Inline submit+present fallback. Same trace as
   (1); count `Full` events on the present channel.
4. **[opaque-quads-pipeline]** — Mali tile-cache flush per PSO switch. Use
   mixed opaque/alpha quad scene; correlate spike with batch sequence.
5. **[clear-bottom-quad]** — O(N) scan over `scene.quads`. Profile
   scan time on heavy markdown.
6. **[skip-high-rate-present]** — 60↔120 Hz transition jank. Reproduce
   the post-scroll-idle delay.
7. **[single-write-buffer]** — full-buffer memset on grow. Time
   `staging.resize` per frame.
8. **[present-fifo]** — interaction with [pipelined-render-thread] and
   [offload-present].

Reaudit should produce: before/after frame-time histogram, p50/p95/p99,
worst-case spike duration, and a clear keep/redesign/drop verdict.
