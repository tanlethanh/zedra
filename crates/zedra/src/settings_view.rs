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
                    state.apply_if_current(&snapshot, next, cx)
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

    fn request_push_token(&mut self, cx: &mut Context<Self>) {
        if self.delta_busy {
            return;
        }
        self.delta_busy = true;
        self.delta_message_target = DeltaMessageTarget::Notifications;
        self.delta_message = Some("Requesting notification permission".to_string());
        platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
        let (tx, rx) = oneshot::channel();
        platform_bridge::request_delta_push_token(move |result| {
            let _ = tx.send(result);
        });
        let delta_state = self.delta_state.clone();
        cx.spawn(async move |this, cx| {
            let token = match rx.await {
                Ok(Ok(token)) => token,
                Ok(Err(message)) => {
                    tracing::error!(error = %message, "Push token acquisition failed before Delta registration");
                    return Self::report_error(&this, cx, message);
                }
                Err(_) => return,
            };
            let _ = this.update(cx, |this, cx| {
                this.delta_message = Some("Registering push token".to_string());
                cx.notify();
            });
            let snapshot = delta_state.read_with(cx, |state, _| state.snapshot());
            let result = Tokio::spawn_result(
                cx,
                delta::register_push_token(
                    snapshot.clone(),
                    token.provider,
                    token.token,
                    token.environment,
                ),
            )
            .await;
            Self::apply_delta_result(
                &this,
                &delta_state,
                cx,
                snapshot,
                result,
                DeltaMessageTarget::Notifications,
            );
        })
        .detach();
        cx.notify();
    }

    fn confirm_logout(&mut self, cx: &mut Context<Self>) {
        self.delta_message_target = DeltaMessageTarget::Profile;
        let snapshot = self.delta_state.read(cx).snapshot();
        match delta::sign_out(snapshot) {
            Ok(next) => {
                self.delta_state
                    .update(cx, |state, cx| state.apply(next, cx));
                self.delta_busy = false;
                self.delta_message = Some("Signed out of Delta".to_string());
            }
            Err(error) => self.finish_delta_error(format!("{error:#}")),
        }
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
                    state.apply_if_current(&snapshot, next, cx)
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
        status
            .email
            .clone()
            .unwrap_or_else(|| "Signed in".to_string())
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

    fn show_logout_confirmation(&self, cx: &mut Context<Self>) {
        platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
        let (tx, rx) = oneshot::channel();
        platform_bridge::show_alert(
            "Log out of Delta?",
            "",
            vec![
                AlertButton::destructive("Log Out"),
                AlertButton::cancel("Cancel"),
            ],
            move |button_index| {
                let _ = tx.send(button_index);
            },
        );
        cx.spawn(async move |this, cx| {
            if let Ok(0) = rx.await {
                let _ = this.update(cx, |this, cx| this.confirm_logout(cx));
            }
        })
        .detach();
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
                    tracing::info!("[debug:webview] message: {message}");
                    // Echo back into the page to exercise Rust->web eval.
                    let escaped = message.replace('\\', "\\\\").replace('\'', "\\'");
                    crate::webview::eval_js(&format!("window.zedraSetStatus('got: {escaped}')"));
                })
                .on_navigate(|url| {
                    // Allow the initial data: load; block external https links.
                    if url.starts_with("https://") {
                        tracing::info!("[debug:webview] blocked navigation: {url}");
                        crate::webview::NavigationPolicy::Cancel
                    } else {
                        crate::webview::NavigationPolicy::Allow
                    }
                })
                .on_dismiss(|| tracing::info!("[debug:webview] dismissed")),
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
        let profile_initial = profile_title
            .chars()
            .next()
            .unwrap_or('Z')
            .to_ascii_uppercase()
            .to_string();
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
                            .max_w(px(520.0))
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
                                    Some(cx.listener(|this, _event, _window, cx| {
                                        this.show_logout_confirmation(cx);
                                    })),
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
                .child(
                    div()
                        .w(px(1.0))
                        .h(px(22.0))
                        .bg(rgb(theme::border_subtle(cx))),
                )
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
) -> impl IntoElement {
    let compiled_out = cfg!(feature = "no-telemetry");
    let control = if compiled_out {
        div()
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
            .into_any_element()
    } else {
        div()
            .flex_none()
            .rounded(px(8.0))
            .border_1()
            .border_color(rgb(theme::border_default(cx)))
            .bg(rgb(theme::bg_surface(cx)))
            .flex()
            .flex_row()
            .child(telemetry_toggle_segment(
                cx,
                "settings-telemetry-on",
                "On",
                enabled,
                on_enable,
            ))
            .child(
                div()
                    .w(px(1.0))
                    .h(px(22.0))
                    .bg(rgb(theme::border_subtle(cx))),
            )
            .child(telemetry_toggle_segment(
                cx,
                "settings-telemetry-off",
                "Off",
                !enabled,
                on_disable,
            ))
            .into_any_element()
    };

    div()
        .id("settings-telemetry-toggle")
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
                        .text_color(rgb(if compiled_out {
                            theme::text_muted(cx)
                        } else {
                            theme::text_secondary(cx)
                        }))
                        .text_size(px(theme::FONT_BODY))
                        .font_family(fonts::MONO_FONT_FAMILY)
                        .font_weight(FontWeight::MEDIUM)
                        .child("Telemetry metrics"),
                )
                .child(
                    div()
                        .text_color(rgb(theme::text_muted(cx)))
                        .text_size(px(theme::FONT_DETAIL))
                        .font_family(fonts::MONO_FONT_FAMILY)
                        .child(if compiled_out {
                            "Disabled by build flag"
                        } else {
                            "Send anonymous usage data"
                        }),
                ),
        )
        .child(control)
}

fn telemetry_toggle_segment(
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
    on_logout: Option<impl Fn(&PressEvent, &mut Window, &mut App) + 'static>,
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

    if let Some(on_logout) = on_logout {
        row = row.child(logout_button(cx, on_logout));
    }

    row
}

fn logout_button(
    cx: &App,
    on_press: impl Fn(&PressEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id("settings-delta-logout")
        .flex_none()
        .size(px(16.0))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hit_slop(px(14.0))
        .on_press(on_press)
        .child(
            svg()
                .path("icons/log-out.svg")
                .size(px(12.0))
                .text_color(rgb(theme::accent_red(cx))),
        )
}

fn short_id(id: uuid::Uuid) -> String {
    id.to_string().chars().take(8).collect()
}
