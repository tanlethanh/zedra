//! Main PreviewApp application for component preview

use crate::device::Device;
use crate::preview::Preview;
use crate::sidebar::Sidebar;
use crate::toolbar::Toolbar;
use gpui::{
    div, prelude::*, px, rgb, AnyElement, App, AppContext, Element, Hsla, IntoElement,
    ParentElement, Render, SharedString, Styled, View, ViewContext, VisualContext, WindowContext,
    WindowOptions,
};
use std::sync::Arc;

/// Main preview application
pub struct PreviewApp {
    /// Registered previews
    previews: Vec<Arc<Preview>>,
    /// Currently selected preview index
    selected_preview: Option<usize>,
    /// Currently selected variant index
    selected_variant: usize,
    /// Current device for preview
    device: Device,
    /// Dark mode enabled
    dark_mode: bool,
    /// Show device frame
    show_frame: bool,
    /// Zoom level
    zoom: f32,
}

impl PreviewApp {
    /// Create a new preview app
    pub fn new() -> Self {
        Self {
            previews: Vec::new(),
            selected_preview: None,
            selected_variant: 0,
            device: Device::default(),
            dark_mode: false,
            show_frame: true,
            zoom: 1.0,
        }
    }

    /// Register a preview
    pub fn register(mut self, preview: Preview) -> Self {
        self.previews.push(Arc::new(preview));
        self
    }

    /// Register multiple previews
    pub fn register_all(mut self, previews: impl IntoIterator<Item = Preview>) -> Self {
        for preview in previews {
            self.previews.push(Arc::new(preview));
        }
        self
    }

    /// Set default device
    pub fn default_device(mut self, device: Device) -> Self {
        self.device = device;
        self
    }

    /// Set default dark mode
    pub fn default_dark_mode(mut self, enabled: bool) -> Self {
        self.dark_mode = enabled;
        self
    }

    /// Run the preview application
    pub fn run(self) {
        App::new().run(move |cx| {
            let app = self;
            cx.open_window(
                WindowOptions {
                    ..Default::default()
                },
                |cx| cx.new_view(|_| app),
            )
            .expect("Failed to open preview window");
        });
    }

    /// Select a preview by index
    fn select_preview(&mut self, index: usize) {
        if index < self.previews.len() {
            self.selected_preview = Some(index);
            self.selected_variant = 0;

            // Apply preview defaults
            let preview = &self.previews[index];
            self.device = preview.device;
            self.dark_mode = preview.dark_mode;
            self.show_frame = preview.show_frame;
        }
    }

    /// Select a variant by index
    fn select_variant(&mut self, index: usize) {
        if let Some(preview_idx) = self.selected_preview {
            if let Some(preview) = self.previews.get(preview_idx) {
                if index < preview.variants.len() {
                    self.selected_variant = index;
                }
            }
        }
    }

    /// Get the current preview
    fn current_preview(&self) -> Option<&Arc<Preview>> {
        self.selected_preview.and_then(|idx| self.previews.get(idx))
    }

    /// Render the preview content
    fn render_preview_content(&self) -> Option<AnyElement> {
        let preview = self.current_preview()?;
        let variant = preview.variants.get(self.selected_variant)?;
        Some((variant.render)())
    }

    /// Render the device frame
    fn render_device_frame(&self, content: AnyElement) -> impl IntoElement {
        let size = self.device.size();
        let safe_area = self.device.safe_area();
        let scaled_width = size.width.0 * self.zoom;
        let scaled_height = size.height.0 * self.zoom;

        div()
            .flex()
            .flex_col()
            .items_center()
            .child(
                // Device frame container
                div()
                    .when(self.show_frame, |el| {
                        el.p(px(12.0))
                            .bg(rgb(0x1a1a1a))
                            .rounded(px(40.0))
                            .shadow_lg()
                    })
                    .child(
                        // Screen area
                        div()
                            .w(px(scaled_width))
                            .h(px(scaled_height))
                            .overflow_hidden()
                            .when(self.show_frame, |el| el.rounded(px(32.0)))
                            .bg(if self.dark_mode {
                                rgb(0x1e1e1e)
                            } else {
                                rgb(0xffffff)
                            })
                            .child(content),
                    ),
            )
            .child(
                // Device name label
                div()
                    .mt(px(8.0))
                    .text_xs()
                    .text_color(rgb(0x888888))
                    .child(self.device.name()),
            )
    }

    /// Render variant tabs
    fn render_variant_tabs(&self, cx: &mut ViewContext<Self>) -> impl IntoElement {
        let Some(preview) = self.current_preview() else {
            return div();
        };

        if preview.variants.len() <= 1 {
            return div();
        }

        div()
            .flex()
            .items_center()
            .gap(px(4.0))
            .px(px(12.0))
            .py(px(8.0))
            .border_b_1()
            .border_color(rgb(0x333333))
            .children(preview.variants.iter().enumerate().map(|(idx, variant)| {
                let is_selected = idx == self.selected_variant;

                div()
                    .id(SharedString::from(format!("variant-{}", idx)))
                    .px(px(12.0))
                    .py(px(4.0))
                    .rounded(px(4.0))
                    .cursor_pointer()
                    .when(is_selected, |s| s.bg(rgb(0x094771)))
                    .hover(|s| if is_selected { s } else { s.bg(rgb(0x3c3c3c)) })
                    .on_click(move |_, cx| {
                        cx.notify();
                    })
                    .child(
                        div()
                            .text_sm()
                            .text_color(if is_selected {
                                rgb(0xffffff)
                            } else {
                                rgb(0xcccccc)
                            })
                            .child(variant.name.clone()),
                    )
            }))
    }

    /// Render the controls panel
    fn render_controls(&self, cx: &mut ViewContext<Self>) -> impl IntoElement {
        let Some(preview) = self.current_preview() else {
            return div();
        };

        if preview.controls.is_empty() {
            return div();
        }

        div()
            .flex()
            .flex_col()
            .w(px(240.0))
            .h_full()
            .bg(rgb(0x1e1e1e))
            .border_l_1()
            .border_color(rgb(0x333333))
            .child(
                div()
                    .px(px(12.0))
                    .py(px(8.0))
                    .border_b_1()
                    .border_color(rgb(0x333333))
                    .child(div().text_sm().text_color(rgb(0xcccccc)).child("Controls")),
            )
            .child(div().flex_1().overflow_y_scroll().p(px(12.0)).children(
                preview.controls.iter().map(|control| {
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .mb(px(12.0))
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(0x888888))
                                .child(control.name.clone()),
                        )
                        .child(self.render_control_input(control))
                }),
            ))
    }

    /// Render a control input based on its type
    fn render_control_input(&self, control: &crate::preview::PropControl) -> impl IntoElement {
        use crate::preview::PropValue;

        match &control.value {
            PropValue::Bool(value) => div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(
                    div()
                        .w(px(36.0))
                        .h(px(20.0))
                        .rounded(px(10.0))
                        .bg(if *value { rgb(0x0078d4) } else { rgb(0x3c3c3c) })
                        .cursor_pointer()
                        .child(
                            div()
                                .w(px(16.0))
                                .h(px(16.0))
                                .mt(px(2.0))
                                .ml(if *value { px(18.0) } else { px(2.0) })
                                .rounded_full()
                                .bg(rgb(0xffffff)),
                        ),
                )
                .child(div().text_sm().text_color(rgb(0xcccccc)).child(if *value {
                    "On"
                } else {
                    "Off"
                }))
                .into_any_element(),

            PropValue::String(value) => div()
                .px(px(8.0))
                .py(px(6.0))
                .bg(rgb(0x2a2a2a))
                .rounded(px(4.0))
                .text_sm()
                .text_color(rgb(0xcccccc))
                .child(value.clone())
                .into_any_element(),

            PropValue::Number(value) => div()
                .px(px(8.0))
                .py(px(6.0))
                .bg(rgb(0x2a2a2a))
                .rounded(px(4.0))
                .text_sm()
                .text_color(rgb(0xcccccc))
                .child(format!("{:.2}", value))
                .into_any_element(),

            PropValue::Color(color) => div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(div().w(px(24.0)).h(px(24.0)).rounded(px(4.0)).bg(*color))
                .child(div().text_sm().text_color(rgb(0xcccccc)).child(format!(
                    "hsla({:.0}, {:.0}%, {:.0}%, {:.2})",
                    color.h * 360.0,
                    color.s * 100.0,
                    color.l * 100.0,
                    color.a
                )))
                .into_any_element(),

            PropValue::Enum { selected, options } => div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .children(options.iter().map(|opt| {
                    let is_selected = opt == selected;
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .cursor_pointer()
                        .child(
                            div()
                                .w(px(16.0))
                                .h(px(16.0))
                                .rounded_full()
                                .border_2()
                                .border_color(if is_selected {
                                    rgb(0x0078d4)
                                } else {
                                    rgb(0x555555)
                                })
                                .when(is_selected, |el| {
                                    el.child(
                                        div()
                                            .w(px(8.0))
                                            .h(px(8.0))
                                            .mt(px(2.0))
                                            .ml(px(2.0))
                                            .rounded_full()
                                            .bg(rgb(0x0078d4)),
                                    )
                                }),
                        )
                        .child(div().text_sm().text_color(rgb(0xcccccc)).child(opt.clone()))
                }))
                .into_any_element(),
        }
    }
}

impl Default for PreviewApp {
    fn default() -> Self {
        Self::new()
    }
}

impl Render for PreviewApp {
    fn render(&mut self, cx: &mut ViewContext<Self>) -> impl IntoElement {
        let has_controls = self
            .current_preview()
            .map(|p| !p.controls.is_empty())
            .unwrap_or(false);

        div()
            .flex()
            .w_full()
            .h_full()
            .bg(rgb(0x1e1e1e))
            .child(
                // Sidebar
                {
                    let previews: Vec<Arc<Preview>> = self.previews.clone();
                    let mut sidebar = Sidebar::new(previews);
                    sidebar.set_selected(self.selected_preview);

                    div()
                        .flex()
                        .flex_col()
                        .w(px(240.0))
                        .h_full()
                        .bg(rgb(0x1e1e1e))
                        .border_r_1()
                        .border_color(rgb(0x333333))
                        .child(
                            // Header
                            div()
                                .px(px(12.0))
                                .py(px(8.0))
                                .border_b_1()
                                .border_color(rgb(0x333333))
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(rgb(0xcccccc))
                                        .child("Components"),
                                ),
                        )
                        .child(
                            // Preview list
                            div().flex_1().overflow_y_scroll().children(
                                self.previews.iter().enumerate().map(|(idx, preview)| {
                                    let is_selected = self.selected_preview == Some(idx);

                                    div()
                                        .id(SharedString::from(format!("preview-{}", idx)))
                                        .px(px(12.0))
                                        .py(px(6.0))
                                        .cursor_pointer()
                                        .when(is_selected, |s| s.bg(rgb(0x094771)))
                                        .hover(
                                            |s| {
                                                if is_selected {
                                                    s
                                                } else {
                                                    s.bg(rgb(0x2a2a2a))
                                                }
                                            },
                                        )
                                        .on_click(move |_, cx| {
                                            cx.notify();
                                        })
                                        .child(
                                            div()
                                                .text_sm()
                                                .text_color(if is_selected {
                                                    rgb(0xffffff)
                                                } else {
                                                    rgb(0xcccccc)
                                                })
                                                .child(preview.name.clone()),
                                        )
                                }),
                            ),
                        )
                },
            )
            .child(
                // Main content area
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .child(
                        // Toolbar
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .h(px(40.0))
                            .px(px(12.0))
                            .bg(rgb(0x252526))
                            .border_b_1()
                            .border_color(rgb(0x333333))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.0))
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(rgb(0xcccccc))
                                            .child(self.device.name()),
                                    )
                                    .child(div().text_xs().text_color(rgb(0x888888)).child({
                                        let size = self.device.size();
                                        format!("{}×{}", size.width.0 as u32, size.height.0 as u32)
                                    })),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.0))
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(rgb(0xcccccc))
                                            .child(format!("{}%", (self.zoom * 100.0) as u32)),
                                    )
                                    .child(
                                        div()
                                            .id("dark-mode-toggle")
                                            .px(px(8.0))
                                            .py(px(4.0))
                                            .rounded(px(4.0))
                                            .cursor_pointer()
                                            .when(self.dark_mode, |s| s.bg(rgb(0x094771)))
                                            .hover(|s| s.bg(rgb(0x3c3c3c)))
                                            .on_click(|_, cx| {
                                                cx.notify();
                                            })
                                            .child(
                                                div().text_sm().text_color(rgb(0xcccccc)).child(
                                                    if self.dark_mode { "Dark" } else { "Light" },
                                                ),
                                            ),
                                    ),
                            ),
                    )
                    .child(
                        // Variant tabs
                        self.render_variant_tabs(cx),
                    )
                    .child(
                        // Preview area
                        div()
                            .flex_1()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(rgb(0x2d2d2d))
                            .overflow_hidden()
                            .child(if let Some(content) = self.render_preview_content() {
                                self.render_device_frame(content).into_any_element()
                            } else {
                                div()
                                    .flex()
                                    .flex_col()
                                    .items_center()
                                    .gap(px(12.0))
                                    .child(
                                        div()
                                            .text_xl()
                                            .text_color(rgb(0x666666))
                                            .child("No Preview Selected"),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(rgb(0x555555))
                                            .child("Select a component from the sidebar"),
                                    )
                                    .into_any_element()
                            }),
                    ),
            )
            .when(has_controls, |el| el.child(self.render_controls(cx)))
    }
}
