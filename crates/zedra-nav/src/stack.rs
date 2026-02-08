use gpui::prelude::FluentBuilder;
use gpui::*;

struct StackEntry {
    view: AnyView,
    title: SharedString,
}

pub struct HeaderConfig {
    pub height: f32,
    pub bg_color: Hsla,
    pub title_color: Hsla,
    pub back_color: Hsla,
    pub show_header: bool,
}

impl Default for HeaderConfig {
    fn default() -> Self {
        Self {
            height: 44.0,
            bg_color: hsla(220.0 / 360.0, 0.13, 0.14, 1.0),   // #21252b
            title_color: hsla(207.0 / 360.0, 0.82, 0.66, 1.0), // #61afef
            back_color: hsla(220.0 / 360.0, 0.14, 0.71, 1.0),  // #abb2bf
            show_header: true,
        }
    }
}

#[derive(Clone, Debug)]
pub enum StackEvent {
    Pushed,
    Popped,
}

pub struct StackNavigator {
    stack: Vec<StackEntry>,
    focus_handle: FocusHandle,
    header_config: HeaderConfig,
}

impl StackNavigator {
    pub fn new(header_config: HeaderConfig, cx: &mut Context<Self>) -> Self {
        Self {
            stack: Vec::new(),
            focus_handle: cx.focus_handle(),
            header_config,
        }
    }

    pub fn push(&mut self, view: AnyView, title: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.stack.push(StackEntry {
            view,
            title: title.into(),
        });
        cx.emit(StackEvent::Pushed);
        cx.notify();
    }

    pub fn pop(&mut self, cx: &mut Context<Self>) -> Option<AnyView> {
        if self.stack.len() <= 1 {
            return None;
        }
        let entry = self.stack.pop();
        cx.emit(StackEvent::Popped);
        cx.notify();
        entry.map(|e| e.view)
    }

    pub fn can_pop(&self) -> bool {
        self.stack.len() > 1
    }

    pub fn pop_to_root(&mut self, cx: &mut Context<Self>) {
        if self.stack.len() > 1 {
            self.stack.truncate(1);
            cx.emit(StackEvent::Popped);
            cx.notify();
        }
    }

    pub fn replace(
        &mut self,
        view: AnyView,
        title: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.stack.pop();
        self.stack.push(StackEntry {
            view,
            title: title.into(),
        });
        cx.notify();
    }

    pub fn stack_depth(&self) -> usize {
        self.stack.len()
    }

    pub fn current_title(&self) -> Option<&SharedString> {
        self.stack.last().map(|e| &e.title)
    }
}

impl EventEmitter<StackEvent> for StackNavigator {}

impl Focusable for StackNavigator {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for StackNavigator {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = self.stack.last().map(|e| e.view.clone());
        let show_header = self.header_config.show_header && !self.stack.is_empty();
        let show_back = self.can_pop();
        let title = self
            .stack
            .last()
            .map(|e| e.title.clone())
            .unwrap_or_default();

        let header_height = self.header_config.height;
        let header_bg = self.header_config.bg_color;
        let title_color = self.header_config.title_color;
        let back_color = self.header_config.back_color;

        div()
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .size_full()
            .when(show_header, |d| {
                d.child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .h(px(header_height))
                        .px_2()
                        .bg(header_bg)
                        .border_b_1()
                        .border_color(hsla(220.0 / 360.0, 0.14, 0.27, 1.0)) // #3e4451
                        .when(show_back, |d| {
                            d.child(
                                div()
                                    .px_2()
                                    .py_1()
                                    .rounded(px(4.0))
                                    .text_color(back_color)
                                    .cursor_pointer()
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _event, _window, cx| {
                                            this.pop(cx);
                                        }),
                                    )
                                    .child("< Back"),
                            )
                        })
                        .child(
                            div()
                                .flex_1()
                                .text_color(title_color)
                                .text_sm()
                                .child(title),
                        ),
                )
            })
            .when_some(content, |d, view| d.child(div().flex_1().child(view)))
    }
}
