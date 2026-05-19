# GPUI Devtool

In-app HTTP server that exposes the latest frame's interactive hitboxes by
`ElementId` path and accepts synthetic taps. Lets an AI agent (Claude, Codex,
…) or test harness drive the Zedra UI without manual reproduction.

## Build prerequisite

```sh
./scripts/run-android.sh --debug --target arm64-v8a --devtool
```

`--devtool` only takes effect in debug builds. The cargo feature is
`devtool`, plumbed through `zedra → gpui_android → gpui`. On start the app
logs `devtool: listening on 127.0.0.1:9777`.

## Bridge and use

```sh
./scripts/devtool.sh bridge          # adb forward + /ping
./scripts/devtool.sh list            # leaf-only element table
./scripts/devtool.sh tap <leaf>      # synthetic tap, by ElementId leaf or full path
./scripts/devtool.sh tap-xy <x> <y>  # raw logical-pixel coords
./scripts/devtool.sh ping            # liveness check
./scripts/devtool.sh elements        # raw JSON
```

Run `bridge` once per session (or each time the device is reconnected). The
wrapper is plain bash — any agent that can `Bash:` can drive the UI.

Port: `9777` by default. Override with `ZEDRA_DEVTOOL_PORT` for both the app
(at launch) and the bridge.

## Typical agent loop

1. Build + launch with `--devtool`.
2. `scripts/devtool.sh bridge`.
3. `scripts/devtool.sh list` to discover targets.
4. `scripts/devtool.sh tap <leaf>` to drive the UI.
5. Capture logs (`adb logcat`, `scripts/android-perf-debug.py`, …) and
   iterate.

## HTTP surface

For non-bash callers and reference. Endpoints accept and return JSON.

### `GET /ping`

```json
{"ok": true}
```

### `GET /elements`

```json
{
  "frame_id": 42,
  "entries": [
    {
      "path": "view#4294967297/.../drawer-host/drawer-toggle-btn",
      "instance": 0,
      "x": 0.00, "y": 36.73,
      "w": 41.82, "h": 41.82
    }
  ]
}
```

- `path` — slash-joined `ElementId` chain from root. Developer-tagged
  elements appear with their `.id("…")` string; untagged ancestors render as
  `view#<entity_id>`.
- `instance` — disambiguates same-path elements in one frame.
- `x`/`y`/`w`/`h` — logical-pixel bounds (post scale factor).
- `frame_id` — monotonic, useful for "tap then re-query" synchronisation.

### `POST /tap`

```json
{"element_id": "drawer-toggle-btn"}
```

Accepts either the bare leaf or the full slash path. If a leaf matches
multiple entries, the smallest-area (topmost) entry wins. Returns
`{"ok":true,"x":…,"y":…,"frame_id":…}` or
`{"ok":false,"error":"element not found"}`.

### `POST /tap_xy`

```json
{"x": 540, "y": 120}
```

Coordinates are logical pixels. Returns `{"ok":true,"x":…,"y":…}` or
`{"ok":false,"error":"x and y required"}`.

## Element coverage

Only GPUI `div` elements (and other `Interactive` callers of
`insert_hitbox`) that have an `ElementId` show up. Tag interactive surfaces
with `.id("drawer-toggle-btn")`, `.id("ws-card-0")`, etc. Untagged regions
fall through to `tap-xy`.

## Limits

- Single window assumed. The first registered window receives all taps.
- Taps fire on the next Choreographer tick (≈8-16 ms after enqueue), not
  immediately.
- One request per connection — no keep-alive, no concurrency.
- Bound to `127.0.0.1`; rely on `adb forward` for access. No auth.
- Release builds with `--devtool` compile clean but the server does not
  start (gate requires `debug_assertions` or `feature = "inspector"`).

## Where the code lives

- `vendor/zed/crates/gpui/src/devtool.rs` — element snapshot registry.
- `vendor/zed/crates/gpui/src/window.rs` — snapshot publish + relaxed
  picking-mode guard on `insert_inspector_hitbox`.
- `vendor/zed/crates/gpui_android/src/android/devtool_server.rs` — HTTP
  loop + tap queue.
- `vendor/zed/crates/gpui_android/src/android/platform.rs` — server boot,
  `process_devtool_taps()`.
- `vendor/zed/crates/gpui_android/src/android/ffi.rs` — drains the queue
  inside `gpuiRequestFrame` so taps land on the main thread.

iOS is not wired yet. The gpui-side registry is platform-neutral, so adding
an iOS server is a matter of mirroring `devtool_server.rs` against
`gpui_ios`.
