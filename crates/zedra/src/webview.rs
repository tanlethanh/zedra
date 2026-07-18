//! Generic native in-app webview.
//!
//! This is the single reusable seam for presenting a native webview (WKWebView
//! on iOS, `android.webkit.WebView` on Android) and driving it from Rust. It
//! owns nothing use-case specific: callers describe what they want with
//! [`WebviewConfig`] and react to the page through optional callbacks. The web
//! tunnel is one caller; see [`crate::web_tunnel`].
//!
//! Flow:
//! - [`open`] serializes the config to JSON, stores its callbacks under a fresh
//!   id, and asks the platform bridge to present the webview.
//! - The native layer calls back into [`dispatch_message`],
//!   [`dispatch_navigate`], and [`dispatch_dismiss`] with that id.
//! - [`eval_js`] and [`close`] drive the currently presented webview.
//!
//! Callbacks run on the native UI thread, off the GPUI thread. Keep them cheap
//! and do not re-enter `webview` (e.g. do not call [`open`] from inside a
//! handler). To update GPUI state, hop back through a `PendingSlot` or a
//! channel rather than touching `App` directly.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde::Serialize;

use crate::platform_bridge;

/// Decision returned from a navigation interception callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavigationPolicy {
    /// Let the webview load the navigation.
    Allow,
    /// Block the navigation. The page stays where it is.
    Cancel,
}

/// JS bridge name installed when a message handler is set. The page posts to
/// `window.webkit.messageHandlers.zedra` (iOS) or `window.zedra` (Android).
pub const MESSAGE_HANDLER_NAME: &str = "zedra";

type MessageHandler = Arc<dyn Fn(String) + Send + Sync>;
type NavigationHandler = Arc<dyn Fn(&str) -> NavigationPolicy + Send + Sync>;
type DismissHandler = Box<dyn FnOnce() + Send>;

struct Handlers {
    on_message: Option<MessageHandler>,
    on_navigate: Option<NavigationHandler>,
    on_dismiss: Option<DismissHandler>,
}

static NEXT_ID: AtomicU32 = AtomicU32::new(1);
static HANDLERS: OnceLock<Mutex<HashMap<u32, Handlers>>> = OnceLock::new();

fn handlers() -> &'static Mutex<HashMap<u32, Handlers>> {
    HANDLERS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Description of a native webview to present. Build with [`WebviewConfig::new`]
/// and chain the setters you need. Storage is non-persistent: each open starts
/// from a clean, private session.
pub struct WebviewConfig {
    url: String,
    title: String,
    socks_proxy: Option<String>,
    on_message: Option<MessageHandler>,
    on_navigate: Option<NavigationHandler>,
    on_dismiss: Option<DismissHandler>,
}

impl WebviewConfig {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            title: String::new(),
            socks_proxy: None,
            on_message: None,
            on_navigate: None,
            on_dismiss: None,
        }
    }

    /// Navigation-bar title. Falls back to the page title when empty.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// Route the webview's traffic through a SOCKS5 proxy at `host:port`
    /// (iOS 17+ `proxyConfigurations`, Android `ProxyController`). The web tunnel
    /// uses this to reach the host's loopback; see [`crate::web_tunnel`].
    pub fn socks_proxy(mut self, host: impl std::fmt::Display, port: u16) -> Self {
        self.socks_proxy = Some(format!("{host}:{port}"));
        self
    }

    /// Receive messages the page posts through the JS bridge named
    /// [`MESSAGE_HANDLER_NAME`].
    pub fn on_message(mut self, handler: impl Fn(String) + Send + Sync + 'static) -> Self {
        self.on_message = Some(Arc::new(handler));
        self
    }

    /// Intercept navigations. Return [`NavigationPolicy::Cancel`] to block a
    /// load. Runs synchronously on the native UI thread, so keep it fast.
    pub fn on_navigate(
        mut self,
        handler: impl Fn(&str) -> NavigationPolicy + Send + Sync + 'static,
    ) -> Self {
        self.on_navigate = Some(Arc::new(handler));
        self
    }

    /// Called once when the user dismisses the webview.
    pub fn on_dismiss(mut self, handler: impl FnOnce() + Send + 'static) -> Self {
        self.on_dismiss = Some(Box::new(handler));
        self
    }
}

/// Data half of [`WebviewConfig`] sent to the native layer. Callbacks stay in
/// the Rust registry; the native side only learns which capabilities to wire up.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireConfig<'a> {
    url: &'a str,
    title: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    socks_proxy: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message_handler_name: Option<&'a str>,
    intercept_navigation: bool,
}

/// Present the webview described by `config`. Returns the id the native layer
/// uses for callbacks. Any previously open webview is replaced.
pub fn open(config: WebviewConfig) -> u32 {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);

    let wire = WireConfig {
        url: &config.url,
        title: &config.title,
        socks_proxy: config.socks_proxy.as_deref(),
        message_handler_name: config.on_message.as_ref().map(|_| MESSAGE_HANDLER_NAME),
        intercept_navigation: config.on_navigate.is_some(),
    };
    let config_json = serde_json::to_string(&wire).unwrap_or_default();

    handlers().lock().unwrap().insert(
        id,
        Handlers {
            on_message: config.on_message,
            on_navigate: config.on_navigate,
            on_dismiss: config.on_dismiss,
        },
    );

    platform_bridge::bridge().open_webview(id, &config.url, &config_json);
    id
}

/// Dismiss the currently presented webview, if any.
pub fn close() {
    platform_bridge::bridge().close_webview();
}

/// Evaluate `js` in the currently presented webview. No-op when none is open.
pub fn eval_js(js: &str) {
    platform_bridge::bridge().eval_webview_js(js);
}

/// Deliver a message the page posted through the JS bridge. Called by the
/// native layer.
pub fn dispatch_message(id: u32, message: String) {
    let handler = handlers()
        .lock()
        .unwrap()
        .get(&id)
        .and_then(|handlers| handlers.on_message.clone());
    if let Some(handler) = handler {
        handler(message);
    }
}

/// Ask Rust whether a navigation to `url` should proceed. Called by the native
/// layer synchronously; returns `true` to allow. Allows by default when no
/// handler is registered.
pub fn dispatch_navigate(id: u32, url: &str) -> bool {
    let handler = handlers()
        .lock()
        .unwrap()
        .get(&id)
        .and_then(|handlers| handlers.on_navigate.clone());
    match handler {
        Some(handler) => handler(url) == NavigationPolicy::Allow,
        None => true,
    }
}

/// Report that the webview was dismissed. Drops its handlers and fires
/// `on_dismiss`. Called by the native layer.
pub fn dispatch_dismiss(id: u32) {
    let entry = handlers().lock().unwrap().remove(&id);
    if let Some(handler) = entry.and_then(|h| h.on_dismiss) {
        handler();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_callbacks_run_without_holding_the_registry_lock() {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        handlers().lock().unwrap().insert(
            id,
            Handlers {
                on_message: Some(Arc::new(|_| {
                    assert!(handlers().try_lock().is_ok());
                })),
                on_navigate: Some(Arc::new(|_| {
                    assert!(handlers().try_lock().is_ok());
                    NavigationPolicy::Allow
                })),
                on_dismiss: None,
            },
        );

        dispatch_message(id, String::new());
        assert!(dispatch_navigate(id, "https://example.com"));
        handlers().lock().unwrap().remove(&id);
    }
}
