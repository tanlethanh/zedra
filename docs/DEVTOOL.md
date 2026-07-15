# GPUI Devtool

In-app HTTP server that exposes the current screen's interactive elements by
`ElementId` path and accepts synthetic presses. Lets an AI agent or test
harness drive the Zedra UI without guessing pixel coordinates. Shared
between iOS and Android: the HTTP server and gesture player live in the
platform-neutral `gpui_devtool` crate; each platform crate just starts the
server and feeds gesture events into its own input pipeline.

Deliberately minimal: query elements, press/long-press/tap-xy, batch with
`/sequence`. Pair with an external screenshot for visual confirmation
(`xcrun simctl io <udid> screenshot <path>` on simulator). An earlier
version also had named per-*element* actions and read-only state queries
(`.devtool_action(...)` attached to a specific `div`) — cut because they
needed bespoke code changes per element instead of working generically;
use a real gesture + screenshot instead. `/call` (below) replaces that
need for logic that isn't reachable through any element at all — register
once while debugging, delete the registration when done.

## Build and launch

```sh
./scripts/run-ios.sh sim --devtool          # or: device --devtool
./scripts/run-android.sh --debug --target arm64-v8a --devtool
```

`--devtool` requires a debug build (`--release` is rejected). On start the
app logs `devtool: listening on 127.0.0.1:9777`.

## Use

```sh
./scripts/devtool.sh bridge-ios        # or: bridge (Android)
./scripts/devtool.sh list              # elements, leaf-only table
./scripts/devtool.sh press <leaf>      # fires on_press, by ElementId leaf or full path
./scripts/devtool.sh long-press <leaf> # fires on_long_press
./scripts/devtool.sh tap-xy <x> <y>    # raw screen touch, logical-pixel coords
./scripts/devtool.sh call <name> ['<json-params>']  # see "Debug functions" below
./scripts/devtool.sh elements          # raw JSON
./scripts/devtool.sh sequence '<json-steps-array>'
./scripts/devtool.sh ping
```

Run `bridge`/`bridge-ios` once per session, or after reconnecting the
device or switching targets. It reads the same device preference file as
`run-ios.sh`/`ios-log.sh`, so it targets whichever device/simulator you
last ran. On simulator this just verifies `/ping` (shared loopback, no
forwarding needed); on device it spawns `iproxy -u <UDID> 9777:9777` in the
background and swaps the tunnel when the target changes.

Every response is JSON; `press`/`tap-xy` print the error body too, not just
on success. Port defaults to `9777`; override with `ZEDRA_DEVTOOL_PORT`.

## Typical agent loop

1. Build + launch with `--devtool`.
2. `devtool.sh bridge`/`bridge-ios`.
3. `devtool.sh list` to see what's on screen and its position.
4. `devtool.sh press`/`long-press`/`tap-xy` to drive the UI; a screenshot or
   `devtool.sh list` again to confirm the result.
5. Capture logs (`./scripts/ios-log.sh daemon`, `adb logcat`); use
   `ios-log.sh wait '<pattern>' --timeout Ns` to poll for an expected line
   instead of a fixed `sleep`.

## HTTP surface

All bodies are JSON. Dispatching endpoints block until the gesture has
actually played out (capped at 2s; past that, `"completed": false`) rather
than returning the instant the request is queued.

| Endpoint | Body | Returns |
|---|---|---|
| `GET /ping` | — | `{"ok":true,"pid":N}` — `pid` disambiguates when sim and device could both be live |
| `GET /elements` | — | `{"frame_id":N,"pid":N,"entries":[{"path","instance","x","y","w","h"}]}` |
| `POST /press` | `{"element_id"}` | Fires `on_press` at the element's center. Bare leaf or full path; topmost (smallest) entry wins on a shared leaf. |
| `POST /long_press` | `{"element_id"}` | Same as `/press`, fires `on_long_press`, holds ~600ms. |
| `POST /tap_xy` | `{"x","y"}` | Raw screen touch at logical-pixel coordinates, no element resolution. |
| `POST /call` | `{"name","params"}` | Runs a registered debug function by name (see below). |
| `POST /sequence` | `{"steps":[...]}` | Runs `press`/`long_press`/`tap_xy`/`call`/`wait_for_element` steps in order in one round trip; stops at the first unresolvable step. |

`/press` and `/long_press` return `{"ok":true,"completed":bool,"x","y"}` on
success or `{"ok":false,"error":"element not found"}`. `/sequence` returns
`{"ok":bool,"steps_completed":N,"steps_total":N,"results":[...]}`, each
result being that step's own body plus `"type"`. `wait_for_element` steps
poll in-process (capped at 10s) and take `{"element_id","timeout_ms"}`.

Elements need an `.id("...")` to show up (any GPUI `div` or other
`Interactive` element). Untagged regions fall through to `tap-xy`.

## Debug functions

`/call` runs a function app code registered ahead of time — not tied to
any UI element, so it works for logic that isn't reachable through a press
at all (opening the web tunnel from an arbitrary URL, for example, instead
of writing text into a terminal and waiting for hyperlink detection).

```rust
cx.register_devtool_action("workspace_open_webview", {
    let session_handle = session_handle.clone();
    move |params, _window, _cx| {
        if let Some(url) = params.get("url").and_then(|v| v.as_str()) {
            crate::web_tunnel::open_url(session_handle.clone(), url);
        }
    }
});

cx.register_devtool_action("workspace_session_phase", |_params, _window, cx| {
    serde_json::json!({"phase": format!("{:?}", session_state.read(cx).phase())})
});
```

```sh
./scripts/devtool.sh call workspace_open_webview '{"url":"http://localhost:5173"}'
./scripts/devtool.sh call workspace_session_phase
```

- `register_devtool_action(name, f)` — `f: Fn(Value, &mut Window, &mut App) -> R`, where `R`
  is `()` for a pure side effect (reported as `"result":null`) or `serde_json::Value` if the
  agent needs to check something back. Registering the same name twice replaces the previous
  closure. Always compiles and no-ops when the `devtool` feature is off, so the call site never
  needs `#[cfg(...)]`.
- The single `params` argument is a raw `serde_json::Value` — the function parses whatever
  fields it needs itself; devtool doesn't validate a shape. Omit `params` in the request for a
  function that doesn't need any (defaults to `Value::Null`).
- Meant to be temporary: add the registration while debugging a specific feature, delete it
  once you're done. It's not a permanent extension point for app features — see the "cut" note
  above for why per-element actions with the same shape didn't stay.
- `{"ok":true,"completed":bool,"invoked":bool,"result":Value}` — `invoked`
  is `false` if nothing was ever registered under that `name` (distinct
  from `completed:false`, a timeout).

## Limits

- **No text input.** No `/type` endpoint on either platform. For a
  terminal, write directly to its PTY device file instead
  (`printf '%s\n' 'command' > /dev/ttysNNN`) — bypasses devtool entirely.
  No equivalent for other text fields yet.
- **No screenshot on physical device without a manual step.**
  `idevicescreenshot` and `devicectl` both lack a working path on modern
  iOS without sudo-level tooling this project doesn't use. Ask a human to
  screenshot manually, or rely on `/elements` + logs. Simulator has neither
  limitation (`xcrun simctl io <udid> screenshot <path>`).
- **Native overlays are invisible to `/elements`.** `WKWebView`, native
  alert/action sheets, and the system keyboard aren't GPUI elements. A real
  long-press + screenshot still exercises the underlying handler even
  though the sheet itself can't be addressed.
- **Port collision if sim and device are both live.** The port
  (`ZEDRA_DEVTOOL_PORT`, default 9777) is a single fixed number; a
  simulator and a device's `iproxy` tunnel can both bind it, and
  `localhost:9777` then resolves to whichever the OS picks — silently.
  Check `pid` in the response if two calls seem to hit different
  instances. `bridge-ios` guards its own tunnel against this, but a
  manually started `iproxy` outside it isn't covered.
- Single window assumed; one gesture in flight at a time (a new request
  queues behind it); one HTTP connection at a time, no keep-alive.
- Every endpoint except `/ping` requires an `X-Devtool-Token` header
  (`devtool.sh` sends it automatically) — otherwise any local process
  could drive the app's real UI via simulated presses.

## Assessment

Field notes from driving the iOS app during the terminal image-upload work,
kept honest so we invest where it pays off.

**Where it earned its keep**

- Reproducible navigation without touching the phone: reconnecting a
  workspace, retrying a connect button, tapping into the terminal — scripted
  via `/tap_xy` + `/sequence`, repeatable across rebuild cycles.
- `/ping` as a cheap "is this build alive and listening" probe after each
  install.
- `/call` for reaching logic with no addressable element while debugging.

**Where it fell short (biggest first)**

- **`/elements` returns empty on iOS.** With no element tree, every tap is a
  blind `/tap_xy` against remembered coordinates — brittle across layout
  changes and the single feature that would most raise the tool's ceiling.
  Android populates it; iOS parity is the top ask.
- **Blind to the thing under test.** The whole image-upload flow is native
  UIKit — `UIEditMenuInteraction` long-press menu, `PHPickerViewController`,
  the progress HUD window, alerts. None are GPUI elements, so the devtool
  could neither see nor drive any of it. The real progress came from
  `devicectl` console capture + `nm` symbol inspection, not the devtool. The
  tool positioned the app *up to* the native boundary, then went dark.
- **On-device token friction.** The token file write fails under the app
  sandbox, so on a physical device the token must be scraped out of the
  console log before any authed call works — a real cold-start tax that
  `devtool.sh` partly hides but doesn't remove.

**Suggested improvements, prioritized**

1. Populate `/elements` on iOS (unblocks non-blind interaction).
2. Read-only native-presentation state: expose *whether* a native modal
   (picker / alert / sheet / edit menu) is currently up, even if its buttons
   stay unaddressable — enough for an agent to detect "a sheet is blocking"
   instead of tapping into the void.
3. A `/type` text-input endpoint (see Limits) to retire the PTY-write hack.
4. Deliver the on-device token without console scraping (e.g. an authed
   bootstrap read, or a sandbox-writable path both sides agree on).
5. Screenshot-on-device path, so visual confirmation doesn't need a human.

Net: strong for scripted GPUI navigation and liveness probing; currently thin
for anything touching native presentations or needing to *observe* state,
which is exactly where mobile bugs tend to live.

## Where the code lives

- `vendor/zed/crates/gpui/src/elements/div.rs` — `GestureKind` (`Press`,
  `LongPress`), named to match the `on_press`/`on_long_press` callback
  each one fires.
- `vendor/zed/crates/gpui/src/element.rs` — computes the `inspector_id`
  every hitbox needs to reach the devtool registry, under
  `feature = "inspector"` (which `devtool` depends on in Cargo.toml).
- `vendor/zed/crates/gpui/src/devtool.rs` — the cross-thread snapshot
  registry. `publish()` is pull-triggered: it only rebuilds the entry list
  when an HTTP request actually asks for one, not on every draw. Also the
  `/call` queue and `ActionRegistry` (an `App` `Global`, main-thread-only
  since it holds real closures) that `register_devtool_action` writes to.
- `vendor/zed/crates/gpui/src/window.rs` — snapshot publish and `/call`
  dispatch (`Window::invoke_devtool_call`), both called from `Window::draw`.
- `vendor/zed/crates/gpui_devtool/src/gpui_devtool.rs` — the HTTP server,
  JSON handling, and the gesture player. Platform-neutral, shared by both
  backends below.
- `vendor/zed/crates/gpui_android/src/android/{platform,ffi}.rs` — starts
  the server, drains gesture events into `PlatformInput` each frame.
- `vendor/zed/crates/gpui_ios/src/ios/{platform,window,ffi}.rs` — same for
  iOS, driven by `CADisplayLink` via `gpui_ios_request_frame`.
