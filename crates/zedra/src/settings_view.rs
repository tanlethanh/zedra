use std::time::Duration;

use gpui::{prelude::FluentBuilder as _, *};

use crate::delta;
use crate::fonts;
use crate::pending::{PendingSlot, spawn_periodic_task};
use crate::platform_bridge::{
    self, AlertButton, CustomSheetDetent, CustomSheetOptions, HapticFeedback,
    NativeNotificationKind, NativeNotificationOptions,
};
use crate::settings::ThemeState;
use crate::sheet_demo_state::SheetDemoState;
use crate::telemetry::view_telemetry;
use crate::theme::{self, ThemePreference};

#[derive(Clone, Debug)]
pub enum SettingsEvent {
    NavigateHome,
}

impl EventEmitter<SettingsEvent> for SettingsView {}

enum DeltaSettingsEvent {
    StartGoogleSignIn,
    GoogleSignIn(Result<platform_bridge::DeltaGoogleSignInResult, String>),
    StartAppleSignIn,
    AppleSignIn(Result<platform_bridge::DeltaAppleSignInResult, String>),
    PushToken(Result<platform_bridge::DeltaPushTokenResult, String>),
    ConfirmLogout,
    ReconcileMobileNode(Result<delta::MobileNodeReconcileResult, String>),
    OperationComplete {
        result: Result<delta::DeltaStatus, String>,
        success_message: Option<&'static str>,
        target: DeltaMessageTarget,
    },
}

static PENDING_DELTA_EVENT: PendingSlot<DeltaSettingsEvent> = PendingSlot::new();

pub fn reconcile_delta_mobile_node_on_launch() {
    zedra_session::session_runtime().spawn(async {
        let result = delta::reconcile_mobile_node()
            .await
            .map_err(|error| format!("{error:#}"));
        PENDING_DELTA_EVENT.set(DeltaSettingsEvent::ReconcileMobileNode(result));
    });
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DeltaMessageTarget {
    Profile,
    Notifications,
}

pub struct SettingsView {
    focus_handle: FocusHandle,
    theme_state: Entity<ThemeState>,
    sheet_state: Entity<SheetDemoState>,
    sheet_view: Entity<crate::sheet_demo_view::SheetDemoView>,
    delta_status: delta::DeltaStatus,
    delta_message: Option<String>,
    delta_message_target: DeltaMessageTarget,
    delta_busy: bool,
    _delta_task: Task<()>,
}

impl SettingsView {
    pub fn new(theme_state: Entity<ThemeState>, cx: &mut Context<Self>) -> Self {
        let sheet_state = cx.new(|cx| SheetDemoState::new(cx));
        let sheet_view =
            cx.new(|cx| crate::sheet_demo_view::SheetDemoView::new(sheet_state.clone(), cx));
        let delta_task = spawn_periodic_task(cx, Duration::from_millis(120), |this, cx| {
            this.process_delta_events(cx);
        });
        Self {
            focus_handle: cx.focus_handle(),
            theme_state,
            sheet_state,
            sheet_view,
            delta_status: delta::status(),
            delta_message: None,
            delta_message_target: DeltaMessageTarget::Profile,
            delta_busy: false,
            _delta_task: delta_task,
        }
    }

    fn start_oauth_sign_in(&mut self, cx: &mut Context<Self>, message: &str, start: impl FnOnce()) {
        if self.delta_busy {
            return;
        }
        self.delta_busy = true;
        self.delta_message_target = DeltaMessageTarget::Profile;
        self.delta_message = Some(message.to_string());
        platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
        start();
        cx.notify();
    }

    fn start_apple_sign_in(&mut self, cx: &mut Context<Self>) {
        self.start_oauth_sign_in(cx, "Opening Apple sign-in", || {
            platform_bridge::start_delta_apple_sign_in(|result| {
                PENDING_DELTA_EVENT.set(DeltaSettingsEvent::AppleSignIn(result));
            });
        });
    }

    fn start_google_sign_in(&mut self, cx: &mut Context<Self>) {
        self.start_oauth_sign_in(cx, "Opening Google sign-in", || {
            platform_bridge::start_delta_google_sign_in(|result| {
                PENDING_DELTA_EVENT.set(DeltaSettingsEvent::GoogleSignIn(result));
            });
        });
    }

    fn spawn_sign_in_task(
        &mut self,
        sign_in: impl std::future::Future<Output = anyhow::Result<delta::DeltaStatus>> + Send + 'static,
    ) {
        self.delta_message_target = DeltaMessageTarget::Profile;
        self.delta_message = Some("Registering Delta mobile node".to_string());
        zedra_session::session_runtime().spawn(async move {
            let result = sign_in.await.map_err(|err| format!("{err:#}"));
            PENDING_DELTA_EVENT.set(DeltaSettingsEvent::OperationComplete {
                result,
                success_message: None,
                target: DeltaMessageTarget::Profile,
            });
        });
    }

    fn show_sign_in_methods(&mut self, cx: &mut Context<Self>) {
        if self.delta_busy {
            return;
        }
        self.delta_message_target = DeltaMessageTarget::Profile;
        self.delta_message = None;
        platform_bridge::trigger_haptic(HapticFeedback::SelectionChanged);
        platform_bridge::show_selection(
            "Sign In",
            "Choose a sign-in method for Delta.",
            vec![
                AlertButton::default("Sign in with Google").image("Google"),
                AlertButton::default("Sign in with Apple").image("Apple"),
            ],
            |result| match result {
                Some(0) => PENDING_DELTA_EVENT.set(DeltaSettingsEvent::StartGoogleSignIn),
                Some(1) => PENDING_DELTA_EVENT.set(DeltaSettingsEvent::StartAppleSignIn),
                _ => {}
            },
        );
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
        platform_bridge::request_delta_push_token(|result| {
            PENDING_DELTA_EVENT.set(DeltaSettingsEvent::PushToken(result));
        });
        cx.notify();
    }

    fn process_delta_events(&mut self, cx: &mut Context<Self>) {
        let Some(event) = PENDING_DELTA_EVENT.take() else {
            return;
        };

        match event {
            DeltaSettingsEvent::StartGoogleSignIn => {
                self.start_google_sign_in(cx);
            }
            DeltaSettingsEvent::StartAppleSignIn => {
                self.start_apple_sign_in(cx);
            }
            DeltaSettingsEvent::AppleSignIn(Ok(result)) => {
                self.spawn_sign_in_task(delta::sign_in_with_apple(result.id_token, result.email));
            }
            DeltaSettingsEvent::AppleSignIn(Err(message))
            | DeltaSettingsEvent::GoogleSignIn(Err(message)) => {
                self.finish_delta_error(message);
            }
            DeltaSettingsEvent::GoogleSignIn(Ok(result)) => {
                self.spawn_sign_in_task(delta::sign_in_with_google(result.id_token, result.email));
            }
            DeltaSettingsEvent::PushToken(Ok(result)) => {
                self.delta_message_target = DeltaMessageTarget::Notifications;
                self.delta_message = Some("Registering push token".to_string());
                zedra_session::session_runtime().spawn(async move {
                    let result = delta::register_push_token(
                        result.provider,
                        result.token,
                        result.environment,
                    )
                    .await
                    .map_err(|err| format!("{err:#}"));
                    PENDING_DELTA_EVENT.set(DeltaSettingsEvent::OperationComplete {
                        result,
                        success_message: Some("Notifications enabled"),
                        target: DeltaMessageTarget::Notifications,
                    });
                });
            }
            DeltaSettingsEvent::PushToken(Err(message)) => {
                tracing::error!(error = %message, "Push token acquisition failed before Delta registration");
                self.finish_delta_error(message);
            }
            DeltaSettingsEvent::ConfirmLogout => {
                self.delta_message_target = DeltaMessageTarget::Profile;
                match delta::sign_out().map_err(|err| format!("{err:#}")) {
                    Ok(status) => {
                        self.delta_status = status;
                        self.delta_busy = false;
                        self.delta_message = Some("Signed out of Delta".to_string());
                    }
                    Err(message) => {
                        self.finish_delta_error(message);
                    }
                }
            }
            DeltaSettingsEvent::ReconcileMobileNode(result) => match result {
                Ok(result) => {
                    tracing::info!(?result, "Delta mobile node reconciliation completed");
                    if result == delta::MobileNodeReconcileResult::SignedOut {
                        self.delta_status = delta::status();
                    }
                }
                Err(error) => {
                    tracing::warn!("Delta mobile node reconciliation failed: {error}")
                }
            },
            DeltaSettingsEvent::OperationComplete {
                result: Ok(status),
                success_message: _,
                target,
            } => {
                self.delta_status = status;
                self.delta_busy = false;
                self.delta_message_target = target;
                self.delta_message = None;
            }
            DeltaSettingsEvent::OperationComplete {
                result: Err(message),
                target,
                ..
            } => {
                if target == DeltaMessageTarget::Notifications {
                    tracing::error!(error = %message, "Delta push token registration failed");
                } else {
                    tracing::error!(error = %message, "Delta setup operation failed");
                }
                self.finish_delta_error(message);
            }
        }
        cx.notify();
    }

    fn finish_delta_error(&mut self, message: String) {
        self.delta_busy = false;
        self.delta_message = Some(message.clone());
        platform_bridge::show_native_notification(
            NativeNotificationOptions::new("Delta setup failed")
                .message(message)
                .kind(NativeNotificationKind::Error)
                .system_image("exclamationmark.triangle")
                .duration_secs(4.2),
        );
    }

    fn profile_title(&self) -> String {
        self.delta_status
            .email
            .clone()
            .unwrap_or_else(|| "Signed in".to_string())
    }

    fn profile_summary(&self) -> String {
        let stack = self
            .delta_status
            .stack_id
            .map(short_id)
            .unwrap_or_else(|| "no stack".to_string());
        let node = self
            .delta_status
            .mobile_node_id
            .map(short_id)
            .unwrap_or_else(|| "no node".to_string());
        format!("Stack {stack} · Node {node}")
    }

    fn push_summary(&self) -> String {
        match (
            self.delta_status.push_registered,
            self.delta_status.push_provider.as_deref(),
            self.delta_status.push_environment.as_deref(),
            self.delta_status.signed_in,
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

    fn show_logout_confirmation(&self) {
        platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
        platform_bridge::show_alert(
            "",
            "Log out of Delta?",
            vec![
                AlertButton::destructive("Log Out"),
                AlertButton::cancel("Cancel"),
            ],
            |button_index| {
                if button_index == 0 {
                    PENDING_DELTA_EVENT.set(DeltaSettingsEvent::ConfirmLogout);
                }
            },
        );
    }

    fn set_theme_preference(&self, preference: ThemePreference, cx: &mut Context<Self>) {
        platform_bridge::trigger_haptic(HapticFeedback::SelectionChanged);
        self.theme_state.update(cx, |state, cx| {
            state.set_preference(preference, cx);
        });
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

    fn show_test_native_notification(&self) {
        platform_bridge::show_native_notification(
            NativeNotificationOptions::new("Terminal created")
                .message("Background mock notification for the bubble stack.")
                .system_image("terminal")
                .duration_secs(3.8),
        );
        platform_bridge::show_native_notification_with_action(
            NativeNotificationOptions::new("Agent completed")
                .message("Developer mock notification from Settings.")
                .image("AgentCodex")
                .kind(NativeNotificationKind::Success)
                .duration_secs(3.4),
            || {
                platform_bridge::show_native_notification(
                    NativeNotificationOptions::new("Notification tapped")
                        .message("Callback action fired from the native banner.")
                        .system_image("hand.tap")
                        .duration_secs(2.4),
                );
            },
        );
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
        let delta_message = self.delta_message.clone();
        let profile_title = self.profile_title();
        let profile_initial = profile_title
            .chars()
            .next()
            .unwrap_or('Z')
            .to_ascii_uppercase()
            .to_string();
        let profile_summary = status_or_summary(
            self.profile_summary(),
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
            self.push_summary(),
            delta_message.as_deref(),
            self.delta_message_target,
            DeltaMessageTarget::Notifications,
        );
        let sign_in_title = if self.delta_busy {
            "Signing in..."
        } else {
            "Sign In"
        };
        let preference = self.theme_state.read(cx).preference();

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
                            .when(self.delta_status.signed_in, |this| {
                                this.child(profile_info_row(
                                    cx,
                                    "settings-delta-profile",
                                    profile_initial,
                                    profile_title,
                                    profile_summary,
                                    Some(cx.listener(|this, _event, _window, _cx| {
                                        this.show_logout_confirmation();
                                    })),
                                ))
                            })
                            .when(!self.delta_status.signed_in, |this| {
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
                                            "settings-test-native-notification",
                                            "Native Notification",
                                            "In-app glass banner presentation",
                                        )
                                        .on_press(cx.listener(|this, _event, _window, _cx| {
                                            this.show_test_native_notification();
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
        .min_w(px(42.0))
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
