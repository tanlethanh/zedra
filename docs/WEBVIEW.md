# Webview

Present a native in-app webview and drive it from Rust. `crates/zedra/src/webview.rs` is the single reusable seam: callers describe what they want with `WebviewConfig` and react to the page through optional callbacks. The [web tunnel](WEB_TUNNEL.md) is one caller; anything that needs an embedded browser uses the same API.

## Goal

Open a webview from any feature without touching platform code, and customize its behavior from Rust: route traffic through a proxy, receive messages the page posts, intercept navigations, evaluate JavaScript on demand, and learn when it closes.

## Quick start

```rust
use zedra::webview::{self, WebviewConfig, NavigationPolicy};

webview::open(
    WebviewConfig::new("https://example.com")
        .title("Example")
        .on_message(|msg| tracing::info!("webview: page said: {msg}"))
        .on_navigate(|url| {
            if url.starts_with("https://") { NavigationPolicy::Allow } else { NavigationPolicy::Cancel }
        })
        .on_dismiss(|| tracing::info!("webview: closed")),
);
```

Drive the open webview:

```rust
webview::eval_js("document.body.style.background = 'black'");
webview::close();
```

## Config

`WebviewConfig::new(url)` starts with a non-persistent session (cookies and storage do not survive across opens). Chain the setters you need:

| Setter | Effect |
| --- | --- |
| `title(s)` | Navigation-bar title. Falls back to the page title when empty. |
| `socks_proxy(host, port)` | Route the webview's traffic through a SOCKS5 proxy (iOS 17+ `proxyConfigurations`, Android `ProxyController`). The web tunnel uses this to reach the host's loopback. |
| `inject_js(js)` | Run `js` in the main frame on every navigation, ahead of the page's own scripts where the platform allows. Must be idempotent — see below. |
| `hide_input_accessory(bool)` | Drop the keyboard's form accessory bar (the prev/next/Done strip). iOS only. |
| `on_message(fn)` | Receive messages the page posts through the `zedra` JS bridge. |
| `on_navigate(fn)` | Decide per navigation; return `NavigationPolicy::Cancel` to block. |
| `on_dismiss(fn)` | Fired once when the user dismisses the webview. |

Only one webview is presented at a time. Calling `open` again replaces the current one (the old one's `on_dismiss` fires first).

## Messaging

Set `on_message` to install a JS bridge named `zedra`. The page posts a string; Rust receives it. The entry point differs per platform:

- iOS: `window.webkit.messageHandlers.zedra.postMessage("hello")`
- Android: `window.zedra.postMessage("hello")`

A page that targets both papers over the difference with a one-liner:

```js
const post = (m) =>
  (window.webkit?.messageHandlers?.zedra?.postMessage ?? window.zedra?.postMessage)(m);
```

Send data the other way with `webview::eval_js` (for example, call a function the page defined).

## Navigation interception

When `on_navigate` is set, the native layer asks Rust before each navigation and blocks it when you return `NavigationPolicy::Cancel`. The handler runs synchronously on the native UI thread, so keep it fast — no blocking I/O.

`on_navigate` only sees real navigations. A single-page app changes route with `history.pushState`, which fires neither `decidePolicyFor` nor `shouldOverrideUrlLoading` — to follow those, inject a script that hooks the history API and posts back over the `zedra` bridge. The web tunnel does this to track where the user is inside a tunnelled app; see `web_tunnel::open_url_with`.

## Script injection

`inject_js` runs a script in the main frame on each navigation. The platforms differ in *when*, so the script must be idempotent (guard on a `window` flag):

- iOS: a `WKUserScript` at `.atDocumentStart` — before the page's own scripts, once per document.
- Android: `WebView` has no user-script API, so the script runs from both `onPageStarted` (which may precede a usable JS context) and `onPageFinished`. Injecting twice is why the guard is required.

Unlike `eval_js`, injection races nothing on iOS: it is installed before the page runs.

## Callback threading

Callbacks run on the native UI thread, off the GPUI thread. They are not handed an `App`. Keep them cheap, and do not re-enter `webview` (e.g. do not call `open` from inside a handler). To update GPUI state, hop back through a `PendingSlot` or a channel rather than touching `App` directly.

## Architecture

```text
feature code
  -> webview::open(WebviewConfig)            # serializes data to JSON, stores callbacks under an id
  -> platform_bridge::open_webview(id, json) # thin trait method
        iOS:     ios_open_webview            -> NativeWebViewController (WKWebView)
        Android: openWebView                 -> NativePresentations (android.webkit.WebView)
  <- zedra_ios_webview_* / nativeWebView*    # native calls back with the id
  <- webview::dispatch_{message,navigate,dismiss}
```

The config crosses the FFI boundary as a JSON string, so adding a knob is a `WebviewConfig` field plus its native parse — no ABI churn. Callbacks never cross FFI: they live in a Rust registry keyed by `callback_id`, and the native side only learns which capabilities to wire up (`messageHandlerName`, `interceptNavigation`).

### Adding a config field

1. Add the field and setter to `WebviewConfig`, and the wire field to `WireConfig` in `webview.rs`.
2. Read it on iOS in `NativeWebViewConfig` (`ios/Zedra/Presentations.swift`) and apply it.
3. Read it on Android in `parseWebViewConfig` (`android/.../NativePresentations.kt`) and apply it.

### Adding a new callback

Mirror the message/navigate/dismiss path: a `dispatch_*` in `webview.rs`, a `zedra_ios_webview_*` `extern "C"` in `crates/zedra/src/ios/bridge.rs` (declared in `include/zedra_ios.h`, called from Swift), and a `nativeWebView*` JNI export in `crates/zedra/src/android/jni.rs` (declared `external fun` in `MainActivity.kt`, called from Kotlin).

## Platform notes

- iOS uses a non-persistent `WKWebsiteDataStore`, a `WKScriptMessageHandler` for messages, and `decidePolicyFor` for navigation. Proxy uses Network framework `ProxyConfiguration(socksv5Proxy:)`; iOS 17+.
- Android uses `addJavascriptInterface` for messages and `WebViewClient.shouldOverrideUrlLoading` for navigation. Proxy uses AndroidX `ProxyController.setProxyOverride`, which is process-global while enabled and is cleared when the webview closes. Cleartext HTTP is scoped to loopback hosts via `res/xml/network_security_config.xml`.

## Manual test

See `docs/MANUAL_TEST.md` for device steps. In short: open a webview from a feature, confirm the page loads, that `on_message` fires when the page posts, that `eval_js` runs, that `on_navigate` can block a link, and that `on_dismiss` fires on close.
