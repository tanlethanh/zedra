use std::time::Duration;

use futures::channel::oneshot;
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_tokio::Tokio;

use crate::delta::{self, DeltaState};
use crate::fonts;
use crate::platform_bridge::{self, AlertButton, HapticFeedback};
use crate::theme;

#[derive(Clone, Debug)]
pub enum AccountEvent {
    Close,
}

impl EventEmitter<AccountEvent> for AccountView {}

/// Bigger than the surrounding detail text so it reads as a control.
const COPY_ICON_SIZE: f32 = 16.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CopyField {
    Stack,
    Device,
    Node(uuid::Uuid),
}

pub struct AccountView {
    delta_state: Entity<DeltaState>,
    deleting: bool,
    message: Option<String>,
    copied: Option<CopyField>,
    /// Set while a push-permission round trip is in flight.
    push_busy: bool,
    /// Dropping this cancels the pending "copied" reset.
    copied_reset: Option<Task<()>>,
    nodes_busy: bool,
}

impl AccountView {
    pub fn new(delta_state: Entity<DeltaState>, _cx: &mut Context<Self>) -> Self {
        Self {
            delta_state,
            deleting: false,
            message: None,
            copied: None,
            push_busy: false,
            copied_reset: None,
            nodes_busy: false,
        }
    }

    /// Cached nodes paint immediately; this only refetches once they age out.
    pub fn refresh_nodes_if_stale(&mut self, cx: &mut Context<Self>) {
        if self.delta_state.read(cx).status().nodes_stale {
            self.refresh_nodes(cx);
        }
    }

    fn refresh_nodes(&mut self, cx: &mut Context<Self>) {
        if self.nodes_busy || !self.delta_state.read(cx).status().signed_in {
            return;
        }
        self.nodes_busy = true;
        let delta_state = self.delta_state.clone();
        let snapshot = delta_state.read(cx).snapshot();
        cx.spawn(async move |this, cx| {
            let result = Tokio::spawn_result(cx, delta::fetch_nodes(snapshot.clone())).await;
            let _ = this.update(cx, |this, cx| {
                this.nodes_busy = false;
                match result {
                    Ok((fetched, nodes)) => {
                        let count = nodes.len();
                        let applied = delta_state.update(cx, |state, cx| {
                            state.apply_nodes(&snapshot, fetched, nodes, cx)
                        });
                        if applied {
                            tracing::info!(count, "account: cached stack nodes");
                        } else {
                            tracing::info!("account: dropped node list for a previous stack");
                        }
                    }
                    Err(error) => {
                        tracing::warn!(error = %error, "account: listing stack nodes failed")
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Shared with Settings: see `delta::acquire_and_register_push_token`.
    fn enable_notifications(&mut self, cx: &mut Context<Self>) {
        if self.push_busy {
            return;
        }
        self.push_busy = true;
        self.message = Some("Requesting notification permission".to_string());
        platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
        let delta_state = self.delta_state.clone();
        cx.spawn(async move |this, cx| {
            let result = delta::acquire_and_register_push_token(delta_state, cx, |cx| {
                let _ = this.update(cx, |this, cx| {
                    this.message = Some("Registering push token".to_string());
                    cx.notify();
                });
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                this.push_busy = false;
                this.message = match result {
                    Ok(_) => None,
                    Err(error) => {
                        tracing::warn!(error = %error, "account: enabling notifications failed");
                        Some(format!("{error:#}"))
                    }
                };
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn copy_field(&mut self, field: CopyField, value: String, cx: &mut Context<Self>) {
        if value.is_empty() || value == "\u{2014}" {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(value));
        platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
        self.copied = Some(field);
        self.copied_reset = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1400))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.copied = None;
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn confirm_delete(&mut self, cx: &mut Context<Self>) {
        if self.deleting {
            return;
        }
        platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
        let (tx, rx) = oneshot::channel();
        platform_bridge::show_alert(
            "Delete account?",
            "This permanently erases your Delta account, stack, registered devices, notifications, and synced activity. This cannot be undone.",
            vec![
                AlertButton::destructive("Delete Account"),
                AlertButton::cancel("Cancel"),
            ],
            move |index| {
                let _ = tx.send(index);
            },
        );
        cx.spawn(async move |this, cx| {
            if let Ok(0) = rx.await {
                let _ = this.update(cx, |this, cx| this.delete(cx));
            }
        })
        .detach();
    }

    fn delete(&mut self, cx: &mut Context<Self>) {
        self.deleting = true;
        self.message = None;
        let snapshot = self.delta_state.read(cx).snapshot();
        let deleted_stack = self.delta_state.read(cx).status().stack_id;
        cx.spawn(async move |this, cx| {
            let result = Tokio::spawn_result(cx, delta::delete_account(snapshot)).await;
            let _ = this.update(cx, |this, cx| {
                this.deleting = false;
                match result {
                    Ok(next) => {
                        let applied = this
                            .delta_state
                            .update(cx, |state, cx| state.apply_deleted(deleted_stack, next, cx));
                        if !applied {
                            tracing::warn!(
                                "account: deleted account state was not adopted; signing out"
                            );
                            this.log_out(cx);
                        }
                        cx.emit(AccountEvent::Close);
                    }
                    Err(error) => {
                        this.message = Some(format!("{error:#}"));
                        cx.notify();
                    }
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn log_out(&mut self, cx: &mut Context<Self>) {
        let snapshot = self.delta_state.read(cx).snapshot();
        match delta::sign_out(snapshot) {
            Ok(next) => {
                self.delta_state
                    .update(cx, |state, cx| state.apply(next, cx));
                cx.emit(AccountEvent::Close);
            }
            Err(error) => {
                self.message = Some(format!("{error:#}"));
                cx.notify();
            }
        }
    }
}

impl Render for AccountView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Mirrors the Settings screen shell: it is drawer-mounted too, so it owns
        // its own status-bar and home-indicator insets.
        let top_inset = platform_bridge::status_bar_inset();
        let bottom_inset = platform_bridge::home_indicator_inset();
        let status = self.delta_state.read(cx).status();
        let email = status
            .email
            .clone()
            .unwrap_or_else(|| "Not signed in".to_string());
        let initial = email
            .chars()
            .next()
            .unwrap_or('Z')
            .to_ascii_uppercase()
            .to_string();
        let stack = status
            .stack_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "\u{2014}".to_string());
        let node = status
            .node_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "\u{2014}".to_string());
        let summary = format!(
            "Stack {} \u{00b7} Node {}",
            short_id(status.stack_id),
            short_id(status.node_id)
        );
        let device = platform_bridge::device_name().unwrap_or_else(|| "This device".to_string());
        let copied = self.copied;
        let stack_value = stack.clone();
        let node_value = node.clone();

        div()
            .id("account-view")
            .size_full()
            .min_h_0()
            .min_w_0()
            .bg(rgb(theme::bg_primary(cx)))
            .flex()
            .flex_col()
            .child(
                div()
                    .w_full()
                    .pt(px(top_inset))
                    .px(px(theme::SPACING_MD))
                    .pb(px(10.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(10.0))
                            .child(
                                div()
                                    .id("account-back-button")
                                    .hit_slop(px(10.0))
                                    .cursor_pointer()
                                    .on_press(cx.listener(|_this, _event, _window, cx| {
                                        cx.emit(AccountEvent::Close);
                                    }))
                                    .child(
                                        svg()
                                            .path("icons/chevron-left.svg")
                                            .size(px(theme::ICON_SM))
                                            .text_color(rgb(theme::text_muted(cx)))
                                            .into_any_element(),
                                    ),
                            )
                            .child(
                                div()
                                    .text_color(rgb(theme::text_primary(cx)))
                                    .text_size(px(theme::FONT_TITLE))
                                    .font_family(fonts::HEADING_FONT_FAMILY)
                                    .font_weight(FontWeight::MEDIUM)
                                    .child("Account"),
                            ),
                    ),
            )
            .child(
                div()
                    .id("account-scroll")
                    .overflow_y_scroll()
                    .flex_1()
                    .min_h_0()
                    .px(px(theme::SPACING_LG))
                    .pb(px(bottom_inset + 18.0))
                    .child(
                        div()
                            .w_full()
                            .max_w(px(theme::CONTENT_MAX_WIDTH))
                            .mx_auto()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .child(profile_row(&initial, &email, &summary, cx))
                            .child(divider(cx))
                            .child(if status.push_registered {
                                info_row(cx, "Notifications", &push_summary(&status))
                                    .into_any_element()
                            } else {
                                enable_row(
                                    cx,
                                    "account-enable-notifications",
                                    "Notifications",
                                    if self.push_busy {
                                        "Enabling\u{2026}"
                                    } else {
                                        "Tap to enable"
                                    },
                                    self.push_busy,
                                    cx.listener(|this, _, _, cx| this.enable_notifications(cx)),
                                )
                                .into_any_element()
                            })
                            .child(info_row(cx, "Server", &server_host(&status.base_url)))
                            .child(copy_row(
                                cx,
                                "account-copy-stack",
                                "Stack",
                                &stack,
                                copied == Some(CopyField::Stack),
                                cx.listener(move |this, _, _, cx| {
                                    this.copy_field(CopyField::Stack, stack_value.clone(), cx)
                                }),
                            ))
                            .child(copy_row(
                                cx,
                                "account-copy-device",
                                &device,
                                &node,
                                copied == Some(CopyField::Device),
                                cx.listener(move |this, _, _, cx| {
                                    this.copy_field(CopyField::Device, node_value.clone(), cx)
                                }),
                            ))
                            .child(divider(cx))
                            .child(devices_header(cx, status.nodes.len(), self.nodes_busy))
                            .children(status.nodes.iter().map(|stored| {
                                let id = stored.id;
                                let value = id.to_string();
                                node_row(
                                    cx,
                                    stored,
                                    Some(id) == status.node_id,
                                    copied == Some(CopyField::Node(id)),
                                    cx.listener(move |this, _, _, cx| {
                                        this.copy_field(CopyField::Node(id), value.clone(), cx)
                                    }),
                                )
                            }))
                            .child(divider(cx))
                            .child(action_row(
                                cx,
                                "account-log-out",
                                "Log Out",
                                false,
                                false,
                                cx.listener(|this, _, _, cx| this.log_out(cx)),
                            ))
                            // Keep the destructive action off the edge of a
                            // mis-tap on Log Out.
                            .child(divider(cx))
                            .child(action_row(
                                cx,
                                "account-delete",
                                "Delete Account",
                                true,
                                self.deleting,
                                cx.listener(|this, _, _, cx| this.confirm_delete(cx)),
                            ))
                            .when_some(self.message.clone(), |this, message| {
                                this.child(
                                    div()
                                        .min_w_0()
                                        .text_color(rgb(theme::accent_red(cx)))
                                        .text_size(px(theme::FONT_DETAIL))
                                        .font_family(fonts::MONO_FONT_FAMILY)
                                        .whitespace_normal()
                                        .child(message),
                                )
                            }),
                    ),
            )
    }
}

/// Avatar + identity, matching `settings_view::profile_info_row` metrics. No
/// remote image loading on this path, so the avatar is the account initial.
fn profile_row(initial: &str, email: &str, summary: &str, cx: &App) -> impl IntoElement {
    div()
        .min_w_0()
        .py(px(theme::SPACING_SM))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::SPACING_SM))
        .child(
            div()
                .flex_shrink_0()
                .size(px(32.0))
                .rounded_full()
                .bg(rgb(theme::bg_card(cx)))
                .border_1()
                .border_color(rgb(theme::border_subtle(cx)))
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(theme::text_secondary(cx)))
                .text_size(px(theme::FONT_BODY))
                .font_family(fonts::MONO_FONT_FAMILY)
                .font_weight(FontWeight::MEDIUM)
                .child(initial.to_string()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(1.0))
                .overflow_hidden()
                .child(
                    div()
                        .text_color(rgb(theme::text_secondary(cx)))
                        .text_size(px(theme::FONT_BODY))
                        .font_family(fonts::MONO_FONT_FAMILY)
                        .font_weight(FontWeight::MEDIUM)
                        .child(email.to_string()),
                )
                .child(
                    div()
                        .text_color(rgb(theme::text_muted(cx)))
                        .text_size(px(theme::FONT_DETAIL))
                        .font_family(fonts::MONO_FONT_FAMILY)
                        .child(summary.to_string()),
                ),
        )
}

fn divider(cx: &App) -> impl IntoElement {
    div()
        .my(px(theme::SPACING_XS))
        .border_t_1()
        .border_color(rgb(theme::border_subtle(cx)))
}

fn short_id(id: Option<uuid::Uuid>) -> String {
    id.map(|id| id.to_string().chars().take(8).collect())
        .unwrap_or_else(|| "\u{2014}".to_string())
}

/// "Enabled \u{00b7} APNs (production)" — plain language, no raw enum labels.
fn push_summary(status: &delta::DeltaStatus) -> String {
    if !status.push_registered {
        return "Not enabled on this device".to_string();
    }
    let provider = match status.push_provider.as_deref() {
        Some("apns") => "APNs",
        Some("fcm") => "FCM",
        Some(other) => other,
        None => return "Enabled".to_string(),
    };
    match status.push_environment.as_deref() {
        Some(environment) => format!("Enabled \u{00b7} {provider} ({environment})"),
        None => format!("Enabled \u{00b7} {provider}"),
    }
}

fn server_host(base_url: &str) -> String {
    base_url
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(base_url)
        .trim_end_matches('/')
        .to_string()
}

/// Label left, value right on one line. Nothing here is an id.
fn info_row(cx: &App, label: &'static str, value: &str) -> impl IntoElement {
    div()
        .min_w_0()
        .py(px(5.0))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(theme::SPACING_SM))
        .child(
            div()
                .flex_shrink_0()
                .text_color(rgb(theme::text_muted(cx)))
                .text_size(px(theme::FONT_DETAIL))
                .font_family(fonts::MONO_FONT_FAMILY)
                .child(label),
        )
        .child(
            div()
                .min_w_0()
                .text_color(rgb(theme::text_secondary(cx)))
                .text_size(px(theme::FONT_DETAIL))
                .font_family(fonts::MONO_FONT_FAMILY)
                .truncate()
                .child(value.to_string()),
        )
}

/// Same shape as `info_row`, but tappable, with a chevron to say so.
fn enable_row(
    cx: &App,
    id: &'static str,
    label: &'static str,
    value: &'static str,
    disabled: bool,
    on_press: impl Fn(&PressEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .min_w_0()
        .py(px(5.0))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(theme::SPACING_SM))
        .when(!disabled, |this| this.cursor_pointer().on_press(on_press))
        .opacity(if disabled { 0.5 } else { 1.0 })
        .child(
            div()
                .flex_shrink_0()
                .text_color(rgb(theme::text_muted(cx)))
                .text_size(px(theme::FONT_DETAIL))
                .font_family(fonts::MONO_FONT_FAMILY)
                .child(label),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(theme::SPACING_XS))
                .min_w_0()
                .child(
                    div()
                        .min_w_0()
                        .text_color(rgb(theme::accent_blue(cx)))
                        .text_size(px(theme::FONT_DETAIL))
                        .font_family(fonts::MONO_FONT_FAMILY)
                        .truncate()
                        .child(value),
                )
                .child(
                    svg()
                        .path("icons/chevron-right.svg")
                        .size(px(12.0))
                        .flex_shrink_0()
                        .text_color(rgb(theme::text_muted(cx))),
                ),
        )
}

/// "Devices  3" with a quiet marker while the list is refreshing.
fn devices_header(cx: &App, count: usize, busy: bool) -> impl IntoElement {
    div()
        .min_w_0()
        .py(px(5.0))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(theme::SPACING_SM))
        .child(
            div()
                .text_color(rgb(theme::text_muted(cx)))
                .text_size(px(theme::FONT_DETAIL))
                .font_family(fonts::MONO_FONT_FAMILY)
                .child("Devices"),
        )
        .child(
            div()
                .text_color(rgb(theme::text_muted(cx)))
                .text_size(px(theme::FONT_DETAIL))
                .font_family(fonts::MONO_FONT_FAMILY)
                .child(if busy {
                    "\u{2026}".to_string()
                } else {
                    count.to_string()
                }),
        )
}

/// One registered node. The whole row copies the node id, so it needs no
/// separate control at this density.
fn node_row(
    cx: &App,
    node: &delta::StoredNode,
    is_this_device: bool,
    copied: bool,
    on_press: impl Fn(&PressEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let mut meta = vec![node.kind.clone()];
    if let Some(joined) = node.joined_date() {
        meta.push(joined);
    }
    if is_this_device {
        meta.push("this device".to_string());
    }
    div()
        .id(SharedString::from(format!("account-node-{}", node.id)))
        .min_w_0()
        .py(px(5.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::SPACING_SM))
        .cursor_pointer()
        .on_press(on_press)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(1.0))
                .child(
                    div()
                        .text_color(rgb(if is_this_device {
                            theme::text_primary(cx)
                        } else {
                            theme::text_secondary(cx)
                        }))
                        .text_size(px(theme::FONT_DETAIL))
                        .font_family(fonts::MONO_FONT_FAMILY)
                        .font_weight(FontWeight::MEDIUM)
                        .truncate()
                        .child(node.name()),
                )
                .child(
                    div()
                        .text_color(rgb(theme::text_muted(cx)))
                        .text_size(px(theme::FONT_DETAIL))
                        .font_family(fonts::MONO_FONT_FAMILY)
                        .truncate()
                        .child(meta.join(" \u{00b7} ")),
                ),
        )
        .child(copy_icon(cx, copied))
}

/// The one copy affordance on this screen: a bare glyph, no container, that
/// swaps to a check for a moment after copying.
fn copy_icon(cx: &App, copied: bool) -> impl IntoElement {
    svg()
        .path(if copied {
            "icons/check.svg"
        } else {
            "icons/copy.svg"
        })
        .size(px(COPY_ICON_SIZE))
        .flex_shrink_0()
        .text_color(rgb(if copied {
            theme::accent_green(cx)
        } else {
            theme::text_muted(cx)
        }))
}

/// Identifier row. The whole row copies the value, matching the device rows.
fn copy_row(
    cx: &App,
    id: &'static str,
    label: &str,
    value: &str,
    copied: bool,
    on_press: impl Fn(&PressEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .min_w_0()
        .py(px(5.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::SPACING_SM))
        .cursor_pointer()
        .on_press(on_press)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(1.0))
                .child(
                    div()
                        .text_color(rgb(theme::text_muted(cx)))
                        .text_size(px(theme::FONT_DETAIL))
                        .font_family(fonts::MONO_FONT_FAMILY)
                        .truncate()
                        .child(label.to_string()),
                )
                .child(
                    div()
                        .text_color(rgb(theme::text_secondary(cx)))
                        .text_size(px(theme::FONT_DETAIL))
                        .font_family(fonts::MONO_FONT_FAMILY)
                        .truncate()
                        .child(value.to_string()),
                ),
        )
        .child(copy_icon(cx, copied))
}

/// Terminal action, one line. Destructive variant carries the red accent.
fn action_row(
    cx: &App,
    id: &'static str,
    title: &'static str,
    destructive: bool,
    disabled: bool,
    on_press: impl Fn(&PressEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let color = if destructive {
        theme::accent_red(cx)
    } else {
        theme::text_secondary(cx)
    };
    div()
        .id(id)
        .min_w_0()
        .py(px(theme::SPACING_SM))
        .flex()
        .flex_row()
        .items_center()
        .when(!disabled, |this| this.cursor_pointer().on_press(on_press))
        .opacity(if disabled { 0.5 } else { 1.0 })
        .child(
            div()
                .text_color(rgb(color))
                .text_size(px(theme::FONT_BODY))
                .font_family(fonts::MONO_FONT_FAMILY)
                .font_weight(FontWeight::MEDIUM)
                .child(title),
        )
}
