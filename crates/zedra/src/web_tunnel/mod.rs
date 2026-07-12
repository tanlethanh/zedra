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
pub fn open_url(session_handle: SessionHandle, url: &str) {
    let Ok(port) = parse_loopback_target(url) else {
        tracing::info!("[debug:web-tunnel] {url} not host-local -> system browser");
        platform_bridge::bridge().open_url(url);
        return;
    };
    let (Some(endpoint_id), Ok(runtime)) = (session_handle.endpoint_id(), session_handle.runtime())
    else {
        tracing::info!("[debug:web-tunnel] no session endpoint -> system browser: {url}");
        platform_bridge::bridge().open_url(url);
        return;
    };
    tracing::info!("[debug:web-tunnel] open {url} (loopback :{port}) via {endpoint_id}");
    registry()
        .sessions
        .lock()
        .unwrap()
        .insert(endpoint_id, session_handle);
    let url = url.to_string();
    let title = webview_title(&url);
    runtime.spawn(async move { serve(endpoint_id, url, title, port).await });
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
