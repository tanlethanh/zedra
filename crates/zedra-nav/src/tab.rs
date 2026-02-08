use gpui::prelude::FluentBuilder;
use gpui::*;

struct TabEntry {
    label: SharedString,
    icon: SharedString,
    builder: Box<dyn Fn(&mut Window, &mut App) -> AnyView>,
    view: Option<AnyView>,
}

pub struct TabBarConfig {
    pub height: f32,
    pub bg_color: Hsla,
    pub active_color: Hsla,
    pub inactive_color: Hsla,
}

impl Default for TabBarConfig {
    fn default() -> Self {
        Self {
            height: 56.0,
            bg_color: hsla(220.0 / 360.0, 0.13, 0.14, 1.0),   // #21252b
            active_color: hsla(207.0 / 360.0, 0.82, 0.66, 1.0), // #61afef
            inactive_color: hsla(220.0 / 360.0, 0.10, 0.44, 1.0), // #5c6370
        }
    }
}

#[derive(Clone, Debug)]
pub struct TabEvent {
    pub from: usize,
    pub to: usize,
}

pub struct TabNavigator {
    tabs: Vec<TabEntry>,
    active_index: usize,
    focus_handle: FocusHandle,
    config: TabBarConfig,
}

impl TabNavigator {
    pub fn new(config: TabBarConfig, cx: &mut Context<Self>) -> Self {
        Self {
            tabs: Vec::new(),
            active_index: 0,
            focus_handle: cx.focus_handle(),
            config,
        }
    }

    pub fn add_tab(
        &mut self,
        label: impl Into<SharedString>,
        icon: impl Into<SharedString>,
        builder: impl Fn(&mut Window, &mut App) -> AnyView + 'static,
    ) {
        self.tabs.push(TabEntry {
            label: label.into(),
            icon: icon.into(),
            builder: Box::new(builder),
            view: None,
        });
    }

    pub fn set_active(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.tabs.len() || index == self.active_index {
            return;
        }
        let from = self.active_index;
        self.active_index = index;

        // Lazily create view if needed
        if self.tabs[index].view.is_none() {
            let view = (self.tabs[index].builder)(window, cx);
            self.tabs[index].view = Some(view);
        }

        cx.emit(TabEvent { from, to: index });
        cx.notify();
    }

    pub fn active_index(&self) -> usize {
        self.active_index
    }

    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    /// Ensure the active tab's view is created (call after adding all tabs).
    pub fn ensure_active_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.tabs.is_empty() && self.tabs[self.active_index].view.is_none() {
            let view = (self.tabs[self.active_index].builder)(window, cx);
            self.tabs[self.active_index].view = Some(view);
        }
    }

    /// Get the active tab's view.
    pub fn active_view(&self) -> Option<&AnyView> {
        self.tabs.get(self.active_index).and_then(|t| t.view.as_ref())
    }
}

impl EventEmitter<TabEvent> for TabNavigator {}

impl Focusable for TabNavigator {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TabNavigator {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let active_view = self
            .tabs
            .get(self.active_index)
            .and_then(|t| t.view.clone());

        let tab_bar_height = self.config.height;
        let tab_bg = self.config.bg_color;
        let active_color = self.config.active_color;
        let inactive_color = self.config.inactive_color;
        let active_index = self.active_index;

        let mut tab_bar = div()
            .flex()
            .flex_row()
            .h(px(tab_bar_height))
            .bg(tab_bg)
            .border_t_1()
            .border_color(hsla(220.0 / 360.0, 0.14, 0.27, 1.0)); // #3e4451

        for (idx, tab) in self.tabs.iter().enumerate() {
            let is_active = idx == active_index;
            let color = if is_active {
                active_color
            } else {
                inactive_color
            };
            let icon = tab.icon.clone();
            let label = tab.label.clone();

            tab_bar = tab_bar.child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _event, window, cx| {
                            this.set_active(idx, window, cx);
                        }),
                    )
                    .child(
                        div()
                            .text_color(color)
                            .text_xl()
                            .child(icon),
                    )
                    .child(
                        div()
                            .text_color(color)
                            .text_xs()
                            .child(label),
                    ),
            );
        }

        div()
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .size_full()
            .child(
                // Content area
                div()
                    .flex_1()
                    .overflow_hidden()
                    .when_some(active_view, |d, view| d.child(view)),
            )
            .child(tab_bar)
    }
}
