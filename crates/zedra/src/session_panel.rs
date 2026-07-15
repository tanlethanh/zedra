/// Session info panel for the workspace drawer.
///
/// Displays host info, connection details, endpoints, and disconnect button.
use futures::channel::oneshot;
use gpui::*;

use crate::platform_bridge::{self, AlertButton, HapticFeedback};
use crate::transport_badge::{render_transport_badge, transport_badge};
use crate::workspace_state::{TrackedTunnel, WorkspaceState};
use crate::{fonts, theme, web_tunnel, workspace_action};
use zedra_rpc::proto::{HostBatteryInfo, HostInfoSnapshot};
use zedra_session::{SessionHandle, SessionState};

pub struct SessionPanel {
    workspace_state: Entity<WorkspaceState>,
    session_state: Entity<SessionState>,
    session_handle: SessionHandle,
    _subscriptions: Vec<Subscription>,
}

impl SessionPanel {
    pub fn new(
        workspace_state: Entity<WorkspaceState>,
        session_state: Entity<SessionState>,
        session_handle: SessionHandle,
        cx: &mut Context<Self>,
    ) -> Self {
        let workspace_state_sub = cx.observe(&workspace_state, |_, _, cx| cx.notify());

        Self {
            workspace_state,
            session_state,
            session_handle,
            _subscriptions: vec![workspace_state_sub],
        }
    }

    /// Reopen a tracked tunnel and bump it to the front of the list.
    fn open_tunnel(&self, tunnel: TrackedTunnel, cx: &mut Context<Self>) {
        platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
        self.workspace_state.update(cx, |state, cx| {
            state.record_web_tunnel(&tunnel.url, &tunnel.title, cx);
        });
        web_tunnel::open_url(self.session_handle.clone(), &tunnel.url);
    }

    /// Long-press a tracked tunnel: open it or remove it from the list.
    fn long_press_tunnel(&self, tunnel: TrackedTunnel, cx: &mut Context<Self>) {
        platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
        let (tx, rx) = oneshot::channel();
        platform_bridge::show_selection(
            &tunnel.title,
            &tunnel.url,
            vec![
                AlertButton::default("Open"),
                AlertButton::destructive("Remove"),
            ],
            move |choice| {
                let _ = tx.send(choice);
            },
        );
        cx.spawn(async move |this, cx| match rx.await {
            Ok(Some(0)) => {
                let _ = this.update(cx, |this, cx| this.open_tunnel(tunnel, cx));
            }
            Ok(Some(1)) => {
                let _ = this.update(cx, |this, cx| {
                    this.workspace_state.update(cx, |state, cx| {
                        state.remove_web_tunnel(&tunnel.url, cx);
                    });
                });
            }
            _ => {}
        })
        .detach();
    }

    /// Prompt for an address and open it as a new webview, tracking it.
    fn open_manual(&self, cx: &mut Context<Self>) {
        platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
        let (tx, rx) = oneshot::channel();
        platform_bridge::show_text_input(
            "Open webview",
            "localhost:5173 or https://example.com",
            "",
            move |result| {
                let _ = tx.send(result);
            },
        );
        cx.spawn(async move |this, cx| {
            let Ok(Some(input)) = rx.await else { return };
            let url = web_tunnel::normalize_target(&input);
            if url.is_empty() {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                if let Some(title) = web_tunnel::open_url(this.session_handle.clone(), &url) {
                    this.workspace_state.update(cx, |state, cx| {
                        state.record_web_tunnel(&url, &title, cx);
                    });
                }
            });
        })
        .detach();
    }
}

impl Render for SessionPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let session_state = self.session_state.read(cx);
        let workspace_state = self.workspace_state.read(cx);

        let phase = workspace_state
            .connect_phase
            .clone()
            .unwrap_or_else(|| session_state.phase());
        let is_empty = phase.is_init();
        if is_empty {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(theme::text_muted(cx)))
                .text_size(px(theme::FONT_BODY))
                .child("No active session");
        }

        let snap = session_state.snapshot();
        let host_info = workspace_state.host_info.clone();
        let web_tunnels = workspace_state.web_tunnels.clone();

        let mut info = div().px(px(theme::DRAWER_PADDING)).flex().flex_col();

        // Host row carries the disconnect button, right-aligned. The button must
        // render even before SyncComplete populates host metadata — it is the only
        // way to abort a connection that hangs during bind/auth/sync.
        let host_label = if !snap.username.is_empty() && !snap.hostname.is_empty() {
            info_row(cx, "Host", format!("{}@{}", snap.username, snap.hostname))
        } else {
            div()
        };
        info = info.child(
            div()
                .w_full()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(theme::SPACING_MD))
                .child(host_label.flex_1().min_w_0())
                .child(disconnect_button(cx)),
        );

        if let Some(os) = snap.os_version.as_deref()
            && let Some(arch) = snap.arch.as_deref()
        {
            let platform = format!("{} / {}", os, arch,);
            info = info.child(info_row(cx, "Platform", platform));
        }

        if let Some(host_info) = host_info.as_ref() {
            info = info.child(render_host_info(cx, host_info));
        }

        if !snap.strip_path.is_empty() {
            info = info.child(info_row(cx, "Directory", snap.strip_path.clone()));
        }

        // --- Connection badge ---
        let (badge_label, badge_color) =
            transport_badge(&theme::palette(cx), &phase, snap.transport.as_ref());
        info = info.child(
            div()
                .id("session-connection-section")
                .w_full()
                .min_w_0()
                .py(px(4.0))
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(theme::SPACING_MD))
                .cursor_pointer()
                .on_press(cx.listener(|_this, _event, window, cx| {
                    window.dispatch_action(workspace_action::ShowConnecting.boxed_clone(), cx);
                }))
                .child(
                    div()
                        .w_full()
                        .min_w_0()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .text_color(rgb(theme::text_muted(cx)))
                                .text_size(px(theme::FONT_DETAIL))
                                .child("Connection"),
                        )
                        .child(
                            render_transport_badge(badge_label, badge_color)
                                .w_full()
                                .min_w_0()
                                .whitespace_normal(),
                        ),
                )
                .child(
                    div().flex_shrink_0().pl(px(8.0)).child(
                        svg()
                            .path("icons/chevron-right.svg")
                            .size(px(theme::ICON_SM))
                            .text_color(rgb(theme::text_muted(cx))),
                    ),
                ),
        );

        // --- Web tunnels section ---
        info = info.child(
            div()
                .mt(px(8.0))
                .mb(px(2.0))
                .text_color(rgb(theme::text_muted(cx)))
                .text_size(px(theme::FONT_DETAIL))
                .child("Web tunnels"),
        );
        let mut list = div().flex().flex_col();
        for (idx, tunnel) in web_tunnels.into_iter().enumerate() {
            list = list.child(tunnel_row(idx, tunnel, cx));
        }
        info = info.child(list.child(open_webview_row(cx)));

        info.child(div().h(px(16.0)))
    }
}

/// Small icon button that tears down the session, aligned with the host row.
fn disconnect_button(cx: &mut Context<SessionPanel>) -> impl IntoElement {
    div()
        .id("session-disconnect-btn")
        .flex_shrink_0()
        .p(px(6.0))
        .rounded(px(6.0))
        .cursor_pointer()
        .hit_slop(px(8.0))
        .on_press(cx.listener(|_this, _event, window, cx| {
            window.dispatch_action(workspace_action::RequestDisconnect.boxed_clone(), cx);
        }))
        .child(
            svg()
                .path("icons/log-out.svg")
                .size(px(theme::ICON_SM))
                .text_color(rgb(theme::accent_red(cx))),
        )
}

fn render_host_info(cx: &App, host_info: &HostInfoSnapshot) -> Div {
    let mut parts = vec![
        format!("CPU {:.0}%", host_info.cpu_usage_percent),
        format!(
            "RAM {}",
            format_percent(host_info.memory_used_bytes, host_info.memory_total_bytes)
        ),
    ];

    if let Some(batteries) = format_batteries(&host_info.batteries) {
        parts.push(format!("Batteries {batteries}"));
    }

    info_row(cx, "System Stats", parts.join(" \u{00b7} "))
}

fn format_percent(used: u64, total: u64) -> String {
    if total == 0 {
        "--%".to_string()
    } else {
        format!("{:.0}%", (used as f64 / total as f64) * 100.0)
    }
}

fn format_batteries(batteries: &[HostBatteryInfo]) -> Option<String> {
    let labels = batteries
        .iter()
        .filter_map(|battery| {
            battery
                .charge_percent
                .map(|value| format!("{}%", value.min(100)))
        })
        .collect::<Vec<_>>();
    if labels.is_empty() {
        None
    } else {
        Some(labels.join(", "))
    }
}

/// Row that prompts for an address and opens it as a new webview.
fn open_webview_row(cx: &mut Context<SessionPanel>) -> impl IntoElement {
    div()
        .id("session-web-tunnel-add")
        .w_full()
        .py(px(theme::SPACING_XS))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::SPACING_SM))
        .cursor_pointer()
        .hit_slop(px(4.0))
        .on_press(cx.listener(|this, _event, _window, cx| this.open_manual(cx)))
        .child(
            svg()
                .path("icons/plus.svg")
                .size(px(theme::ICON_SM))
                .flex_shrink_0()
                .text_color(rgb(theme::text_muted(cx))),
        )
        .child(
            div()
                .text_color(rgb(theme::text_muted(cx)))
                .text_size(px(theme::FONT_BODY))
                .child("Open webview"),
        )
}

fn tunnel_row(
    idx: usize,
    tunnel: TrackedTunnel,
    cx: &mut Context<SessionPanel>,
) -> impl IntoElement {
    let on_open = tunnel.clone();
    let on_long = tunnel.clone();
    div()
        .id(("session-web-tunnel", idx))
        .w_full()
        .min_w_0()
        .py(px(theme::SPACING_XS))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(theme::SPACING_MD))
        .cursor_pointer()
        .hit_slop(px(4.0))
        .on_press(cx.listener(move |this, _event, _window, cx| {
            this.open_tunnel(on_open.clone(), cx);
        }))
        .on_long_press(cx.listener(move |this, _event, _window, cx| {
            this.long_press_tunnel(on_long.clone(), cx);
        }))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(1.0))
                .child(
                    div()
                        .text_color(rgb(theme::text_primary(cx)))
                        .text_size(px(theme::FONT_BODY))
                        .font_family(fonts::MONO_FONT_FAMILY)
                        .child(tunnel.title.clone()),
                )
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .text_color(rgb(theme::text_muted(cx)))
                        .text_size(px(theme::FONT_DETAIL))
                        .child(tunnel.url.clone()),
                ),
        )
        .child(
            div().flex_shrink_0().pl(px(8.0)).child(
                svg()
                    .path("icons/chevron-right.svg")
                    .size(px(theme::ICON_SM))
                    .text_color(rgb(theme::text_muted(cx))),
            ),
        )
}

fn info_row(cx: &App, label: &'static str, value: String) -> Div {
    div()
        .py(px(4.0))
        .child(
            div()
                .text_color(rgb(theme::text_muted(cx)))
                .text_size(px(theme::FONT_DETAIL))
                .child(label),
        )
        .child(
            div()
                .mt(px(1.0))
                .text_color(rgb(theme::text_secondary(cx)))
                .text_size(px(theme::FONT_BODY))
                .child(value),
        )
}
