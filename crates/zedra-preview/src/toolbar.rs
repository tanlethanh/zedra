//! Toolbar for device selection, theme toggle, and preview controls

use crate::device::Device;
use gpui::{
    div, prelude::*, px, rgb, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, ViewContext,
};
use std::sync::Arc;

/// Toolbar state
pub struct Toolbar {
    /// Currently selected device
    device: Device,
    /// Dark mode enabled
    dark_mode: bool,
    /// Show device frame
    show_frame: bool,
    /// Zoom level (1.0 = 100%)
    zoom: f32,
    /// Device dropdown open
    device_dropdown_open: bool,
    /// Callbacks
    on_device_change: Option<Arc<dyn Fn(Device, &mut ViewContext<Self>) + Send + Sync>>,
    on_dark_mode_toggle: Option<Arc<dyn Fn(bool, &mut ViewContext<Self>) + Send + Sync>>,
    on_frame_toggle: Option<Arc<dyn Fn(bool, &mut ViewContext<Self>) + Send + Sync>>,
    on_zoom_change: Option<Arc<dyn Fn(f32, &mut ViewContext<Self>) + Send + Sync>>,
}

impl Toolbar {
    pub fn new() -> Self {
        Self {
            device: Device::default(),
            dark_mode: false,
            show_frame: true,
            zoom: 1.0,
            device_dropdown_open: false,
            on_device_change: None,
            on_dark_mode_toggle: None,
            on_frame_toggle: None,
            on_zoom_change: None,
        }
    }

    /// Set the current device
    pub fn device(mut self, device: Device) -> Self {
        self.device = device;
        self
    }

    /// Set dark mode
    pub fn dark_mode(mut self, enabled: bool) -> Self {
        self.dark_mode = enabled;
        self
    }

    /// Set show frame
    pub fn show_frame(mut self, show: bool) -> Self {
        self.show_frame = show;
        self
    }

    /// Set zoom level
    pub fn zoom(mut self, zoom: f32) -> Self {
        self.zoom = zoom;
        self
    }

    /// Set device change callback
    pub fn on_device_change(
        mut self,
        callback: impl Fn(Device, &mut ViewContext<Self>) + Send + Sync + 'static,
    ) -> Self {
        self.on_device_change = Some(Arc::new(callback));
        self
    }

    /// Set dark mode toggle callback
    pub fn on_dark_mode_toggle(
        mut self,
        callback: impl Fn(bool, &mut ViewContext<Self>) + Send + Sync + 'static,
    ) -> Self {
        self.on_dark_mode_toggle = Some(Arc::new(callback));
        self
    }

    /// Update device
    pub fn set_device(&mut self, device: Device) {
        self.device = device;
    }

    /// Toggle dark mode
    pub fn toggle_dark_mode(&mut self) {
        self.dark_mode = !self.dark_mode;
    }

    /// Toggle frame visibility
    pub fn toggle_frame(&mut self) {
        self.show_frame = !self.show_frame;
    }

    /// Set zoom level
    pub fn set_zoom(&mut self, zoom: f32) {
        self.zoom = zoom.clamp(0.25, 2.0);
    }

    /// Get current device
    pub fn current_device(&self) -> Device {
        self.device
    }

    /// Get dark mode state
    pub fn is_dark_mode(&self) -> bool {
        self.dark_mode
    }

    /// Get frame visibility
    pub fn shows_frame(&self) -> bool {
        self.show_frame
    }

    /// Get current zoom
    pub fn current_zoom(&self) -> f32 {
        self.zoom
    }
}

impl Default for Toolbar {
    fn default() -> Self {
        Self::new()
    }
}

impl Render for Toolbar {
    fn render(&mut self, cx: &mut ViewContext<Self>) -> impl IntoElement {
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
                // Left side - Device selector
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .id("device-selector")
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .px(px(8.0))
                            .py(px(4.0))
                            .bg(rgb(0x3c3c3c))
                            .rounded(px(4.0))
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(0x4c4c4c)))
                            .on_click(|_, cx| {
                                cx.notify();
                            })
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0xcccccc))
                                    .child(self.device.name()),
                            )
                            .child(div().text_xs().text_color(rgb(0x888888)).child("▼")),
                    )
                    .child(
                        // Device dimensions display
                        div().text_xs().text_color(rgb(0x888888)).child({
                            let size = self.device.size();
                            format!("{}×{}", size.width.0 as u32, size.height.0 as u32)
                        }),
                    ),
            )
            .child(
                // Center - Zoom controls
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(
                        div()
                            .id("zoom-out")
                            .w(px(24.0))
                            .h(px(24.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.0))
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(0x3c3c3c)))
                            .on_click(|_, cx| {
                                cx.notify();
                            })
                            .child(div().text_sm().text_color(rgb(0xcccccc)).child("−")),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0xcccccc))
                            .child(format!("{}%", (self.zoom * 100.0) as u32)),
                    )
                    .child(
                        div()
                            .id("zoom-in")
                            .w(px(24.0))
                            .h(px(24.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.0))
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(0x3c3c3c)))
                            .on_click(|_, cx| {
                                cx.notify();
                            })
                            .child(div().text_sm().text_color(rgb(0xcccccc)).child("+")),
                    ),
            )
            .child(
                // Right side - Toggle buttons
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        // Dark mode toggle
                        div()
                            .id("dark-mode-toggle")
                            .px(px(8.0))
                            .py(px(4.0))
                            .rounded(px(4.0))
                            .cursor_pointer()
                            .when(self.dark_mode, |s| s.bg(rgb(0x094771)))
                            .hover(|s| {
                                if self.dark_mode {
                                    s
                                } else {
                                    s.bg(rgb(0x3c3c3c))
                                }
                            })
                            .on_click(|_, cx| {
                                cx.notify();
                            })
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(if self.dark_mode {
                                        rgb(0xffffff)
                                    } else {
                                        rgb(0xcccccc)
                                    })
                                    .child(if self.dark_mode { "🌙" } else { "☀️" }),
                            ),
                    )
                    .child(
                        // Frame toggle
                        div()
                            .id("frame-toggle")
                            .px(px(8.0))
                            .py(px(4.0))
                            .rounded(px(4.0))
                            .cursor_pointer()
                            .when(self.show_frame, |s| s.bg(rgb(0x094771)))
                            .hover(|s| {
                                if self.show_frame {
                                    s
                                } else {
                                    s.bg(rgb(0x3c3c3c))
                                }
                            })
                            .on_click(|_, cx| {
                                cx.notify();
                            })
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(if self.show_frame {
                                        rgb(0xffffff)
                                    } else {
                                        rgb(0xcccccc)
                                    })
                                    .child("Frame"),
                            ),
                    ),
            )
    }
}
