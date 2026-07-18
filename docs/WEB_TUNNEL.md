# Web Tunnel

Open host-side localhost web apps inside the mobile app and let the page keep talking to its backend through the existing Zedra session.

## Goal

The mobile WebView should behave as if it can reach the host machine's loopback web server. A page opened from `http://localhost:5173` should be able to load assets, call APIs, open WebSockets, use server-sent events, and run dev-server HMR without public network exposure.

This is not just HTTP fetch replay. A replay proxy can load simple pages, but it breaks interaction patterns that depend on a long-lived bidirectional connection. The tunnel must forward browser TCP streams to the host.

## Design

Two adapters behind one seam (`crates/zedra/src/web_tunnel/`), chosen per host by
the session's stable non-relay endpoint id. **`docs/WEB_TUNNEL_MODES.md` is
canonical** for how they differ, their origin tradeoffs, and when each applies;
this is the transport summary.

- **Exact-port (default).** Bind a real `127.0.0.1:<port>` listener on the device
  so the WebView loads the unmodified `http://localhost:<port>` — an honest
  loopback origin, so CORS, cookies, and `localhost`-registered OAuth behave.
  Companion ports the page references are found by a plaintext byte sniffer and
  bound the same way. Each device port is owned by one host; a collision or bind
  failure is "unavailable" and offers the alias.
- **Alias (opt-in fallback).** Serve the host under a per-host `<label>.zedra.test`
  via an ephemeral in-app SOCKS5 proxy the WebView is pointed at with
  `proxyConfigurations` (iOS 17+) / `ProxyController` (Android). The proxy decodes
  the label to the endpoint id and forwards over `WebConnect`; external hosts dial
  direct. A real device **bypasses the proxy for all loopback** (`localhost`,
  every `127.0.0.0/8`, `::1` — verified on iOS 26.5), so a non-loopback alias is
  the only proxy-viable form; a per-host alias is also what lets concurrent hosts
  share a port.

Both funnel each accepted connection through one path:

```text
WebView -> exact-port listener  OR  alias SOCKS proxy   (device loopback)
  -> WebConnect bidi RPC over the existing Zedra session
  -> host TcpStream to 127.0.0.1:<port>   (host validates loopback)
  -> host web server
```

This matches the `TermAttach` model: a local IO stream bridged through paired
irpc channels; the tunnel never parses HTTP (except the exact-port companion
sniffer).

## Scope

Supported:

- `http://localhost:<port>` and loopback IP equivalents.
- `https://localhost:<port>` when the host server already has certificates the WebView accepts.
- Normal HTTP requests and responses.
- WebSocket upgrades and WebSocket frames.
- Streaming responses such as SSE.
- Dev-server HMR and API calls to other host-side localhost ports.

Not supported initially:

- Public hostnames or arbitrary LAN destinations.
- HTTPS interception or certificate generation.
- UDP/WebRTC/TUN-level networking.
- Cross-workspace shared tunnel ports.

## WebView Routing

Exact-port opens the WebView at the literal `http://localhost:<port>` with **no
proxy** — a real device listener serves it. Alias-mode opens it at
`http://<label>.zedra.test:<port>` with the WebView pointed at the in-app SOCKS
proxy: iOS `WKWebsiteDataStore.proxyConfigurations` with a
`ProxyConfiguration(socksv5Proxy:)` (iOS 17+); Android `ProxyController.setProxyOverride`
with a `socks5://` rule, cleared when the webview closes (process-global while
set). The proxy crosses the webview seam as the `socksProxy` config field (see
`docs/WEBVIEW.md`).

## Opening a tunnel

Two ways to open a host web app on the phone:

- **From a terminal.** Tapping a `http://localhost:<port>` link printed in a
  Zedra terminal routes through the tunnel (`workspace_terminal.rs`).
- **From the host CLI.** `zedra open <target>` pushes a request to the connected
  phone. `<target>` accepts a bare port (`8080`), `host:port`
  (`localhost:5173`), or a full URL (`http://localhost:5173/app`); bare forms
  normalize to `http://` loopback. This lets a dev script fire up a server on a
  free port and then open it on the phone. The command hits the local REST API
  (`POST /api/webview`), which emits `HostEvent::WebViewRequested { url }` to the
  session with an active `Subscribe` stream; the app opens it through the same
  seam. Requires a phone connected to that workspace's daemon.

## Tracked tunnels

Each loopback tunnel opened for a workspace — from a terminal link or `zedra
open` — is recorded on `WorkspaceState.web_tunnels` (most-recent-first) and
persisted with the workspace, so the list survives app restarts and reconnects.
The session panel (drawer) shows a **Web tunnels** section: tap a row to reopen
it, long-press for Open / Remove. Non-loopback URLs open in the system browser
and are not tracked. Opening never happens automatically — the list is a
quick-reopen shortcut, not an auto-reconnect.

## Security

Keep the tunnel narrow:

- Only allow host loopback destinations: `localhost`, `127.0.0.0/8`, and `::1`.
- Reject `WebConnect` requests for non-loopback destinations (enforced host-side).
- Bind exact-port listeners and the SOCKS proxy to loopback only.
- Treat tunnel capability as part of the authenticated Zedra session, not as a public service.
- Log destination host and port and failures; do not log tunneled bytes.

## Caveats

Network layer:

- The tunnel forwards TCP only. It does not support UDP, WebRTC media paths, multicast discovery, mDNS, or a device-level VPN/TUN interface.
- `localhost` in a `WebConnect` request means host loopback, not mobile loopback. The app's exact-port listeners and SOCKS proxy bind to mobile loopback (`127.0.0.1`).
- Alias mode only: a page that internally hardcodes `http://localhost:<other>` (or an IP loopback) bypasses the proxy — a real device bypasses the proxy for all loopback — so relative/same-origin URLs work but hardcoded-loopback ones don't. Exact-port mode has no such limit (real listeners). See `docs/WEB_TUNNEL_MODES.md`.
- HTTPS works only when the WebView trusts the host server certificate. The tunnel does not terminate TLS or mint certificates.
- Plain cleartext loads are blocked by platform policy unless explicitly exempted: iOS needs `NSAllowsLocalNetworking` plus an `NSExceptionDomains` entry for `zedra.test` in `Info.plist`; Android needs `localhost`/`127.0.0.1`/`zedra.test` allowed in `network_security_config.xml`. All in place.
- When a load fails anyway — the host web server isn't listening (host-side connect refused), the tunnel drops, or any other navigation error — iOS renders an inline error page (globe icon, host, the underlying error, a **Try Again** retry) instead of a blank white page. `WKNavigationDelegate.didFailProvisionalNavigation` in `ios/Zedra/Presentations.swift` drives this; Android's `WebViewClient.onReceivedError` still only logs (see `docs/MANUAL_TEST.md` §9a).

Transport layer:

- Each browser connection maps to one `WebConnect` stream over the existing authenticated Zedra transport. QUIC stream backpressure and relay latency directly affect page load and HMR responsiveness.
- Keep byte chunks bounded. Large downloads should stream through chunks instead of accumulating whole bodies.
- A stream close/error should tear down only that browser connection, not the whole Zedra session.

Session layer:

- Web tunnels are tied to the current `SessionHandle`. Reconnect, workspace switch, app backgrounding, or host restart can drop open browser connections; the WebView should recover via reload.
- Bound exact-port listeners and the SOCKS proxy live for the app's process lifetime. Routing is keyed by the host endpoint id, so the session each forwards through is repointed to that host's latest session on reconnect.

## Implementation Notes

- Add `WebConnect` to `zedra-rpc` as a bidirectional streaming RPC. The browser path is byte streams only; an HTTP-replay fetch cannot support WebSocket/HMR/SSE semantics.
- Use byte frame structs such as `WebTunnelInput` and `WebTunnelOutput`, plus explicit connected/close/error signals.
- On the app, run a SOCKS5 proxy (`crates/zedra/src/web_tunnel/`): loopback CONNECTs bridge to the irpc channels with separate read and write tasks; non-loopback CONNECTs dial directly.
- On the host, bridge the irpc channels to `tokio::net::TcpStream`.
- Prefer bounded channels and chunk limits to avoid unbounded buffering.
- Do not parse HTTP at all. Preserving the browser's bytes is what keeps WebSockets, streaming, upgrades, and keep-alive behavior intact.

## Manual Test

For a ready-made multi-port app (page + JSON API + SSE + WebSocket), run `./examples/webview-tunnel/run.sh` on the host and open `http://localhost:5173` from a Zedra terminal. See `examples/webview-tunnel/README.md`.

1. Start a dev server on the host that serves a page and a WebSocket endpoint.
2. Print or open `http://localhost:<port>` from a Zedra terminal.
3. Tap the link in the mobile app.
4. Confirm the page loads in the native WebView.
5. Confirm the page can call a backend route and exchange WebSocket messages.
6. Background and foreground the app; confirm the tunnel either survives or fails visibly and recovers on reopen.

### Debugging a blank/broken load

The tunnel is quiet on success and logs at failure, all under the `web-tunnel:` prefix:

- **App** (`crates/zedra/src/web_tunnel/`): `info` when an exact-port listener or the alias SOCKS proxy binds; `info` when exact-port is unavailable and the alias is offered; `warn` on setup/accept failure.
- **Host** (`crates/zedra-host/src/rpc_daemon.rs`): `warn` when a `WebConnect` is rejected — invalid port, non-loopback destination, or the host can't reach the localhost server (the "web server isn't running" case).

WebView navigation failures log under `webview:`: iOS `didFailProvisionalNavigation`/`didFail` with the `NSURLErrorDomain` code (then it renders the inline error page); Android `onReceivedError`/`onReceivedHttpError`.

Read the logs by platform:

- Host: `zedra logs` or the `zedra start --verbose` terminal, filter for `web-tunnel`.
- iOS: physical devices need `xcrun devicectl device process launch --console` (`./scripts/ios-log.sh daemon`), not `idevicesyslog` — iOS can't locally decode a third-party binary's compact log entries over the classic syslog relay. See `crates/zedra/src/ios/logger.rs`.
- Android: `adb logcat -s zedra | grep -E 'web-tunnel|webview'`.

Localizing a blank/broken load:
- No app-side or webview log at all — the tap never reached the tunnel seam (check the terminal link handler).
- A host `warn` about a rejected/unreachable `WebConnect` — the host web server isn't listening on that port (start it), or the destination isn't loopback.
- A `webview:` navigation failure with no host log — a platform network policy block (iOS ATS, Android cleartext) or a session/RPC transport issue; the `NSURLErrorDomain`/`ERR_CLEARTEXT_NOT_PERMITTED` code narrows it. iOS shows the inline error page here rather than blanking.
- The page loads but stays blank — the TCP bridge is fine; inspect the page's own JS/console output.
- The main page loads but a companion port never connects — exact-port mode: the sniffer didn't surface that `localhost:<port>` in the first 64KB (or it was HTTPS); alias mode: the page hardcoded a loopback companion (bypassed). Or the host isn't serving that port (host `warn`).

## Readings And References

- Tokio `copy_bidirectional`: `https://docs.rs/tokio/latest/tokio/io/fn.copy_bidirectional.html`
- Zedra terminal stream model: `TermAttach` in `crates/zedra-rpc/src/proto.rs`, `crates/zedra-session/src/terminal.rs`, and `crates/zedra-host/src/rpc_daemon.rs`
- Chisel TCP/UDP tunnel over HTTP: `https://github.com/jpillora/chisel`
- frp reverse proxy and TCP multiplexing: `https://github.com/fatedier/frp`
- rathole Rust reverse proxy: `https://github.com/rathole-org/rathole`
- yamux stream multiplexing: `https://docs.rs/yamux/latest/yamux/`
- Android WebView resource interception limits: `https://developer.android.com/reference/android/webkit/WebViewClient#shouldInterceptRequest(android.webkit.WebView,android.webkit.WebResourceRequest)`
- WebSocket protocol: `https://www.rfc-editor.org/rfc/rfc6455`
