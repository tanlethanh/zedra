//! Web tunnel: open a host's localhost web app in the in-app webview over the
//! authenticated Zedra session. Two adapters behind one seam, chosen per host:
//!
//! - [`exact_port`] (default): bind a real `127.0.0.1:<port>` listener so the
//!   webview loads the unmodified `http://localhost:<port>` — honest loopback
//!   origin, so CORS/cookies/OAuth behave. One host owns a port at a time.
//! - [`alias`] (opt-in fallback): when a port can't be bound (another app, or a
//!   second host colliding on the same port), route this host through a
//!   `<label>.zedra.test` alias + in-app SOCKS proxy. A real device bypasses the
//!   proxy for loopback, so a non-loopback alias is the only proxy-viable form.
//!
//! Routing is keyed by the session's stable non-relay endpoint id, so bound
//! listeners and aliases survive reconnects. See `docs/WEB_TUNNEL_MODES.md`.

mod alias;
mod bridge;
mod exact_port;

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::{Mutex, OnceLock};

use iroh::PublicKey;
use reqwest::Url;
use zedra_rpc::proto::is_loopback_host;
use zedra_session::SessionHandle;

use crate::platform_bridge::{self, NativeNotificationKind, NativeNotificationOptions};

/// Live exact-port listeners + a stop control, for the manager view
/// (`web_tunnel_manager.rs`) to free a device port that conflicts with another app.
pub(crate) use exact_port::{
    ListenerInfo, list_listeners as active_listeners, stop as stop_listener,
};

/// External write-up of the two modes and their origin tradeoffs.
const MODES_DOC_URL: &str = "https://zedra.dev/docs/web-tunnel-modes";

/// Called with the page's current path (+ query) whenever it changes. See
/// [`open_url_with`].
pub type RouteHook = std::sync::Arc<dyn Fn(String) + Send + Sync>;

/// Turn off the keyboard's QuickType suggestion strip for the page's editable
/// fields. The strip belongs to the keyboard, not the webview, so there is no
/// bar to remove: WebKit maps these attributes onto each field's UIKit text
/// traits, which is the only lever short of private API. A web app mounts its
/// composer after load and may swap it per route, so new fields are caught as
/// they appear.
const DISABLE_AUTOCORRECT_JS: &str = r#"
(() => {
  if (window.__zedraAutocorrectOff) return;
  window.__zedraAutocorrectOff = true;
  const SELECTOR = "input, textarea, [contenteditable]";
  const off = (el) => {
    if (el.dataset.zedraAutocorrectOff) return;
    el.dataset.zedraAutocorrectOff = "1";
    el.setAttribute("autocorrect", "off");
    el.setAttribute("autocapitalize", "off");
    el.setAttribute("spellcheck", "false");
  };
  const scan = (node) => {
    if (!(node instanceof Element)) return;
    if (node.matches(SELECTOR)) off(node);
    node.querySelectorAll(SELECTOR).forEach(off);
  };
  const start = () => {
    scan(document.body);
    // Only walk nodes as they mount; a full re-sweep per mutation would run on
    // every streamed token of an agent's reply.
    new MutationObserver((records) => {
      for (const record of records) for (const node of record.addedNodes) scan(node);
    }).observe(document.documentElement, { childList: true, subtree: true });
  };
  if (document.body) start();
  else addEventListener("DOMContentLoaded", start, { once: true });
})();
"#;

/// Report the page's route to Rust over the `zedra` bridge. A single-page app
/// changes route with `history.pushState`, which fires no navigation the native
/// layer can intercept, so hook the history API itself. Injected more than once
/// per document on Android, so the hook guards against wrapping twice.
const ROUTE_REPORTER_JS: &str = r#"
(() => {
  if (window.__zedraRouteHooked) return;
  window.__zedraRouteHooked = true;
  const post = (m) => {
    if (window.webkit?.messageHandlers?.zedra) window.webkit.messageHandlers.zedra.postMessage(m);
    else if (window.zedra) window.zedra.postMessage(m);
  };
  let last = null;
  const report = () => {
    const path = location.pathname + location.search;
    if (path === last) return;
    last = path;
    post(path);
  };
  for (const name of ["pushState", "replaceState"]) {
    const original = history[name];
    history[name] = function (...args) {
      const result = original.apply(this, args);
      report();
      return result;
    };
  }
  addEventListener("popstate", report);
  addEventListener("load", report);
  report();
})();
"#;

#[derive(Clone, Copy, PartialEq, Eq)]
enum AdapterKind {
    Alias,
}

struct Registry {
    sessions: Mutex<HashMap<PublicKey, SessionHandle>>,
    prefs: Mutex<HashMap<PublicKey, AdapterKind>>,
}

fn registry() -> &'static Registry {
    static R: OnceLock<Registry> = OnceLock::new();
    R.get_or_init(|| Registry {
        sessions: Mutex::new(HashMap::new()),
        prefs: Mutex::new(HashMap::new()),
    })
}

/// The latest session for a host, resolved by its stable endpoint id. Both
/// adapters route through this so listeners/aliases survive reconnects (the
/// session handle changes on reconnect; the endpoint id stays).
fn session_for(endpoint_id: &PublicKey) -> Option<SessionHandle> {
    registry()
        .sessions
        .lock()
        .unwrap()
        .get(endpoint_id)
        .cloned()
}

/// Open `url` the best way: host-local http(s) URLs load in the in-app webview
/// (exact-port by default, alias on a per-host opt-in fallback); anything else
/// (or an unparseable target) falls back to the system browser. Single "open a
/// link" seam for the tunnel.
/// Returns the `host:port` label for a trackable loopback target (so the caller
/// can record it for quick reopen), or `None` when `url` opened in the system
/// browser instead.
pub fn open_url(session_handle: SessionHandle, url: &str) -> Option<String> {
    open_url_with(session_handle, url, None)
}

/// [`open_url`], plus an optional `on_route` hook fired with the page's path
/// whenever it navigates — including single-page-app `pushState` routes, which
/// no native navigation callback sees. Runs on the native UI thread; keep it
/// cheap. Only applies to the in-app webview, not the system-browser fallback.
pub fn open_url_with(
    session_handle: SessionHandle,
    url: &str,
    on_route: Option<RouteHook>,
) -> Option<String> {
    // Log only the origin, never the raw URL: the path/query of a user's local
    // web app can carry session tokens or OAuth params.
    let origin = webview_title(url);
    let Ok(port) = parse_loopback_target(url) else {
        tracing::info!("web-tunnel: {origin} not host-local -> system browser");
        platform_bridge::bridge().open_url(url);
        return None;
    };
    let (Some(endpoint_id), Ok(runtime)) = (session_handle.endpoint_id(), session_handle.runtime())
    else {
        tracing::info!("web-tunnel: no session endpoint -> system browser: {origin}");
        platform_bridge::bridge().open_url(url);
        return None;
    };
    tracing::info!("web-tunnel: open {origin} (loopback :{port}) via {endpoint_id}");
    registry()
        .sessions
        .lock()
        .unwrap()
        .insert(endpoint_id, session_handle);
    let spawn_url = url.to_string();
    let spawn_title = origin.clone();
    runtime.spawn(async move { serve(endpoint_id, spawn_url, spawn_title, port, on_route).await });
    Some(origin)
}

/// Turn manual input into a URL: an explicit scheme passes through, a bare port
/// or `host:port` becomes an `http://` loopback target. Empty input stays empty.
pub fn normalize_target(input: &str) -> String {
    let input = input.trim();
    if input.is_empty() {
        return String::new();
    }
    if input.contains("://") {
        return input.to_string();
    }
    if input.parse::<u16>().is_ok() {
        return format!("http://localhost:{input}");
    }
    format!("http://{input}")
}

async fn serve(
    endpoint_id: PublicKey,
    url: String,
    title: String,
    port: u16,
    on_route: Option<RouteHook>,
) {
    // The alias rewrites the webview host to `<word>.zedra.test`, which changes
    // the TLS SNI; an https host still presents its `localhost` certificate, so
    // validation fails. Alias mode therefore only serves cleartext http.
    let alias_ok = is_cleartext_http(&url);
    if registry().prefs.lock().unwrap().get(&endpoint_id) == Some(&AdapterKind::Alias) {
        if alias_ok {
            serve_alias(endpoint_id, &url, title, on_route).await;
        } else {
            notify_https_needs_exact_port(port);
        }
        return;
    }
    match exact_port::ensure(endpoint_id, port).await {
        Ok(()) => {
            crate::webview::open(tunnel_webview(url, title, on_route));
        }
        Err(()) if alias_ok => prompt_alias_fallback(endpoint_id, url, title, port, on_route),
        Err(()) => notify_https_needs_exact_port(port),
    }
}

/// The webview both adapters present: a tunnelled page the user drives with its
/// own UI, so the keyboard carries no chrome of ours, plus route reporting when
/// the caller asked for it.
fn tunnel_webview(
    url: String,
    title: String,
    on_route: Option<RouteHook>,
) -> crate::webview::WebviewConfig {
    let config = crate::webview::WebviewConfig::new(url)
        .title(title)
        .hide_input_accessory(true);
    let mut js = DISABLE_AUTOCORRECT_JS.to_string();
    match on_route {
        Some(hook) => {
            js.push_str(ROUTE_REPORTER_JS);
            config.inject_js(js).on_message(move |path| hook(path))
        }
        None => config.inject_js(js),
    }
}

fn is_cleartext_http(url: &str) -> bool {
    Url::parse(url)
        .map(|u| u.scheme() == "http")
        .unwrap_or(false)
}

fn notify_https_needs_exact_port(port: u16) {
    tracing::info!("web-tunnel: port :{port} unavailable and https can't use the alias");
    platform_bridge::show_native_notification(
        NativeNotificationOptions::new("Web tunnel: port unavailable")
            .message(format!(
                "localhost:{port} can't be bound on this device, and the alias fallback \
                 can't serve https (its certificate is for localhost). Free the port to load this page."
            ))
            .kind(NativeNotificationKind::Warning),
    );
}

async fn serve_alias(
    endpoint_id: PublicKey,
    url: &str,
    title: String,
    on_route: Option<RouteHook>,
) {
    let proxy_port = match alias::ensure_proxy().await {
        Ok(port) => port,
        Err(error) => {
            tracing::warn!("web-tunnel: alias proxy setup failed: {error}");
            notify_failed(&error);
            return;
        }
    };
    crate::webview::open(
        tunnel_webview(alias_url(url, &endpoint_id), title, on_route)
            .socks_proxy(Ipv4Addr::LOCALHOST, proxy_port),
    );
}

/// Surface the exact-port failure and let the user opt this host into the alias.
fn prompt_alias_fallback(
    endpoint_id: PublicKey,
    url: String,
    title: String,
    port: u16,
    on_route: Option<RouteHook>,
) {
    tracing::info!("web-tunnel: exact-port unavailable for :{port}, offering alias");
    let message = format!(
        "localhost:{port} can't be bound on this device (another host or app holds it). \
         Tap to route this workspace through an alias instead. Learn more: {MODES_DOC_URL}"
    );
    platform_bridge::show_native_notification_with_action(
        NativeNotificationOptions::new("Web tunnel: port unavailable")
            .message(message)
            .kind(NativeNotificationKind::Warning),
        move || approve_alias(endpoint_id, url, title, on_route),
    );
}

/// The user approved the alias for this host: remember it and serve now.
fn approve_alias(endpoint_id: PublicKey, url: String, title: String, on_route: Option<RouteHook>) {
    registry()
        .prefs
        .lock()
        .unwrap()
        .insert(endpoint_id, AdapterKind::Alias);
    let Some(session) = session_for(&endpoint_id) else {
        return;
    };
    let Ok(runtime) = session.runtime() else {
        return;
    };
    runtime.spawn(async move { serve_alias(endpoint_id, &url, title, on_route).await });
}

fn notify_failed(error: &str) {
    platform_bridge::show_native_notification(
        NativeNotificationOptions::new("Web tunnel failed")
            .message(error.to_string())
            .kind(NativeNotificationKind::Error),
    );
}

fn parse_loopback_target(url: &str) -> Result<u16, ()> {
    let parsed = Url::parse(url).map_err(|_| ())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(());
    }
    let host = parsed.host_str().ok_or(())?;
    if !is_loopback_host(host) {
        return Err(());
    }
    parsed.port_or_known_default().ok_or(())
}

/// Rewrite the URL host to this host's alias, preserving scheme/port/path.
fn alias_url(url: &str, endpoint_id: &PublicKey) -> String {
    let Ok(mut parsed) = Url::parse(url) else {
        return url.to_string();
    };
    let host = alias::alias_host(endpoint_id);
    if let Err(error) = parsed.set_host(Some(&host)) {
        tracing::warn!("web-tunnel: alias host rewrite failed: {error}");
        return url.to_string();
    }
    parsed.to_string()
}

fn webview_title(url: &str) -> String {
    let Ok(parsed) = Url::parse(url) else {
        return String::new();
    };
    match (parsed.host_str(), parsed.port()) {
        (Some(host), Some(port)) => format!("{host}:{port}"),
        (Some(host), None) => host.to_string(),
        _ => String::new(),
    }
}

// Devtool hooks (debug builds only): reproduce adapter cases without contriving
// two real hosts. See docs/WEB_TUNNEL_MODES.md.
#[cfg(debug_assertions)]
pub(crate) fn debug_reset() {
    registry().prefs.lock().unwrap().clear();
    exact_port::debug_clear_owners();
}

#[cfg(debug_assertions)]
pub(crate) fn debug_force_alias(session: &SessionHandle) {
    if let Some(id) = session.endpoint_id() {
        registry()
            .prefs
            .lock()
            .unwrap()
            .insert(id, AdapterKind::Alias);
    }
}

#[cfg(debug_assertions)]
pub(crate) fn debug_collide(port: u16) {
    exact_port::debug_mark_foreign(port);
}

#[cfg(test)]
mod tests {
    use super::{normalize_target, parse_loopback_target, webview_title};

    #[test]
    fn parse_loopback_accepts_localhost_and_loopback_ips() {
        assert_eq!(parse_loopback_target("http://localhost:5173"), Ok(5173));
        assert_eq!(parse_loopback_target("http://127.0.0.1:8080"), Ok(8080));
        // The whole 127.0.0.0/8 block is loopback.
        assert_eq!(parse_loopback_target("http://127.0.0.5:3000"), Ok(3000));
        assert_eq!(parse_loopback_target("http://LOCALHOST:9000"), Ok(9000));
    }

    #[test]
    fn parse_loopback_fills_default_port_by_scheme() {
        assert_eq!(parse_loopback_target("http://localhost"), Ok(80));
        assert_eq!(parse_loopback_target("https://localhost"), Ok(443));
    }

    #[test]
    fn parse_loopback_rejects_non_loopback_and_other_schemes() {
        assert_eq!(parse_loopback_target("http://example.com:8080"), Err(()));
        assert_eq!(parse_loopback_target("http://192.168.1.5:8080"), Err(()));
        assert_eq!(parse_loopback_target("http://10.0.0.1:80"), Err(()));
        assert_eq!(parse_loopback_target("ftp://localhost:21"), Err(()));
        assert_eq!(parse_loopback_target("ws://localhost:5173"), Err(()));
        assert_eq!(parse_loopback_target("not a url"), Err(()));
        // IPv6 loopback literals arrive bracketed from the URL parser, which
        // IpAddr::parse rejects, so `[::1]` is treated as non-loopback today.
        assert_eq!(parse_loopback_target("http://[::1]:9000"), Err(()));
    }

    #[test]
    fn webview_title_formats_host_and_optional_port() {
        assert_eq!(webview_title("http://localhost:5173"), "localhost:5173");
        assert_eq!(webview_title("http://localhost"), "localhost");
        assert_eq!(webview_title("https://localhost"), "localhost");
        assert_eq!(webview_title("http://127.0.0.1:8080"), "127.0.0.1:8080");
        // The URL parser strips a scheme-default port, so it is not shown.
        assert_eq!(webview_title("https://localhost:443"), "localhost");
        assert_eq!(webview_title("garbage"), "");
    }

    #[test]
    fn normalize_target_maps_bare_targets_to_http_loopback() {
        assert_eq!(normalize_target("5173"), "http://localhost:5173");
        assert_eq!(normalize_target("  8080  "), "http://localhost:8080");
        assert_eq!(normalize_target("localhost:5173"), "http://localhost:5173");
        assert_eq!(normalize_target("127.0.0.1:3000"), "http://127.0.0.1:3000");
        assert_eq!(normalize_target("example.com"), "http://example.com");
    }

    #[test]
    fn normalize_target_passes_through_explicit_schemes_and_empty() {
        assert_eq!(
            normalize_target("http://localhost:5173/app"),
            "http://localhost:5173/app"
        );
        assert_eq!(
            normalize_target("https://localhost:8443"),
            "https://localhost:8443"
        );
        assert_eq!(normalize_target(""), "");
        assert_eq!(normalize_target("   "), "");
        // A number too large for a port is treated as a hostname, not a port.
        assert_eq!(normalize_target("70000"), "http://70000");
    }
}
