use gpui::*;

use crate::fonts;
use crate::platform_bridge::{
    self, AlertButton, CustomSheetDetent, CustomSheetOptions, HapticFeedback,
};
use crate::sheet_demo_state::SheetDemoState;
use crate::theme;

#[derive(Clone, Debug)]
pub enum SettingsEvent {
    NavigateHome,
}

impl EventEmitter<SettingsEvent> for SettingsView {}

pub struct SettingsView {
    focus_handle: FocusHandle,
    sheet_state: Entity<SheetDemoState>,
    sheet_view: Entity<crate::sheet_demo_view::SheetDemoView>,
}

impl SettingsView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let sheet_state = cx.new(|cx| SheetDemoState::new(cx));
        let sheet_view =
            cx.new(|cx| crate::sheet_demo_view::SheetDemoView::new(sheet_state.clone(), cx));
        Self {
            focus_handle: cx.focus_handle(),
            sheet_state,
            sheet_view,
        }
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

        div()
            .id("settings-view")
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(rgb(theme::BG_PRIMARY))
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
                                            .text_color(rgb(theme::TEXT_MUTED))
                                            .into_any_element()
                                    )
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap(px(8.0))
                                    .child(
                                        div()
                                            .text_color(rgb(theme::TEXT_PRIMARY))
                                            .text_size(px(theme::FONT_TITLE))
                                            .font_family(fonts::HEADING_FONT_FAMILY)
                                            .font_weight(FontWeight::MEDIUM)
                                            .child("Settings"),
                                    )
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
                            .child(
                                div()
                                    .pt(px(12.0))
                                    .pb(px(10.0))
                                    .border_b_1()
                                    .border_color(rgb(theme::BORDER_SUBTLE))
                                    .flex()
                                    .flex_col()
                                    .gap(px(4.0))
                                    .child(
                                        div()
                                            .text_color(rgb(theme::TEXT_PRIMARY))
                                            .text_size(px(theme::FONT_HEADING))
                                            .font_family(fonts::MONO_FONT_FAMILY)
                                            .font_weight(FontWeight::MEDIUM)
                                            .child("Developer"),
                                    )
                            )
                            .child(
                                action_row(
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
                                    "settings-test-selection",
                                    "Native Selection",
                                    "Action sheet selection and behavior",
                                )
                                .on_press(cx.listener(|this, _event, _window, _cx| {
                                    this.show_test_selection();
                                })),
                            ),
                    )
                    .child(
                        div()
                            .w_full()
                            .max_w(px(520.0))
                            .mx_auto()
                            .child(
                                action_row(
                                    "settings-test-custom-sheet",
                                    "Custom Sheet",
                                    "Native sheet with GPUI-rendered content",
                                )
                                .on_press(cx.listener(|this, _event, _window, cx| {
                                    this.show_test_custom_sheet(cx);
                                })),
                            ),
                    )
                    .child(
                        div()
                            .w_full()
                            .max_w(px(520.0))
                            .mx_auto()
                            .child(
                                div()
                                    .text_color(rgb(theme::TEXT_MUTED))
                                    .text_size(px(theme::FONT_DETAIL))
                                    .font_family(fonts::MONO_FONT_FAMILY)
                                    .child("QR scanner and dictation preview remain separate native flows."),
                            ),
                    ),
            )
    }
}

fn action_row(id: &'static str, title: &'static str, description: &'static str) -> Stateful<Div> {
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
                        .text_color(rgb(theme::TEXT_SECONDARY))
                        .text_size(px(theme::FONT_BODY))
                        .font_family(fonts::MONO_FONT_FAMILY)
                        .font_weight(FontWeight::MEDIUM)
                        .child(title),
                )
                .child(
                    div()
                        .text_color(rgb(theme::TEXT_MUTED))
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
                    .text_color(rgb(theme::TEXT_MUTED)),
            ),
        )
}
