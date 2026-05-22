use gpui::{prelude::FluentBuilder as _, *};

use crate::fonts;
use crate::platform_bridge::{
    self, AlertButton, CustomSheetDetent, CustomSheetOptions, HapticFeedback,
    NativeNotificationKind, NativeNotificationOptions,
};
use crate::sheet_demo_state::SheetDemoState;
use crate::telemetry::view_telemetry;
use crate::theme::{self, ThemePreference};
use crate::theme_state::ThemeState;

#[derive(Clone, Debug)]
pub enum SettingsEvent {
    NavigateHome,
}

impl EventEmitter<SettingsEvent> for SettingsView {}

pub struct SettingsView {
    focus_handle: FocusHandle,
    theme_state: Entity<ThemeState>,
    sheet_state: Entity<SheetDemoState>,
    sheet_view: Entity<crate::sheet_demo_view::SheetDemoView>,
}

impl SettingsView {
    pub fn new(theme_state: Entity<ThemeState>, cx: &mut Context<Self>) -> Self {
        let sheet_state = cx.new(|cx| SheetDemoState::new(cx));
        let sheet_view =
            cx.new(|cx| crate::sheet_demo_view::SheetDemoView::new(sheet_state.clone(), cx));
        Self {
            focus_handle: cx.focus_handle(),
            theme_state,
            sheet_state,
            sheet_view,
        }
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
        let preference = self.theme_state.read(cx).preference();

        div()
            .id("settings-view")
            .track_focus(&self.focus_handle)
            .size_full()
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
                    .border_b_1()
                    .border_color(rgb(theme::border_subtle(cx)))
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
                    .overflow_scroll()
                    .flex_1()
                    .px(px(theme::SPACING_LG))
                    .pb(px(bottom_inset + 18.0))
                    .child(
                        div()
                            .w_full()
                            .max_w(px(520.0))
                            .mx_auto()
                            .flex()
                            .flex_col()
                            .gap(px(theme::SPACING_MD))
                            .child(section_header(cx, "Appearance"))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(8.0))
                                    .child(
                                        div()
                                            .text_color(rgb(theme::text_secondary(cx)))
                                            .text_size(px(theme::FONT_BODY))
                                            .font_family(fonts::MONO_FONT_FAMILY)
                                            .child("Theme"),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .flex_row()
                                            .gap(px(8.0))
                                            .child(theme_option_button(
                                                cx,
                                                "settings-theme-dark",
                                                ThemePreference::Dark,
                                                preference == ThemePreference::Dark,
                                                cx.listener(|this, _event, _window, cx| {
                                                    this.set_theme_preference(
                                                        ThemePreference::Dark,
                                                        cx,
                                                    );
                                                }),
                                            ))
                                            .child(theme_option_button(
                                                cx,
                                                "settings-theme-light",
                                                ThemePreference::Light,
                                                preference == ThemePreference::Light,
                                                cx.listener(|this, _event, _window, cx| {
                                                    this.set_theme_preference(
                                                        ThemePreference::Light,
                                                        cx,
                                                    );
                                                }),
                                            )),
                                    ),
                            )
                            .child(
                                div()
                                    .text_color(rgb(theme::text_muted(cx)))
                                    .text_size(px(theme::FONT_DETAIL))
                                    .font_family(fonts::MONO_FONT_FAMILY)
                                    .child("Custom themes will be supported in a future update."),
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

fn theme_option_button(
    cx: &App,
    id: &'static str,
    preference: ThemePreference,
    selected: bool,
    on_press: impl Fn(&PressEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex_1()
        .h(px(36.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(8.0))
        .border_1()
        .border_color(rgb(if selected {
            theme::border_highlight(cx)
        } else {
            theme::border_default(cx)
        }))
        .bg(rgb(if selected {
            theme::bg_card(cx)
        } else {
            theme::bg_primary(cx)
        }))
        .cursor_pointer()
        .text_color(rgb(if selected {
            theme::text_primary(cx)
        } else {
            theme::text_secondary(cx)
        }))
        .text_size(px(theme::FONT_BODY))
        .font_family(fonts::MONO_FONT_FAMILY)
        .on_press(on_press)
        .child(preference.label())
}

fn action_row(
    cx: &App,
    id: &'static str,
    title: &'static str,
    description: &'static str,
) -> Stateful<Div> {
    div()
        .id(id)
        .w_full()
        .min_w_0()
        .min_h(px(56.0))
        .py(px(10.0))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
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
