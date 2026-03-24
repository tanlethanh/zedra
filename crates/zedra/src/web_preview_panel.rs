/// Web preview panel for the workspace drawer.
///
/// Lets the developer tap a port preset and tap "Open Preview" to tunnel
/// that port through the active iroh session and open it in the device browser.
///
/// Flow:
///   1. User selects port (e.g. 3000) or keeps the default
///   2. Taps "Open Preview"
///   3. We start a `TcpProxyServer` on a random loopback port
///   4. We call `platform_bridge::bridge().open_url("http://127.0.0.1:<proxy_port>/")` which
///      hands the URL to the system browser / in-app WebView on the platform
///   5. The proxy server stays alive (held in panel state) until the user changes
///      the port or taps "Stop Proxy"
use gpui::prelude::FluentBuilder as _;
use gpui::*;

use crate::platform_bridge;
use crate::theme;
use crate::workspace_drawer::WorkspaceDrawer;
use zedra_session::{SessionHandle, TcpProxyServer};

pub struct WebPreviewState {
    /// The port text the user selected.
    pub port_input: String,
    /// Running proxy server, if one was started for the current port.
    pub proxy: Option<TcpProxyServer>,
    /// Error message to display if the proxy failed to start.
    pub error: Option<String>,
}

impl Default for WebPreviewState {
    fn default() -> Self {
        Self {
            port_input: "3000".to_string(),
            proxy: None,
            error: None,
        }
    }
}

/// Render the Web Preview tab content for the workspace drawer.
pub fn render_web_preview_tab(
    state: &mut WebPreviewState,
    handle: Option<&SessionHandle>,
    cx: &mut Context<WorkspaceDrawer>,
) -> impl IntoElement {
    let is_connected = handle.map(|h| h.is_connected()).unwrap_or(false);

    let proxy_port = state.proxy.as_ref().map(|p| p.local_port());
    let error_text = state.error.clone();
    let port_label = state.port_input.clone();
    let has_proxy = proxy_port.is_some();

    div()
        .size_full()
        .flex()
        .flex_col()
        .px(px(theme::DRAWER_PADDING))
        .gap(px(12.0))
        .child(
            div()
                .pt(px(8.0))
                .text_size(px(theme::FONT_DETAIL))
                .text_color(rgb(theme::TEXT_MUTED))
                .child("Preview a local dev server running on your host machine."),
        )
        .child(
            // ── Port row ─────────────────────────────────────────────────
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(
                    div()
                        .text_size(px(theme::FONT_BODY))
                        .text_color(rgb(theme::TEXT_SECONDARY))
                        .flex_shrink_0()
                        .child("Port"),
                )
                .child(
                    div()
                        .flex_1()
                        .rounded(px(6.0))
                        .bg(rgb(theme::BG_CARD))
                        .border_1()
                        .border_color(rgb(theme::BORDER_DEFAULT))
                        .px(px(10.0))
                        .py(px(6.0))
                        .text_size(px(theme::FONT_BODY))
                        .text_color(rgb(theme::TEXT_PRIMARY))
                        .child(port_label),
                ),
        )
        .child(
            // ── Port presets ──────────────────────────────────────────────
            div()
                .flex()
                .flex_row()
                .gap(px(8.0))
                .child(port_preset_button("3000", cx))
                .child(port_preset_button("5173", cx))
                .child(port_preset_button("8080", cx))
                .child(port_preset_button("4321", cx)),
        )
        .child(
            // ── Open / status button ──────────────────────────────────────
            if !is_connected {
                div()
                    .rounded(px(8.0))
                    .bg(rgb(theme::BG_CARD))
                    .border_1()
                    .border_color(rgb(theme::BORDER_DEFAULT))
                    .px(px(16.0))
                    .py(px(10.0))
                    .text_size(px(theme::FONT_BODY))
                    .text_color(rgb(theme::TEXT_MUTED))
                    .text_align(TextAlign::Center)
                    .child("Connect to a session first")
            } else if has_proxy {
                let local_port = proxy_port.unwrap();
                let url = format!("http://127.0.0.1:{}/", local_port);
                div()
                    .rounded(px(8.0))
                    .bg(rgb(0x1a4d2e))
                    .border_1()
                    .border_color(rgb(0x2d7a4a))
                    .px(px(16.0))
                    .py(px(10.0))
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |_this, _event, _window, _cx| {
                            platform_bridge::bridge().open_url(&url);
                        }),
                    )
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        div()
                            .text_size(px(theme::FONT_BODY))
                            .text_color(rgb(0x4ade80))
                            .child("Proxy active — tap to open"),
                    )
                    .child(
                        div()
                            .text_size(px(theme::FONT_DETAIL))
                            .text_color(rgb(theme::TEXT_MUTED))
                            .child(format!("127.0.0.1:{}", local_port)),
                    )
            } else {
                let handle_weak = handle.cloned();
                let port_str = state.port_input.clone();
                div()
                    .rounded(px(8.0))
                    .bg(rgb(theme::ACCENT_BLUE))
                    .px(px(16.0))
                    .py(px(10.0))
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |_this, _event, _window, cx| {
                            let Ok(port) = port_str.trim().parse::<u16>() else {
                                return;
                            };
                            if let Some(handle) = handle_weak.clone() {
                                let rt = zedra_session::session_runtime();
                                cx.spawn(async move |this, cx| {
                                    match rt
                                        .spawn(async move { handle.open_proxy(port).await })
                                        .await
                                    {
                                        Ok(Ok(proxy)) => {
                                            let local_port = proxy.local_port();
                                            let url =
                                                format!("http://127.0.0.1:{}/", local_port);
                                            let _ = this.update(
                                                cx,
                                                |drawer: &mut WorkspaceDrawer, cx| {
                                                    drawer.web_preview.proxy = Some(proxy);
                                                    drawer.web_preview.error = None;
                                                    cx.notify();
                                                },
                                            );
                                            platform_bridge::bridge().open_url(&url);
                                        }
                                        Ok(Err(e)) => {
                                            let _ = this.update(
                                                cx,
                                                |drawer: &mut WorkspaceDrawer, cx| {
                                                    drawer.web_preview.error =
                                                        Some(format!("Failed: {}", e));
                                                    cx.notify();
                                                },
                                            );
                                        }
                                        Err(_) => {}
                                    }
                                })
                                .detach();
                            }
                        }),
                    )
                    .text_size(px(theme::FONT_BODY))
                    .text_color(rgb(0xffffff))
                    .text_align(TextAlign::Center)
                    .child("Open Preview")
            },
        )
        .when_some(error_text, |el, err| {
            el.child(
                div()
                    .text_size(px(theme::FONT_DETAIL))
                    .text_color(rgb(0xff6b6b))
                    .child(err),
            )
        })
        .when(has_proxy, |el| {
            el.child(
                div()
                    .rounded(px(8.0))
                    .bg(rgb(theme::BG_CARD))
                    .border_1()
                    .border_color(rgb(theme::BORDER_DEFAULT))
                    .px(px(16.0))
                    .py(px(8.0))
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _event, _window, cx| {
                            this.web_preview.proxy = None;
                            this.web_preview.error = None;
                            cx.notify();
                        }),
                    )
                    .text_size(px(theme::FONT_BODY))
                    .text_color(rgb(theme::TEXT_SECONDARY))
                    .text_align(TextAlign::Center)
                    .child("Stop Proxy"),
            )
        })
}

fn port_preset_button(port: &'static str, cx: &mut Context<WorkspaceDrawer>) -> impl IntoElement {
    div()
        .rounded(px(6.0))
        .bg(rgb(theme::BG_CARD))
        .border_1()
        .border_color(rgb(theme::BORDER_DEFAULT))
        .px(px(10.0))
        .py(px(4.0))
        .cursor_pointer()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _event, _window, cx| {
                this.web_preview.port_input = port.to_string();
                this.web_preview.proxy = None;
                this.web_preview.error = None;
                cx.notify();
            }),
        )
        .text_size(px(theme::FONT_DETAIL))
        .text_color(rgb(theme::TEXT_SECONDARY))
        .child(port)
}
