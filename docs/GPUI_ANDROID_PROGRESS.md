# GPUI Android — Progress

Short status for the Android rendering-backend work. The long investigation
log lives in `docs/GPUI_ANDROID.md`; come here for "what can I test right
now and how."

## Status

Android backend track is on a **clean baseline**. One experimental backend
optimisation is wired up and ready to A/B; the rest are documented and queued
for the next round.

| Experiment | Status | Flag | Where |
|---|---|---|---|
| Mono atlas oldest-first + larger atlas | **landed**, off by default | `atlas-oldest-first` | `gpui_wgpu/src/wgpu_atlas.rs` |
| Async WGPU present worker + backpressure skip | queued | `offload-present` | re-introduce in `gpui_wgpu/src/wgpu_renderer.rs` |
| Instance-batch bind-group cache | queued | `batch-bind-group-cache` | re-introduce in `gpui_wgpu/src/wgpu_renderer.rs` |
| Choreographer driver + debug pacing controls | queued | n/a (always-on) | re-introduce in `gpui_android` |
| FreeType A/B rasterizer + swash handoff cache | queued | TBD | re-introduce in `gpui_android/src/android/text_system.rs` |

"Queued" = previously validated against the workspace repro, reverted during
the cleanup checkpoint, ready for focused re-introduction in its own commit.
See `docs/GPUI_ANDROID.md` for measurements and design notes.

## Build & run with the optimisation flag

`scripts/run-android.sh` accepts `--atlas-oldest-first` (matches the existing
`--preview` / `--debug-telemetry` style). It is forwarded to
`build-android.sh`, which adds `atlas-oldest-first` to the cargo feature list
for `zedra`; that activates the matching feature on `gpui_android` and
`gpui_wgpu` via passthrough.

### Baseline (upstream behaviour, feature off)

```sh
./scripts/run-android.sh --debug --target arm64-v8a
```

### With the experiment on

```sh
./scripts/run-android.sh --debug --target arm64-v8a --atlas-oldest-first
```

Same device, same workspace, same repro — toggle the flag between builds and
compare. Both `--release` and `--debug` builds support the flag.

### Cargo direct (no script)

```sh
# baseline
cargo ndk -t arm64-v8a -o ./android/app/libs build -p zedra --lib --features android-platform

# with the experiment
cargo ndk -t arm64-v8a -o ./android/app/libs build -p zedra --lib \
    --features 'android-platform,atlas-oldest-first'
```

## Manually navigating the repro

The drawer-tap repro is the canonical surface where the Android backend
matters. Connect to a terminal-heavy workspace (one with the file explorer
drawer + an active terminal behind it), then:

1. Tap the drawer toggle (top-left ☰ or top-right corner, depending on the
   side you're testing) to open the drawer.
2. Tap the backdrop on the visible workspace side to close it.
3. Repeat 15-20× at a steady pace.

What to watch for between the baseline and the `--atlas-oldest-first` build:

- Per-tap perceived smoothness, especially the *later* opens (the prior
  investigation showed jank that worsens over repeated taps).
- `adb shell dumpsys gfxinfo dev.zedra.app.debug framestats` over the same
  number of taps. Lower jank counts → win.
- `adb logcat` should be quiet during the animation — the cleanup removed
  all per-frame `AndroidPerf` logging.

## When you want the other experiments back

Each queued experiment will land as its own small commit with its own Cargo
feature, default off. The doc you're reading now will gain a row per
experiment as it lands.
