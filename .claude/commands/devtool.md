Drive the Zedra Android UI from the agent via the in-app GPUI devtool HTTP server.

Use this when you need to reproduce a tap-driven flow (drawer open/close, navigation, button activation) without asking the user to operate the phone. Pairs well with `/perf-debug` and `/android-log` for end-to-end capture.

## One-time per session

```sh
./scripts/devtool.sh bridge
```

`adb forward tcp:9777 tcp:9777` + a `/ping` check. Re-run if the device reconnects. Requires the app built with `--devtool` (debug only):

```sh
./scripts/run-android.sh --debug --target arm64-v8a --devtool
```

## Driving the UI

```sh
./scripts/devtool.sh list                 # leaf-only element table for the current frame
./scripts/devtool.sh tap <leaf>           # synthetic tap by ElementId (bare leaf or full path)
./scripts/devtool.sh tap-xy <x> <y>       # raw logical-pixel coords
./scripts/devtool.sh elements             # raw JSON if you need instance ids or full paths
./scripts/devtool.sh ping                 # liveness only
```

After each tap, wait ~0.5–1 s before re-querying so the next frame reflects the new state.

## Typical loop

1. `scripts/devtool.sh bridge`
2. `scripts/devtool.sh list` — discover targets.
3. `scripts/devtool.sh tap ws-card-0` — navigate.
4. Wait, list again, tap the next target.
5. While iterating, grab logs (`adb logcat`, `scripts/android-perf-debug.py`) to confirm behaviour.

## Notes

- Only elements with `.id("…")` are tappable by name; untagged regions need `tap-xy`.
- If multiple entries share a leaf, the smallest-area (topmost) wins. Pass the full path to disambiguate.
- Single-window assumption; first registered window receives all taps.
- Full surface in `docs/DEVTOOL.md`.
