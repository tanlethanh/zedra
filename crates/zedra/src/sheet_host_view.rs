use gpui::*;

pub struct SheetHostView {
    content: AnyView,
}

impl SheetHostView {
    pub fn new(content: AnyView, _cx: &mut Context<Self>) -> Self {
        Self { content }
    }

    pub fn set_content(&mut self, content: AnyView, cx: &mut Context<Self>) {
        if self.content != content {
            self.content = content;
            cx.notify();
        }
    }
}

impl Render for SheetHostView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(div().flex_1().min_h_0().child(self.content.clone()))
    }
}
