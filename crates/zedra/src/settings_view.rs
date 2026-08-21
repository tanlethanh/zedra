use gpui::{prelude::FluentBuilder as _, *};
use gpui_tokio::Tokio;

use futures::channel::oneshot;

use crate::delta::{self, DeltaState};
use crate::platform_bridge::{
    self, AlertButton, CustomSheetDetent, CustomSheetOptions, HapticFeedback,
};
use crate::settings::ThemeState;
use crate::sheet_demo_state::SheetDemoState;
use crate::telemetry::view_telemetry;
use crate::theme::{self, ThemePreference};
use crate::{fonts, settings};

const TELEMETRY_DOCS_URL: &str = "https://zedra.dev/docs/telemetry";
const PRIVACY_POLICY_URL: &str = "https://zedra.dev/privacy";

#[derive(Clone, Debug)]
pub enum SettingsEvent {
    NavigateHome,
    OpenAccount,
    OpenWebTunnel,
    DropletToggled(bool),
}

impl EventEmitter<SettingsEvent> for SettingsView {}

/// Reconcile the persisted mobile node against Delta at launch and fold the
/// result back into the shared state entity.
pub fn reconcile_delta_on_launch<T: 'static>(delta_state: Entity<DeltaState>, cx: &mut Context<T>) {
    let snapshot = delta_state.read(cx).snapshot();
    cx.spawn(async move |_owner, cx| {
        match Tokio::spawn_result(cx, delta::reconcile_mobile_node(snapshot.clone()))
            .await
        {
            Ok((outcome, next)) => {
                let applied = delta_state.update(cx, |state, cx| {
                    // Skip launch-time reconciliation if Delta state changed while it was in flight.
                    state.merge(delta::DeltaPatch::between(&snapshot, &next), cx)
                });
                if applied {
                    tracing::info!(?outcome, "Delta mobile node reconciliation completed");
                } else {
                    tracing::debug!(
                        ?outcome,
                        "Delta mobile node reconciliation completed after state changed; skipping stale update"
                    );
                }
            }
            Err(error) => {
                tracing::warn!("Delta mobile node reconciliation failed: {error:#}");
            }
        }
    })
    .detach();
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DeltaMessageTarget {
    Profile,
    Notifications,
}

#[derive(Clone, Copy)]
enum OAuthProvider {
    Google,
    Apple,
}

pub struct SettingsView {
    focus_handle: FocusHandle,
    theme_state: Entity<ThemeState>,
    sheet_state: Entity<SheetDemoState>,
    sheet_view: Entity<crate::sheet_demo_view::SheetDemoView>,
    delta_state: Entity<DeltaState>,
    delta_message: Option<String>,
    delta_message_target: DeltaMessageTarget,
    delta_busy: bool,
    telemetry_enabled: bool,
    droplet_enabled: bool,
    key_bar_always_visible: bool,
    extended_keypad: bool,
    _delta_observe: Subscription,
}

impl SettingsView {
    pub fn new(
        theme_state: Entity<ThemeState>,
        delta_state: Entity<DeltaState>,
        cx: &mut Context<Self>,
    ) -> Self {
        let sheet_state = cx.new(|cx| SheetDemoState::new(cx));
        let sheet_view =
            cx.new(|cx| crate::sheet_demo_view::SheetDemoView::new(sheet_state.clone(), cx));
        // Re-render when the shared Delta state changes from anywhere.
        let observe = cx.observe(&delta_state, |_, _, cx| cx.notify());
        Self {
            focus_handle: cx.focus_handle(),
            theme_state,
            sheet_state,
            sheet_view,
            delta_state,
            delta_message: None,
            delta_message_target: DeltaMessageTarget::Profile,
            delta_busy: false,
            telemetry_enabled: settings::read_telemetry_enabled(),
            droplet_enabled: settings::read_droplet_enabled(),
            key_bar_always_visible: settings::key_bar_always_visible(),
            extended_keypad: settings::extended_keypad(),
            _delta_observe: observe,
        }
    }

    fn status(&self, cx: &App) -> delta::DeltaStatus {
        self.delta_state.read(cx).status()
    }

    fn start_apple_sign_in(&mut self, cx: &mut Context<Self>) {
        if self.delta_busy {
            return;
        }
        self.begin_profile_op("Opening Apple sign-in");
        let (tx, rx) = oneshot::channel();
        platform_bridge::start_delta_apple_sign_in(move |result| {
            let _ = tx.send(result.map(|r| (r.id_token, r.email)));
        });
        self.spawn_oauth_sign_in(rx, OAuthProvider::Apple, cx);
        cx.notify();
    }

    fn start_google_sign_in(&mut self, cx: &mut Context<Self>) {
        if self.delta_busy {
            return;
        }
        self.begin_profile_op("Opening Google sign-in");
        let (tx, rx) = oneshot::channel();
        platform_bridge::start_delta_google_sign_in(move |result| {
            let _ = tx.send(result.map(|r| (r.id_token, r.email)));
        });
        self.spawn_oauth_sign_in(rx, OAuthProvider::Google, cx);
        cx.notify();
    }

    fn begin_profile_op(&mut self, message: &str) {
        self.delta_busy = true;
        self.delta_message_target = DeltaMessageTarget::Profile;
        self.delta_message = Some(message.to_string());
        platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
    }

    /// Await an OAuth provider callback, then run the network sign-in on the
    /// session runtime and apply the result back onto the shared entity.
    fn spawn_oauth_sign_in(
        &mut self,
        rx: oneshot::Receiver<Result<(String, Option<String>), String>>,
        provider: OAuthProvider,
        cx: &mut Context<Self>,
    ) {
        let delta_state = self.delta_state.clone();
        cx.spawn(async move |this, cx| {
            let (id_token, email) = match rx.await {
                Ok(Ok(creds)) => creds,
                Ok(Err(message)) => return Self::report_error(&this, cx, message),
                Err(_) => return,
            };
            let _ = this.update(cx, |this, cx| {
                this.delta_message = Some("Registering Delta mobile node".to_string());
                cx.notify();
            });
            let snapshot = delta_state.read_with(cx, |state, _| state.snapshot());
            let result = match provider {
                OAuthProvider::Google => {
                    Tokio::spawn_result(
                        cx,
                        delta::sign_in_with_google(snapshot.clone(), id_token, email),
                    )
                    .await
                }
                OAuthProvider::Apple => {
                    Tokio::spawn_result(
                        cx,
                        delta::sign_in_with_apple(snapshot.clone(), id_token, email),
                    )
                    .await
                }
            };
            Self::apply_delta_result(
                &this,
                &delta_state,
                cx,
                snapshot,
                result,
                DeltaMessageTarget::Profile,
            );
        })
        .detach();
    }

    fn show_sign_in_methods(&mut self, cx: &mut Context<Self>) {
        if self.delta_busy {
            return;
        }
        self.delta_message_target = DeltaMessageTarget::Profile;
        self.delta_message = None;
        platform_bridge::trigger_haptic(HapticFeedback::SelectionChanged);
        let mut buttons = vec![AlertButton::default("Sign in with Google").image("google")];
        // Apple Sign-In is only available on iOS.
        if cfg!(target_os = "ios") {
            buttons.push(AlertButton::default("Sign in with Apple").image("apple"));
        }
        let (tx, rx) = oneshot::channel();
        platform_bridge::show_selection(
            "Sign In",
            "Choose a sign-in method for Delta.",
            buttons,
            move |result| {
                let _ = tx.send(result);
            },
        );
        cx.spawn(async move |this, cx| {
            let Ok(choice) = rx.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| match choice {
                Some(0) => this.start_google_sign_in(cx),
                Some(1) => this.start_apple_sign_in(cx),
                _ => {}
            });
        })
        .detach();
        cx.notify();
    }

    /// Shared with the Account screen: see `delta::acquire_and_register_push_token`.
    fn request_push_token(&mut self, cx: &mut Context<Self>) {
        if self.delta_busy {
            return;
        }
        self.delta_busy = true;
        self.delta_message_target = DeltaMessageTarget::Notifications;
        self.delta_message = Some("Requesting notification permission".to_string());
        platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
        let delta_state = self.delta_state.clone();
        cx.spawn(async move |this, cx| {
            let result = delta::acquire_and_register_push_token(delta_state, cx, |cx| {
                let _ = this.update(cx, |this, cx| {
                    this.delta_message = Some("Registering push token".to_string());
                    cx.notify();
                });
            })
            .await;
            match result {
                Ok(_) => {
                    let _ = this.update(cx, |this, cx| {
                        this.delta_busy = false;
                        this.delta_message_target = DeltaMessageTarget::Notifications;
                        this.delta_message = None;
                        cx.notify();
                    });
                }
                Err(error) => {
                    tracing::error!(error = %error, "Delta push token registration failed");
                    Self::report_error(&this, cx, format!("{error:#}"));
                }
            }
        })
        .detach();
        cx.notify();
    }

    /// Apply a completed network result onto the shared entity, clearing the
    /// busy state, or surface the error.
    fn apply_delta_result(
        this: &WeakEntity<Self>,
        delta_state: &Entity<DeltaState>,
        cx: &mut AsyncApp,
        snapshot: DeltaState,
        result: anyhow::Result<DeltaState>,
        target: DeltaMessageTarget,
    ) {
        match result {
            Ok(next) => {
                let applied = delta_state.update(cx, |state, cx| {
                    // Keep newer Delta state changes from being overwritten by a stale async result.
                    state.merge(delta::DeltaPatch::between(&snapshot, &next), cx)
                });
                let _ = this.update(cx, |this, cx| {
                    this.delta_busy = false;
                    this.delta_message_target = target;
                    this.delta_message = None;
                    cx.notify();
                });
                if !applied {
                    tracing::debug!(
                        "Delta async result completed after state changed; skipping stale update"
                    );
                }
            }
            Err(error) => {
                if target == DeltaMessageTarget::Notifications {
                    tracing::error!(error = %error, "Delta push token registration failed");
                } else {
                    tracing::error!(error = %error, "Delta setup operation failed");
                }
                Self::report_error(this, cx, format!("{error:#}"));
            }
        }
    }

    fn report_error(this: &WeakEntity<Self>, cx: &mut AsyncApp, message: String) {
        let _ = this.update(cx, |this, cx| {
            this.finish_delta_error(message);
            cx.notify();
        });
    }

    fn finish_delta_error(&mut self, message: String) {
        self.delta_busy = false;
        self.delta_message = Some(message);
    }

    fn profile_title(status: &delta::DeltaStatus) -> String {
        delta::account_label(status)
    }

    fn profile_summary(status: &delta::DeltaStatus) -> String {
        let stack = status
            .stack_id
            .map(short_id)
            .unwrap_or_else(|| "no stack".to_string());
        let node = status
            .node_id
            .map(short_id)
            .unwrap_or_else(|| "no node".to_string());
        format!("Stack {stack} · Node {node}")
    }

    fn push_summary(status: &delta::DeltaStatus) -> String {
        match (
            status.push_registered,
            status.push_provider.as_deref(),
            status.push_environment.as_deref(),
            status.signed_in,
        ) {
            (true, Some(provider), Some(environment), _) => {
                format!("{provider} {environment} token registered")
            }
            (true, Some(provider), None, _) => format!("{provider} token registered"),
            (false, Some(provider), _, false) => {
                format!("{provider} token saved, sign in to register")
            }
            (false, Some(provider), _, true) => format!("{provider} token not registered"),
            _ => "Request permission and register this device".to_string(),
        }
    }

    fn set_theme_preference(&self, preference: ThemePreference, cx: &mut Context<Self>) {
        platform_bridge::trigger_haptic(HapticFeedback::SelectionChanged);
        self.theme_state.update(cx, |state, cx| {
            state.set_preference(preference, cx);
        });
    }

    fn set_telemetry_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.telemetry_enabled == enabled {
            return;
        }
        platform_bridge::trigger_haptic(HapticFeedback::SelectionChanged);
        self.telemetry_enabled = enabled;
        settings::set_telemetry_enabled(enabled);
        cx.notify();
    }

    fn set_droplet_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.droplet_enabled == enabled {
            return;
        }
        platform_bridge::trigger_haptic(HapticFeedback::SelectionChanged);
        self.droplet_enabled = enabled;
        settings::set_droplet_enabled(enabled);
        cx.emit(SettingsEvent::DropletToggled(enabled));
        cx.notify();
    }

    fn set_extended_keypad(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.extended_keypad == enabled {
            return;
        }
        platform_bridge::trigger_haptic(HapticFeedback::SelectionChanged);
        self.extended_keypad = enabled;
        settings::set_extended_keypad(enabled);
        platform_bridge::bridge().set_keypad_layout(enabled, crate::key_bar::host_uses_cmd_slot());
        cx.notify();
    }

    fn set_key_bar_always_visible(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.key_bar_always_visible == enabled {
            return;
        }
        platform_bridge::trigger_haptic(HapticFeedback::SelectionChanged);
        self.key_bar_always_visible = enabled;
        settings::set_key_bar_always_visible(enabled);
        cx.notify();
    }

    fn open_telemetry_docs(&self) {
        platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
        platform_bridge::bridge().open_url(TELEMETRY_DOCS_URL);
    }

    fn open_privacy_policy(&self) {
        platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
        platform_bridge::bridge().open_url(PRIVACY_POLICY_URL);
    }

    fn show_test_alert(&self) {
        platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
        platform_bridge::show_alert(
            "Developer Alert",
            "This is a native alert presented from the Settings developer session.",
            vec![
                AlertButton::default("Primary"),
                AlertButton::cancel("Cancel"),
            ],
            |_| {},
        );
    }

    fn show_test_selection(&self) {
        platform_bridge::trigger_haptic(HapticFeedback::SelectionChanged);
        platform_bridge::show_selection(
            "Developer Selection",
            "Choose one of the native selection actions below.",
            vec![
                AlertButton::default("First Action"),
                AlertButton::default("Second Action"),
                AlertButton::destructive("Destructive Action"),
                AlertButton::cancel("Cancel"),
            ],
            |_| {},
        );
    }

    fn show_test_webview(&self) {
        use base64::Engine as _;
        platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);

        // Self-contained page that exercises the JS bridge (posts on load and on
        // tap) and offers a link to a blocked external origin.
        const PAGE: &str = r#"<!doctype html><meta name=viewport content="width=device-width,initial-scale=1">
<body style="font-family:-apple-system,system-ui,sans-serif;margin:0;padding:24px;background:#111;color:#eee">
<h2>Zedra Webview Test</h2>
<p id=out>loading…</p>
<button style="font-size:16px;padding:10px 16px" onclick="post('button tapped')">Post message</button>
<p><a href="https://example.com/blocked">Try blocked navigation</a></p>
<script>
function bridge(){
  if(window.webkit&&window.webkit.messageHandlers&&window.webkit.messageHandlers.zedra)return function(m){window.webkit.messageHandlers.zedra.postMessage(m)};
  if(window.zedra&&window.zedra.postMessage)return function(m){window.zedra.postMessage(m)};
  return null;
}
function post(m){var b=bridge();if(b)b(m)}
window.zedraSetStatus=function(s){document.getElementById('out').textContent=s}
console.log('bridge present: '+(bridge()?'yes':'no')+' (webkit='+(typeof window.webkit)+' zedra='+(typeof window.zedra)+')');
document.getElementById('out').textContent=bridge()?'ready (bridge ok)':'ready (no bridge)';
post('page loaded');
</script>"#;
        let data_url = format!(
            "data:text/html;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(PAGE)
        );

        crate::webview::open(
            crate::webview::WebviewConfig::new(data_url)
                .title("Webview Test")
                .on_message(|message| {
                    tracing::info!("webview: message: {message}");
                    // Echo back into the page to exercise Rust->web eval.
                    let escaped = message.replace('\\', "\\\\").replace('\'', "\\'");
                    crate::webview::eval_js(&format!("window.zedraSetStatus('got: {escaped}')"));
                })
                .on_navigate(|url| {
                    // Allow the initial data: load; block external https links.
                    if url.starts_with("https://") {
                        tracing::info!("webview: blocked navigation: {url}");
                        crate::webview::NavigationPolicy::Cancel
                    } else {
                        crate::webview::NavigationPolicy::Allow
                    }
                })
                .on_dismiss(|| tracing::info!("webview: dismissed")),
        );
    }

    fn show_test_custom_sheet(&self, cx: &mut Context<Self>) {
        platform_bridge::trigger_haptic(HapticFeedback::ImpactSoft);
        self.sheet_state.update(cx, |state, cx| {
            state.mark_launched(
                "Custom Sheet Canvas",
                "Shared state from the main app, rendered through a persistent GPUI sheet surface.",
            );
            cx.notify();
        });
        platform_bridge::show_custom_sheet(
            CustomSheetOptions {
                detents: vec![CustomSheetDetent::Medium, CustomSheetDetent::Large],
                initial_detent: CustomSheetDetent::Medium,
                shows_grabber: true,
                expands_on_scroll_edge: true,
                edge_attached_in_compact_height: false,
                width_follows_preferred_content_size_when_edge_attached: false,
                corner_radius: None,
                modal_in_presentation: false,
            },
            self.sheet_view.clone(),
        );
        view_telemetry::record(view_telemetry::CUSTOM_SHEET_DEMO);
    }
}

impl Focusable for SettingsView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let top_inset = platform_bridge::status_bar_inset();
        let bottom_inset = platform_bridge::home_indicator_inset();
        let status = self.status(cx);
        let delta_message = self.delta_message.clone();
        let profile_title = Self::profile_title(&status);
        let profile_initial = delta::account_initial(&status);
        let profile_summary = status_or_summary(
            Self::profile_summary(&status),
            delta_message.as_deref(),
            self.delta_message_target,
            DeltaMessageTarget::Profile,
        );
        let sign_in_summary = status_or_summary(
            "Choose a sign-in method".to_string(),
            delta_message.as_deref(),
            self.delta_message_target,
            DeltaMessageTarget::Profile,
        );
        let push_summary = status_or_summary(
            Self::push_summary(&status),
            delta_message.as_deref(),
            self.delta_message_target,
            DeltaMessageTarget::Notifications,
        );
        let signed_in = status.signed_in;
        let sign_in_title = if self.delta_busy {
            "Signing in..."
        } else {
            "Sign In"
        };
        let preference = self.theme_state.read(cx).preference();
        let telemetry_enabled = self.telemetry_enabled;
        let droplet_enabled = self.droplet_enabled;
        let key_bar_always_visible = self.key_bar_always_visible;
        let extended_keypad = self.extended_keypad;

        div()
            .id("settings-view")
            .track_focus(&self.focus_handle)
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
                                    .id("settings-back-button")
                                    .hit_slop(px(10.0))
                                    .cursor_pointer()
                                    .on_press(cx.listener(|_this, _event, _window, cx| {
                                        cx.emit(SettingsEvent::NavigateHome);
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
                                    .child("Settings"),
                            ),
                    ),
            )
            .child(
                div()
                    .id("settings-scroll")
                    .overflow_y_scroll()
                    .flex_1()
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
                            .gap(px(theme::SPACING_MD))
                            .child(section_header(cx, "Profile"))
                            .when(signed_in, |this| {
                                this.child(profile_info_row(
                                    cx,
                                    "settings-delta-profile",
                                    profile_initial,
                                    profile_title,
                                    profile_summary,
                                    cx.listener(|_this, _event, _window, cx| {
                                        cx.emit(SettingsEvent::OpenAccount);
                                    }),
                                ))
                            })
                            .when(!signed_in, |this| {
                                this.child(
                                    action_row(
                                        cx,
                                        "settings-delta-sign-in",
                                        sign_in_title,
                                        sign_in_summary,
                                    )
                                    .on_press(cx.listener(|this, _event, _window, cx| {
                                        this.show_sign_in_methods(cx);
                                    })),
                                )
                            })
                            .child(section_header(cx, "Notifications"))
                            .child(
                                action_row(
                                    cx,
                                    "settings-delta-push-token",
                                    "Enable Notifications",
                                    push_summary,
                                )
                                .on_press(cx.listener(|this, _event, _window, cx| {
                                    this.request_push_token(cx);
                                })),
                            )
                            .child(section_header(cx, "Appearance"))
                            .child(appearance_theme_toggle(
                                cx,
                                preference,
                                cx.listener(|this, _event, _window, cx| {
                                    this.set_theme_preference(ThemePreference::Dark, cx);
                                }),
                                cx.listener(|this, _event, _window, cx| {
                                    this.set_theme_preference(ThemePreference::Light, cx);
                                }),
                            ))
                            .when(cfg!(target_os = "ios"), |this| {
                                this.child(bool_setting_toggle(
                                    cx,
                                    "settings-droplet-on",
                                    "settings-droplet-off",
                                    "settings-droplet-toggle",
                                    "Water droplet",
                                    "A playful droplet to flick around",
                                    droplet_enabled,
                                    cx.listener(|this, _event, _window, cx| {
                                        this.set_droplet_enabled(true, cx);
                                    }),
                                    cx.listener(|this, _event, _window, cx| {
                                        this.set_droplet_enabled(false, cx);
                                    }),
                                ))
                            })
                            .child(section_header(cx, "Terminal"))
                            .child(bool_setting_toggle(
                                cx,
                                "settings-key-bar-on",
                                "settings-key-bar-off",
                                "settings-key-bar-toggle",
                                "Always show keypad",
                                "Esc, Tab, and arrows stay up when the keyboard is down",
                                key_bar_always_visible,
                                cx.listener(|this, _event, _window, cx| {
                                    this.set_key_bar_always_visible(true, cx);
                                }),
                                cx.listener(|this, _event, _window, cx| {
                                    this.set_key_bar_always_visible(false, cx);
                                }),
                            ))
                            .child(bool_setting_toggle(
                                cx,
                                "settings-extended-keypad-on",
                                "settings-extended-keypad-off",
                                "settings-extended-keypad-toggle",
                                "Extended keypad",
                                "Adds modifiers, symbols, and a swipe-left composer",
                                extended_keypad,
                                cx.listener(|this, _event, _window, cx| {
                                    this.set_extended_keypad(true, cx);
                                }),
                                cx.listener(|this, _event, _window, cx| {
                                    this.set_extended_keypad(false, cx);
                                }),
                            ))
                            .child(section_header(cx, "Privacy"))
                            .child(telemetry_toggle(
                                cx,
                                telemetry_enabled,
                                cx.listener(|this, _event, _window, cx| {
                                    this.set_telemetry_enabled(true, cx);
                                }),
                                cx.listener(|this, _event, _window, cx| {
                                    this.set_telemetry_enabled(false, cx);
                                }),
                            ))
                            .child(
                                action_row(
                                    cx,
                                    "settings-telemetry-docs",
                                    "Telemetry docs",
                                    "zedra.dev/docs/telemetry",
                                )
                                .on_press(cx.listener(|this, _event, _window, _cx| {
                                    this.open_telemetry_docs();
                                })),
                            )
                            .child(
                                action_row(
                                    cx,
                                    "settings-privacy-docs",
                                    "Privacy policy",
                                    "zedra.dev/privacy",
                                )
                                .on_press(cx.listener(|this, _event, _window, _cx| {
                                    this.open_privacy_policy();
                                })),
                            )
                            .when(cfg!(debug_assertions), |section| {
                                section
                                    .child(section_header(cx, "Developer"))
                                    .child(
                                        action_row(
                                            cx,
                                            "settings-test-alert",
                                            "Native Alert",
                                            "Native confirmation/failure prompts",
                                        )
                                        .on_press(cx.listener(|this, _event, _window, _cx| {
                                            this.show_test_alert();
                                        })),
                                    )
                                    .child(
                                        action_row(
                                            cx,
                                            "settings-test-selection",
                                            "Native Selection",
                                            "Action sheet selection and behavior",
                                        )
                                        .on_press(cx.listener(|this, _event, _window, _cx| {
                                            this.show_test_selection();
                                        })),
                                    )
                                    .child(
                                        action_row(
                                            cx,
                                            "settings-test-custom-sheet",
                                            "Custom Sheet",
                                            "Native sheet with GPUI-rendered content",
                                        )
                                        .on_press(cx.listener(|this, _event, _window, cx| {
                                            this.show_test_custom_sheet(cx);
                                        })),
                                    )
                                    .child(
                                        action_row(
                                            cx,
                                            "settings-test-webview",
                                            "Webview",
                                            "JS messaging, eval, and navigation interception",
                                        )
                                        .on_press(cx.listener(|this, _event, _window, _cx| {
                                            this.show_test_webview();
                                        })),
                                    )
                                    .child(
                                        action_row(
                                            cx,
                                            "settings-web-tunnel",
                                            "Web tunnel",
                                            "Manage localhost listeners bound on this device",
                                        )
                                        .on_press(cx.listener(|_this, _event, _window, cx| {
                                            cx.emit(SettingsEvent::OpenWebTunnel);
                                        })),
                                    )
                                    .child(
                                        div()
                                            .text_color(rgb(theme::text_muted(cx)))
                                            .text_size(px(theme::FONT_DETAIL))
                                            .font_family(fonts::MONO_FONT_FAMILY)
                                            .child(
                                                "QR scanner and dictation preview remain separate native flows.",
                                            ),
                                    )
                            }),
                    ),
            )
    }
}

fn section_header(cx: &App, title: &'static str) -> Div {
    div()
        .pt(px(12.0))
        .pb(px(10.0))
        .border_b_1()
        .border_color(rgb(theme::border_subtle(cx)))
        .child(
            div()
                .text_color(rgb(theme::text_primary(cx)))
                .text_size(px(theme::FONT_HEADING))
                .font_family(fonts::MONO_FONT_FAMILY)
                .font_weight(FontWeight::MEDIUM)
                .child(title),
        )
}

/// Settings row with a compact segmented appearance control.
fn appearance_theme_toggle(
    cx: &App,
    preference: ThemePreference,
    on_dark: impl Fn(&PressEvent, &mut Window, &mut App) + 'static,
    on_light: impl Fn(&PressEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let is_dark = preference == ThemePreference::Dark;

    div()
        .id("settings-appearance-toggle")
        .min_w_0()
        .min_h(px(32.0))
        .py(px(2.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::SPACING_MD))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_row()
                .items_center()
                .child(
                    div()
                        .text_color(rgb(theme::text_secondary(cx)))
                        .text_size(px(theme::FONT_BODY))
                        .font_family(fonts::MONO_FONT_FAMILY)
                        .font_weight(FontWeight::MEDIUM)
                        .child("Theme"),
                ),
        )
        .child(
            div()
                .flex_none()
                .rounded(px(8.0))
                .border_1()
                .border_color(rgb(theme::border_default(cx)))
                .bg(rgb(theme::bg_surface(cx)))
                .flex()
                .flex_row()
                .child(theme_toggle_segment(
                    cx,
                    "settings-theme-dark",
                    "icons/moon.svg",
                    is_dark,
                    on_dark,
                ))
                .child(div().w(px(1.0)).h_full().bg(rgb(theme::border_subtle(cx))))
                .child(theme_toggle_segment(
                    cx,
                    "settings-theme-light",
                    "icons/sun.svg",
                    !is_dark,
                    on_light,
                )),
        )
}

fn theme_toggle_segment(
    cx: &App,
    id: &'static str,
    icon_path: &'static str,
    selected: bool,
    on_press: impl Fn(&PressEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let mut segment = div()
        .id(id)
        .min_w(px(36.0))
        .h(px(24.0))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hit_slop(px(6.0))
        .on_press(on_press);

    if selected {
        segment = segment.bg(rgb(theme::bg_card(cx)));
    }

    segment.child(
        svg()
            .path(icon_path)
            .size(px(theme::ICON_XS))
            .text_color(rgb(if selected {
                theme::text_primary(cx)
            } else {
                theme::text_muted(cx)
            })),
    )
}

/// Settings row toggling anonymous usage telemetry on or off.
fn telemetry_toggle(
    cx: &App,
    enabled: bool,
    on_enable: impl Fn(&PressEvent, &mut Window, &mut App) + 'static,
    on_disable: impl Fn(&PressEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    if cfg!(feature = "no-telemetry") {
        let control = div()
            .flex_none()
            .rounded(px(8.0))
            .border_1()
            .border_color(rgb(theme::border_subtle(cx)))
            .bg(rgb(theme::bg_surface(cx)))
            .opacity(0.45)
            .child(
                div()
                    .min_w(px(72.0))
                    .h(px(24.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(theme::FONT_DETAIL))
                    .font_family(fonts::MONO_FONT_FAMILY)
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(theme::text_muted(cx)))
                    .child("Off"),
            )
            .into_any_element();
        return toggle_row(
            cx,
            "settings-telemetry-toggle",
            "Telemetry metrics",
            "Disabled by build flag",
            theme::text_muted(cx),
            control,
        )
        .into_any_element();
    }

    bool_setting_toggle(
        cx,
        "settings-telemetry-on",
        "settings-telemetry-off",
        "settings-telemetry-toggle",
        "Telemetry metrics",
        "Send anonymous usage data",
        enabled,
        on_enable,
        on_disable,
    )
    .into_any_element()
}

/// A settings row whose control is an On/Off segmented toggle.
fn bool_setting_toggle(
    cx: &App,
    on_id: &'static str,
    off_id: &'static str,
    row_id: &'static str,
    title: &'static str,
    description: &'static str,
    enabled: bool,
    on_enable: impl Fn(&PressEvent, &mut Window, &mut App) + 'static,
    on_disable: impl Fn(&PressEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let control = segmented_toggle(cx, on_id, off_id, enabled, on_enable, on_disable);
    toggle_row(
        cx,
        row_id,
        title,
        description,
        theme::text_secondary(cx),
        control,
    )
}

fn segmented_toggle(
    cx: &App,
    on_id: &'static str,
    off_id: &'static str,
    enabled: bool,
    on_enable: impl Fn(&PressEvent, &mut Window, &mut App) + 'static,
    on_disable: impl Fn(&PressEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .flex_none()
        .rounded(px(8.0))
        .border_1()
        .border_color(rgb(theme::border_default(cx)))
        .bg(rgb(theme::bg_surface(cx)))
        .flex()
        .flex_row()
        .child(toggle_segment(cx, on_id, "On", enabled, on_enable))
        .child(div().w(px(1.0)).h_full().bg(rgb(theme::border_subtle(cx))))
        .child(toggle_segment(cx, off_id, "Off", !enabled, on_disable))
        .into_any_element()
}

fn toggle_row(
    cx: &App,
    id: &'static str,
    title: &'static str,
    description: &'static str,
    title_color: u32,
    control: AnyElement,
) -> AnyElement {
    div()
        .id(id)
        .min_w_0()
        .min_h(px(32.0))
        .py(px(2.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::SPACING_MD))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(3.0))
                .overflow_hidden()
                .child(
                    div()
                        .text_color(rgb(title_color))
                        .text_size(px(theme::FONT_BODY))
                        .font_family(fonts::MONO_FONT_FAMILY)
                        .font_weight(FontWeight::MEDIUM)
                        .child(title),
                )
                .child(
                    div()
                        .text_color(rgb(theme::text_muted(cx)))
                        .text_size(px(theme::FONT_DETAIL))
                        .font_family(fonts::MONO_FONT_FAMILY)
                        .child(description),
                ),
        )
        .child(control)
        .into_any_element()
}

fn toggle_segment(
    cx: &App,
    id: &'static str,
    label: &'static str,
    selected: bool,
    on_press: impl Fn(&PressEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let mut segment = div()
        .id(id)
        .min_w(px(36.0))
        .h(px(24.0))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hit_slop(px(6.0))
        .on_press(on_press);

    if selected {
        segment = segment.bg(rgb(theme::bg_card(cx)));
    }

    segment.child(
        div()
            .text_size(px(theme::FONT_DETAIL))
            .font_family(fonts::MONO_FONT_FAMILY)
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(if selected {
                theme::text_primary(cx)
            } else {
                theme::text_muted(cx)
            }))
            .child(label),
    )
}

fn action_row(
    cx: &App,
    id: &'static str,
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
) -> Stateful<Div> {
    let title = title.into();
    let description = description.into();
    div()
        .id(id)
        .min_w_0()
        .min_h(px(56.0))
        .py(px(10.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::SPACING_MD))
        .cursor_pointer()
        .child(
            div()
                .flex_1()
                .flex()
                .flex_col()
                .gap(px(3.0))
                .overflow_hidden()
                .child(
                    div()
                        .text_color(rgb(theme::text_secondary(cx)))
                        .text_size(px(theme::FONT_BODY))
                        .font_family(fonts::MONO_FONT_FAMILY)
                        .font_weight(FontWeight::MEDIUM)
                        .child(title),
                )
                .child(
                    div()
                        .text_color(rgb(theme::text_muted(cx)))
                        .text_size(px(theme::FONT_DETAIL))
                        .font_family(fonts::MONO_FONT_FAMILY)
                        .child(description),
                ),
        )
        .child(
            div().pl(px(8.0)).child(
                svg()
                    .path("icons/chevron-right.svg")
                    .size(px(theme::ICON_SM))
                    .text_color(rgb(theme::text_muted(cx))),
            ),
        )
}

fn status_or_summary(
    summary: String,
    status: Option<&str>,
    status_target: DeltaMessageTarget,
    row_target: DeltaMessageTarget,
) -> String {
    if status_target == row_target {
        if let Some(status) = status.filter(|message| !message.trim().is_empty()) {
            return status.to_string();
        }
    }
    summary
}

fn profile_info_row(
    cx: &App,
    id: &'static str,
    initials: impl Into<SharedString>,
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
    on_press: impl Fn(&PressEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let initials = initials.into();
    let title = title.into();
    let description = description.into();
    let mut row = div()
        .id(id)
        .min_w_0()
        .min_h(px(56.0))
        .py(px(10.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::SPACING_MD))
        .child(
            div()
                .size(px(34.0))
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
                .child(initials),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(3.0))
                .overflow_hidden()
                .child(
                    div()
                        .text_color(rgb(theme::text_secondary(cx)))
                        .text_size(px(theme::FONT_BODY))
                        .font_family(fonts::MONO_FONT_FAMILY)
                        .font_weight(FontWeight::MEDIUM)
                        .child(title),
                )
                .child(
                    div()
                        .text_color(rgb(theme::text_muted(cx)))
                        .text_size(px(theme::FONT_DETAIL))
                        .font_family(fonts::MONO_FONT_FAMILY)
                        .child(description),
                ),
        );

    row = row.cursor_pointer().on_press(on_press).child(
        div().pl(px(8.0)).child(
            svg()
                .path("icons/chevron-right.svg")
                .size(px(theme::ICON_SM))
                .text_color(rgb(theme::text_muted(cx))),
        ),
    );
    row
}

fn short_id(id: uuid::Uuid) -> String {
    id.to_string().chars().take(8).collect()
}
