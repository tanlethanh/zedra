# Web Tunnel Modes

The web tunnel opens a host's localhost web app in the in-app webview over the
authenticated Zedra session. It has two adapters behind one seam, picked per
host and keyed by the session's stable non-relay **endpoint id** (so routing
survives reconnects). Default is **exact-port**; **alias** is an opt-in
fallback. Implementation: `crates/zedra/src/web_tunnel/`.

## Why two modes

A webview can only reach a host's `localhost` two ways, and only one preserves
the literal `localhost` origin:

1. **Actually listen on the port** on the device (`127.0.0.1:<port>`), so the
   webview loads the unmodified `http://localhost:<port>`. The origin genuinely
   *is* `localhost` — CORS, cookies, and `localhost`-registered OAuth behave
   exactly as they do locally. This is what Chrome DevTools / VS Code port
   forwarding do.
2. **Address the host by a non-loopback name** and route it through a proxy. On
   a **real iOS device**, WKWebView's `proxyConfigurations` bypasses the proxy
   for *all* loopback (`localhost`, every `127.0.0.0/8`, `::1`) — verified across
   the range on iOS 26.5 — and no public knob overrides it. Only a genuinely
   non-loopback host routes. So a proxy tunnel requires a synthetic hostname.

Neither is strictly better; they trade origin honesty for multi-host reach.

## Exact-port (default)

- Binds `127.0.0.1:<port>` on the device; the webview loads the literal
  `http://localhost:<port>`. Companion ports the page references are discovered
  by a plaintext byte sniffer and bound the same way.
- Each device port is owned by **one host** (endpoint id). A second host
  colliding on the same port — or a hard bind failure (another app holds the
  port, or an OS restriction) — is "exact-port unavailable" and triggers the
  fallback prompt.
- Origin stays `localhost` → no CORS/OAuth/navigation surprises.

## Alias (opt-in fallback)

- Serves the host under a per-host alias `**<label>.zedra.test**` (`label` =
  short prefix of the endpoint id) via an ephemeral in-app SOCKS5 proxy the
  webview is pointed at with `proxyConfigurations`. The proxy decodes the label
  back to the endpoint id and forwards over that host's `WebConnect`; external
  (non-alias) hosts are dialed directly so links still work.
- Because the alias is non-loopback and per-host, it **routes on device** and
  **disambiguates concurrent hosts** on the same port — the two things exact-port
  can't do.
- The address bar shows the **real alias** (not a spoofed `localhost`): the
  origin is honest, so behavior is predictable — but it is *not* `localhost`.
  Consequences to expect:
  - A page hardcoding `http://localhost:<other>` internally still bypasses the
    proxy (device loopback rule); relative / same-origin URLs work.
  - Cross-port requests are cross-origin (as with real subdomains).
  - OAuth `redirect_uri` and cookies registered for `localhost` won't match the
    alias origin — register the alias, or use exact-port for those flows.
- `.test` is a reserved, non-resolving TLD; with SOCKS remote DNS the alias
  never hits a real resolver. Cleartext is allowed via `Info.plist`
  `NSExceptionDomains` (iOS) and `network_security_config.xml` (Android).

## Orchestration

`web_tunnel::open_url` (called from a tapped terminal link):

1. Non-loopback / non-http(s) targets → system browser.
2. Resolve the host endpoint id; if the host already opted into the alias, use it.
3. Otherwise try exact-port. Success → load literal `localhost`.
4. Unavailable → a native notice ("localhost:PORT can't be bound on this
   device") with a **Use alias** action and a link to this doc. Approving records
   the choice **for that host** and serves via the alias from then on.

## Choosing

| | Exact-port (default) | Alias (opt-in) |
|---|---|---|
| Origin | honest `localhost` | honest `<label>.zedra.test` |
| CORS / cookies / `localhost` OAuth | work unchanged | need the alias origin configured |
| Same-port, multiple hosts concurrently | one host per port | each host disambiguated |
| Port already taken (app/OS) | fails → offers alias | works (one ephemeral proxy port) |
| Companion ports | sniffer binds them | same-origin/relative only |

Use exact-port for origin-sensitive apps (sign-in, cookies, strict CORS); use
the alias when a port can't be bound or you need concurrent same-port hosts.

## Backlog

- **Listener manager** (deferred): exact-port listeners live for the app's
  process lifetime and can't be stopped. Add a settings view
  (`crates/zedra/src/web_tunnel_manager.rs`) that lists active listeners
  (port → owning host) with a **Stop** action to release a port when it conflicts
  with another app, and optionally switch a host to alias mode. Needs
  `exact_port` to hold a per-listener shutdown handle and clear its owner.
