# Web Tunnel

Open host-side localhost web apps inside the mobile app and let the page keep talking to its backend through the existing Zedra session.

## Goal

The mobile WebView should behave as if it can reach the host machine's loopback web server. A page opened from `http://localhost:5173` should be able to load assets, call APIs, open WebSockets, use server-sent events, and run dev-server HMR without public network exposure.

This is not just HTTP fetch replay. A replay proxy can load simple pages, but it breaks interaction patterns that depend on a long-lived bidirectional connection. The tunnel must forward browser TCP streams to the host.

## Target Design

Use Zedra's authenticated iroh/irpc session as the tunnel transport.

```text
Mobile WebView
  -> app-local loopback listener
  -> WebConnect bidi RPC over existing Zedra session
  -> host TcpStream to 127.0.0.1:<port>
  -> host web server
```

For each accepted app-local TCP connection:

1. The app validates the requested destination as host loopback only.
2. The app opens `WebConnect` as a bidirectional streaming RPC.
3. The host validates the same destination and opens a TCP connection to it.
4. Both sides forward raw byte chunks until EOF or error.

This matches the existing `TermAttach` model: a local IO stream is bridged through paired irpc channels. The tunnel layer should not parse HTTP except where needed to implement an app-local proxy entrypoint.

## Scope

Supported:

- `http://localhost:<port>` and loopback IP equivalents.
- Normal HTTP requests and responses.
- WebSocket upgrades and WebSocket frames.
- Streaming responses such as SSE.
- Dev-server HMR and API calls on the tunneled origin.

Not supported initially:

- Public hostnames or arbitrary LAN destinations.
- HTTPS interception or certificate generation.
- UDP/WebRTC/TUN-level networking.
- Cross-workspace shared tunnel ports.

## WebView Routing

The first milestone can keep the current app-local URL rewrite: the tapped host URL opens as an app-local loopback URL, and all traffic to that origin goes through the TCP stream tunnel.

To support pages that call another host-side localhost port, add app-local HTTP proxy support:

- Android: `ProxyController.setProxyOverride` can route WebView network requests through an app proxy, but it applies to all WebViews in the app while enabled.
- iOS: `WKWebsiteDataStore.proxyConfigurations` exists on iOS 17+, but Zedra's deployment target includes iOS 16, so keep a fallback path.
- Fallback: rewrite or inject only when the target app needs it, and keep the first milestone focused on same-origin web apps.

## Security

Keep the tunnel narrow:

- Only allow host loopback destinations: `localhost`, `127.0.0.0/8`, and `::1`.
- Do not follow redirects to non-loopback destinations.
- Bind app listeners to loopback only.
- Treat tunnel capability as part of the authenticated Zedra session, not as a public service.
- Log destination host and port, byte counts, and failures; do not log request bodies or headers.

## Implementation Notes

- Add `WebConnect` to `zedra-rpc` as a bidirectional streaming RPC.
- Use byte frame structs such as `WebTunnelInput` and `WebTunnelOutput`, plus an explicit close/error signal if irpc channel close is not enough for clean shutdown.
- On the app, bridge `TcpStream` to the irpc channels with separate read and write tasks.
- On the host, bridge the irpc channels to `tokio::net::TcpStream`.
- Prefer bounded channels and chunk limits to avoid unbounded buffering.
- Keep one-shot HTTP fetch helpers out of the browser path unless they are used only for diagnostics.

## Manual Test

1. Start a dev server on the host that serves a page and a WebSocket endpoint.
2. Print or open `http://localhost:<port>` from a Zedra terminal.
3. Tap the link in the mobile app.
4. Confirm the page loads in the native WebView.
5. Confirm the page can call a backend route and exchange WebSocket messages.
6. Background and foreground the app; confirm the tunnel either survives or fails visibly and recovers on reopen.

## Readings And References

- Tokio `copy_bidirectional`: `https://docs.rs/tokio/latest/tokio/io/fn.copy_bidirectional.html`
- Zedra terminal stream model: `TermAttach` in `crates/zedra-rpc/src/proto.rs`, `crates/zedra-session/src/terminal.rs`, and `crates/zedra-host/src/rpc_daemon.rs`
- Chisel TCP/UDP tunnel over HTTP: `https://github.com/jpillora/chisel`
- frp reverse proxy and TCP multiplexing: `https://github.com/fatedier/frp`
- rathole Rust reverse proxy: `https://github.com/rathole-org/rathole`
- yamux stream multiplexing: `https://docs.rs/yamux/latest/yamux/`
- Android WebView proxy override: `https://developer.android.com/reference/androidx/webkit/ProxyController`
- Android WebView resource interception limits: `https://developer.android.com/reference/android/webkit/WebViewClient#shouldInterceptRequest(android.webkit.WebView,android.webkit.WebResourceRequest)`
- WebKit data store proxy configurations: `https://developer.apple.com/documentation/webkit/wkwebsitedatastore/proxyconfigurations`
- WebSocket protocol: `https://www.rfc-editor.org/rfc/rfc6455`
