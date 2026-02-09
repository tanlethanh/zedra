//! Sidebar component showing list of previews organized by category

use crate::preview::Preview;
use gpui::{
    div, prelude::*, px, rgb, AnyElement, Element, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, View, ViewContext, VisualContext,
    WindowContext,
};
use std::sync::Arc;

/// Sidebar state
pub struct Sidebar {
    /// All registered previews
    previews: Vec<Arc<Preview>>,
    /// Currently selected preview index
    selected: Option<usize>,
    /// Search filter
    search_query: String,
    /// Collapsed categories
    collapsed_categories: Vec<String>,
    /// Callback when preview is selected
    on_select: Option<Arc<dyn Fn(usize, &mut WindowContext) + Send + Sync>>,
}

impl Sidebar {
    pub fn new(previews: Vec<Arc<Preview>>) -> Self {
        Self {
            previews,
            selected: None,
            search_query: String::new(),
            collapsed_categories: Vec::new(),
            on_select: None,
        }
    }

    /// Set the selection callback
    pub fn on_select(
        mut self,
        callback: impl Fn(usize, &mut WindowContext) + Send + Sync + 'static,
    ) -> Self {
        self.on_select = Some(Arc::new(callback));
        self
    }

    /// Set the selected preview
    pub fn set_selected(&mut self, index: Option<usize>) {
        self.selected = index;
    }

    /// Get previews grouped by category
    fn grouped_previews(&self) -> Vec<(String, Vec<(usize, &Arc<Preview>)>)> {
        let mut groups: Vec<(String, Vec<(usize, &Arc<Preview>)>)> = Vec::new();

        for (idx, preview) in self.previews.iter().enumerate() {
            // Filter by search query
            if !self.search_query.is_empty() {
                let query = self.search_query.to_lowercase();
                if !preview.name.to_lowercase().contains(&query)
                    && !preview.category.to_lowercase().contains(&query)
                {
                    continue;
                }
            }

            // Find or create category group
            if let Some(group) = groups.iter_mut().find(|(cat, _)| cat == &preview.category) {
                group.1.push((idx, preview));
            } else {
                groups.push((preview.category.clone(), vec![(idx, preview)]));
            }
        }

        // Sort categories alphabetically
        groups.sort_by(|a, b| a.0.cmp(&b.0));
        groups
    }

    /// Toggle category collapsed state
    fn toggle_category(&mut self, category: &str) {
        if let Some(pos) = self.collapsed_categories.iter().position(|c| c == category) {
            self.collapsed_categories.remove(pos);
        } else {
            self.collapsed_categories.push(category.to_string());
        }
    }

    /// Check if category is collapsed
    fn is_collapsed(&self, category: &str) -> bool {
        self.collapsed_categories.contains(&category.to_string())
    }
}

impl Render for Sidebar {
    fn render(&mut self, cx: &mut ViewContext<Self>) -> impl IntoElement {
        let groups = self.grouped_previews();

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
                // Search (placeholder - would need text input component)
                div().px(px(8.0)).py(px(8.0)).child(
                    div()
                        .px(px(8.0))
                        .py(px(6.0))
                        .bg(rgb(0x2a2a2a))
                        .rounded(px(4.0))
                        .text_sm()
                        .text_color(rgb(0x888888))
                        .child("Search..."),
                ),
            )
            .child(
                // Preview list
                div()
                    .flex_1()
                    .overflow_y_scroll()
                    .children(groups.into_iter().map(|(category, previews)| {
                        let is_collapsed = self.is_collapsed(&category);
                        let category_clone = category.clone();

                        div()
                            .flex()
                            .flex_col()
                            .child(
                                // Category header
                                div()
                                    .id(SharedString::from(format!("cat-{}", category)))
                                    .px(px(12.0))
                                    .py(px(6.0))
                                    .cursor_pointer()
                                    .hover(|s| s.bg(rgb(0x2a2a2a)))
                                    .on_click({
                                        let cat = category_clone.clone();
                                        move |_, cx| {
                                            cx.notify();
                                        }
                                    })
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(4.0))
                                            .child(
                                                div().text_xs().text_color(rgb(0x888888)).child(
                                                    if is_collapsed { "▶" } else { "▼" },
                                                ),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                                    .text_color(rgb(0x888888))
                                                    .child(category.clone()),
                                            ),
                                    ),
                            )
                            .when(!is_collapsed, |el| {
                                el.children(previews.into_iter().map(|(idx, preview)| {
                                    let is_selected = self.selected == Some(idx);
                                    let on_select = self.on_select.clone();

                                    div()
                                        .id(SharedString::from(format!("preview-{}", idx)))
                                        .pl(px(24.0))
                                        .pr(px(12.0))
                                        .py(px(4.0))
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
                                            if let Some(callback) = &on_select {
                                                callback(idx, cx);
                                            }
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
                                }))
                            })
                    })),
            )
    }
}
