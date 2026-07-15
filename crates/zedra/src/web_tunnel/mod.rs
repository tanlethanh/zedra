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
    let Ok(port) = parse_loopback_target(url) else {
        tracing::info!("[debug:web-tunnel] {url} not host-local -> system browser");
        platform_bridge::bridge().open_url(url);
        return None;
    };
    let (Some(endpoint_id), Ok(runtime)) = (session_handle.endpoint_id(), session_handle.runtime())
    else {
        tracing::info!("[debug:web-tunnel] no session endpoint -> system browser: {url}");
        platform_bridge::bridge().open_url(url);
        return None;
    };
    tracing::info!("[debug:web-tunnel] open {url} (loopback :{port}) via {endpoint_id}");
    registry()
        .sessions
        .lock()
        .unwrap()
        .insert(endpoint_id, session_handle);
    let title = webview_title(url);
    let spawn_url = url.to_string();
    let spawn_title = title.clone();
    runtime.spawn(async move { serve(endpoint_id, spawn_url, spawn_title, port).await });
    Some(title)
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

/// `host:port` label for a trackable loopback target, or `None` when `url` is
/// not a host-local http(s) target (those open in the system browser, untracked).
pub fn loopback_title(url: &str) -> Option<String> {
    parse_loopback_target(url).ok().map(|_| webview_title(url))
}

async fn serve(endpoint_id: PublicKey, url: String, title: String, port: u16) {
    if registry().prefs.lock().unwrap().get(&endpoint_id) == Some(&AdapterKind::Alias) {
        serve_alias(endpoint_id, &url, title).await;
        return;
    }
    match exact_port::ensure(endpoint_id, port).await {
        Ok(()) => {
            crate::webview::open(crate::webview::WebviewConfig::new(url).title(title));
        }
        Err(()) => prompt_alias_fallback(endpoint_id, url, title, port),
    }
}

async fn serve_alias(endpoint_id: PublicKey, url: &str, title: String) {
    let proxy_port = match alias::ensure_proxy().await {
        Ok(port) => port,
        Err(error) => {
            tracing::warn!("[debug:web-tunnel] alias proxy setup failed: {error}");
            notify_failed(&error);
            return;
        }
    };
    crate::webview::open(
        crate::webview::WebviewConfig::new(alias_url(url, &endpoint_id))
            .title(title)
            .socks_proxy(Ipv4Addr::LOCALHOST, proxy_port),
    );
}

/// Surface the exact-port failure and let the user opt this host into the alias.
fn prompt_alias_fallback(endpoint_id: PublicKey, url: String, title: String, port: u16) {
    tracing::info!("[debug:web-tunnel] exact-port unavailable for :{port}, offering alias");
    let message = format!(
        "localhost:{port} can't be bound on this device (another host or app holds it). \
         Tap to route this workspace through an alias instead. Learn more: {MODES_DOC_URL}"
    );
    platform_bridge::show_native_notification_with_action(
        NativeNotificationOptions::new("Web tunnel: port unavailable")
            .message(message)
            .kind(NativeNotificationKind::Warning),
        move || approve_alias(endpoint_id, url, title),
    );
}

/// The user approved the alias for this host: remember it and serve now.
fn approve_alias(endpoint_id: PublicKey, url: String, title: String) {
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
    runtime.spawn(async move { serve_alias(endpoint_id, &url, title).await });
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
    let _ = parsed.set_host(Some(&host));
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
