# Web Tunnel

Open host-side localhost web apps inside the mobile app and let the page keep talking to its backend through the existing Zedra session.

## Goal

The mobile WebView should behave as if it can reach the host machine's loopback web server. A page opened from `http://localhost:5173` should be able to load assets, call APIs, open WebSockets, use server-sent events, and run dev-server HMR without public network exposure.

This is not just HTTP fetch replay. A replay proxy can load simple pages, but it breaks interaction patterns that depend on a long-lived bidirectional connection. The tunnel must forward browser TCP streams to the host.

## Target Design

Use Zedra's authenticated iroh/irpc session as the tunnel transport.

```text
Mobile WebView
  -> app-local SOCKS5 proxy on 127.0.0.1:<ephemeral>
  -> WebConnect bidi RPC over existing Zedra session
  -> host TcpStream to 127.0.0.1:<port>
  -> host web server
```

The native WebView loads the exact URL from the terminal, for example
`http://localhost:5173`. Native platform proxy configuration points WebView
networking at the app-local SOCKS5 proxy. The URL is not rewritten to an
app-local port.

For each accepted SOCKS `CONNECT` request:

1. The app validates the requested destination as host loopback only.
2. The app opens `WebConnect` as a bidirectional streaming RPC.
3. The host validates the same destination and opens a TCP connection to it.
4. Both sides forward raw byte chunks until EOF or error.

This matches the existing `TermAttach` model: a local IO stream is bridged through paired irpc channels. The tunnel layer should not parse HTTP except where needed to implement an app-local proxy entrypoint.

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

Use native WebView proxy APIs:

- iOS: Zedra targets iOS 17+, so the WebView uses a nonpersistent `WKWebsiteDataStore` with `proxyConfigurations` set before page load. The proxy is a Network framework `ProxyConfiguration(socksv5Proxy:)`.
- Android: AndroidX WebKit `ProxyController.setProxyOverride` routes WebView requests through `socks://127.0.0.1:<port>`. Call `removeImplicitRules()` so localhost is not silently bypassed. When `PROXY_OVERRIDE_REVERSE_BYPASS` is available, configure reverse bypass rules so only localhost-style hosts use the proxy; otherwise the Rust SOCKS proxy rejects non-loopback destinations.

This avoids HTTP absolute-form request rewriting and keeps the browser's origin
model intact. The SOCKS handshake gives the app the destination host and port,
which is enough to validate the tunnel and open a raw host TCP stream.

## Security

Keep the tunnel narrow:

- Only allow host loopback destinations: `localhost`, `127.0.0.0/8`, and `::1`.
- Reject SOCKS requests for non-loopback destinations.
- Bind app listeners to loopback only.
- Treat tunnel capability as part of the authenticated Zedra session, not as a public service.
- Log destination host and port and failures; do not log tunneled bytes.

## Caveats

Network layer:

- The tunnel forwards TCP only. It does not support UDP, WebRTC media paths, multicast discovery, mDNS, or a device-level VPN/TUN interface.
- `localhost` means host loopback after SOCKS validation, not mobile loopback. The mobile app binds only its SOCKS proxy on mobile loopback.
- Non-loopback resources requested by the page may be blocked by the tunnel policy unless the platform proxy configuration bypasses them directly.
- HTTPS works only when the WebView trusts the host server certificate. The tunnel does not terminate TLS or mint certificates.

Transport layer:

- Each browser connection maps to one `WebConnect` stream over the existing authenticated Zedra transport. QUIC stream backpressure and relay latency directly affect page load and HMR responsiveness.
- Keep byte chunks bounded. Large downloads should stream through chunks instead of accumulating whole bodies.
- A stream close/error should tear down only that browser connection, not the whole Zedra session.

Session layer:

- Web tunnels are tied to the current `SessionHandle`. Reconnect, workspace switch, app backgrounding, or host restart can drop open browser connections; the WebView should recover via reload.
- Android's WebView proxy override is process-global while enabled, so it must be cleared when the native WebView closes.
- iOS proxy configuration is attached to the WebView's `WKWebsiteDataStore`; set it before page load. Zedra uses iOS 17+ and a nonpersistent data store for tunneled WebViews.

## Implementation Notes

- Add `WebConnect` to `zedra-rpc` as a bidirectional streaming RPC.
- Keep `WebFetch` out of the browser path. It can remain a diagnostic helper, but it cannot support WebSocket/HMR/SSE semantics.
- Use byte frame structs such as `WebTunnelInput` and `WebTunnelOutput`, plus explicit connected/close/error signals so the app can send the correct SOCKS reply.
- On the app, bridge the accepted SOCKS stream to the irpc channels with separate read and write tasks.
- On the host, bridge the irpc channels to `tokio::net::TcpStream`.
- Prefer bounded channels and chunk limits to avoid unbounded buffering.
- Do not parse HTTP after the SOCKS handshake. Preserving the browser's bytes is what keeps WebSockets, streaming, upgrades, and keep-alive behavior intact.

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
- Android WebView proxy builder: `https://developer.android.com/reference/androidx/webkit/ProxyConfig.Builder`
- Android WebView resource interception limits: `https://developer.android.com/reference/android/webkit/WebViewClient#shouldInterceptRequest(android.webkit.WebView,android.webkit.WebResourceRequest)`
- WebKit data store proxy configurations: `https://developer.apple.com/documentation/webkit/wkwebsitedatastore/proxyconfigurations`
- Network framework SOCKSv5 proxy configuration: `nw_proxy_config_create_socksv5` in the iOS 17 SDK `Network.framework/Headers/proxy_config.h`
- SOCKS Protocol Version 5: `https://www.rfc-editor.org/rfc/rfc1928`
- WebSocket protocol: `https://www.rfc-editor.org/rfc/rfc6455`
