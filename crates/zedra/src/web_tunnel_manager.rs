//! Settings subscreen listing the exact-port web-tunnel listeners bound on this
//! device (port -> owning workspace) with a per-row Stop control to free a port
//! that conflicts with another app. See `docs/WEB_TUNNEL_MODES.md` § Managing listeners.

use futures::channel::oneshot;
use gpui::*;

use crate::app_action::SystemBack;
use crate::fonts;
use crate::platform_bridge::{self, AlertButton, HapticFeedback};
use crate::theme;
use crate::ui::{
    chevron_back_button, subscreen_empty_text, subscreen_padded_body, subscreen_page,
    subscreen_refresh_button,
};
use crate::web_tunnel::{self, ListenerInfo};
use crate::workspaces::Workspaces;

pub struct WebTunnelManager {
    workspaces: Entity<Workspaces>,
}

impl WebTunnelManager {
    pub fn new(workspaces: Entity<Workspaces>, _cx: &mut Context<Self>) -> Self {
        Self { workspaces }
    }

    /// Confirm before stopping — a live page loading through the port dies with it.
    fn confirm_stop(&mut self, port: u16, host: String, cx: &mut Context<Self>) {
        platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
        let (tx, rx) = oneshot::channel();
        platform_bridge::show_alert(
            "Stop web tunnel listener",
            &format!("Free port :{port} for {host}? Anything loading through it will stop."),
            vec![
                AlertButton::destructive("Stop"),
                AlertButton::cancel("Cancel"),
            ],
            move |index| {
                let _ = tx.send(index);
            },
        );
        cx.spawn(async move |this, cx| {
            if let Ok(0) = rx.await {
                let _ = this.update(cx, |this, cx| this.stop(port, cx));
            }
        })
        .detach();
    }

    fn stop(&mut self, port: u16, cx: &mut Context<Self>) {
        web_tunnel::stop_listener(port);
        // render re-reads the live listener set.
        cx.notify();
    }

    /// The listener's host by workspace display name, falling back to a short id.
    fn host_name(&self, listener: &ListenerInfo, cx: &App) -> String {
        zedra_rpc::encode_endpoint_identity(listener.endpoint_id)
            .ok()
            .and_then(|id| {
                self.workspaces
                    .read(cx)
                    .states()
                    .iter()
                    .find(|state| state.read(cx).endpoint_addr == id)
                    .map(|state| state.read(cx).display_name().to_string())
            })
            .unwrap_or_else(|| listener.endpoint_id.fmt_short().to_string())
    }
}

impl Render for WebTunnelManager {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Top-level screen (drawer-mounted), so it owns its status-bar inset — unlike
        // workspace subscreens, which the workspace container already insets.
        let top_inset = platform_bridge::status_bar_inset();
        // Resolve names first (immutable borrow) so the row builders can take `cx` mutably.
        let rows: Vec<(u16, String)> = web_tunnel::active_listeners()
            .iter()
            .map(|listener| (listener.port, self.host_name(listener, cx)))
            .collect();

        let body: AnyElement = if rows.is_empty() {
            subscreen_padded_body(subscreen_empty_text(
                "No web tunnel listeners are bound",
                cx,
            ))
            .into_any_element()
        } else {
            let mut list = div()
                .id("web-tunnel-list")
                .w_full()
                .min_w_0()
                .flex()
                .flex_col();
            for (port, host) in rows {
                list = list.child(listener_row(port, host, cx));
            }
            subscreen_padded_body(list).into_any_element()
        };

        div()
            .id("web-tunnel-root")
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(rgb(theme::bg_primary(cx)))
            .child(div().h(px(top_inset)))
            .child(div().flex_1().min_h_0().child(subscreen_page(
                "web-tunnel-manager",
                rgb(theme::bg_primary(cx)),
                header(cx).into_any_element(),
                body,
            )))
    }
}

fn header(cx: &mut Context<WebTunnelManager>) -> impl IntoElement {
    div()
        .id("web-tunnel-header")
        .min_w_0()
        .px(px(theme::SUBSCREEN_PADDING_X))
        .pt(px(theme::SPACING_XS))
        .pb(px(theme::SPACING_SM))
        .child(
            div()
                .id("web-tunnel-header-inner")
                .relative()
                .min_w_0()
                .child(
                    div()
                        .min_w_0()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(theme::SPACING_MD))
                        .child(back_button(cx))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(0.0))
                                .child(
                                    div()
                                        .text_size(px(theme::FONT_HEADING))
                                        .font_family(fonts::HEADING_FONT_FAMILY)
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(rgb(theme::text_primary(cx)))
                                        .child("Web tunnel"),
                                )
                                .child(
                                    div()
                                        .text_size(px(theme::FONT_BODY))
                                        .text_color(rgb(theme::text_muted(cx)))
                                        .child("Localhost listeners bound on this device"),
                                ),
                        ),
                )
                .child(subscreen_refresh_button(
                    "web-tunnel-refresh-btn",
                    cx,
                    |_this, _event, _window, cx| cx.notify(),
                )),
        )
}

fn back_button(cx: &mut Context<WebTunnelManager>) -> Stateful<Div> {
    chevron_back_button("web-tunnel-back-btn", cx, |_this, _event, window, cx| {
        platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
        window.dispatch_action(SystemBack.boxed_clone(), cx);
    })
}

fn listener_row(port: u16, host: String, cx: &mut Context<WebTunnelManager>) -> impl IntoElement {
    div()
        .id(("web-tunnel-row", port as usize))
        .min_h(px(56.0))
        .py(px(theme::SPACING_SM))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::SPACING_MD))
        .border_b_1()
        .border_color(rgb(theme::border_subtle(cx)))
        .child(
            div()
                .flex_shrink_0()
                .text_color(rgb(theme::text_primary(cx)))
                .text_size(px(theme::FONT_BODY))
                .font_family(fonts::MONO_FONT_FAMILY)
                .font_weight(FontWeight::MEDIUM)
                .child(format!(":{port}")),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .text_color(rgb(theme::text_muted(cx)))
                .text_size(px(theme::FONT_DETAIL))
                .font_family(fonts::MONO_FONT_FAMILY)
                .child(host.clone()),
        )
        .child(stop_button(port, host, cx))
}

fn stop_button(port: u16, host: String, cx: &mut Context<WebTunnelManager>) -> impl IntoElement {
    div()
        .id(("web-tunnel-stop", port as usize))
        .flex_shrink_0()
        .px(px(12.0))
        .py(px(8.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(rgb(theme::accent_red(cx)))
        .text_color(rgb(theme::accent_red(cx)))
        .text_size(px(theme::FONT_BODY))
        .cursor_pointer()
        .hit_slop(px(8.0))
        .on_press(cx.listener(move |this, _event, _window, cx| {
            this.confirm_stop(port, host.clone(), cx);
        }))
        .child("Stop")
}
