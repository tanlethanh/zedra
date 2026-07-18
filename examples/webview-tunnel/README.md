# Webview tunnel test app

A self-contained localhost web app for manually testing the Zedra in-app webview and its SOCKS tunnel. It runs three loopback servers so one page proves the tunnel forwards real TCP streams across ports — not just simple HTTP.

| Port | What |
| --- | --- |
| `http://localhost:5173` | Frontend page (the URL you open in Zedra) |
| `http://localhost:5174` | Backend API — `/api/info` (JSON) and `/api/stream` (SSE) |
| `ws://localhost:5175` | WebSocket echo |

Pure Python standard library, no dependencies (Python 3.8+).

## Run

On the host machine the Zedra app connects to:

```sh
./examples/webview-tunnel/run.sh
```

Then from a Zedra terminal on the device:

```sh
printf 'http://localhost:5173\n'
```

Tap the underlined link. The page opens in the in-app webview through the tunnel.

## What to check

- **Page loads** in the native webview (Safari-style bottom bar: back / forward / address pill with lock + reload / share / close).
- **Backend API**: the "BACKEND API" card turns `ok` after tapping *Call /api/info* — proves a second localhost port is reachable.
- **SSE**: the "SERVER-SENT EVENTS" card shows a rising tick count — proves streaming responses survive the tunnel.
- **WebSocket**: the "WEBSOCKET ECHO" card shows `connected`, and *Send* echoes your text back — proves long-lived bidirectional streams.
- **Navigation**: the internal link loads Page 2 and back works.
- **Address bar**: tap the address pill, type a host (e.g. `localhost:5174/api/info`), press Go — it navigates; the field lifts above the keyboard.

## Native JS bridge (optional)

The page also probes `window.zedra` / `window.webkit.messageHandlers.zedra`. The plain tunnel does **not** wire a bridge, so the "NATIVE JS BRIDGE" card shows `absent` — that is expected.

To see the bridge light up (`present`, messages posted to Rust, `window.zedraSetStatus` driven from `eval_js`), open this page from a webview configured with `on_message`/`inject_js` — see the **Settings → Developer → Webview** test item and `docs/WEBVIEW.md`.
