use gpui::prelude::FluentBuilder;
use gpui::*;

#[derive(Clone, Debug)]
pub enum ModalEvent {
    Presented,
    Dismissed,
    BackdropTapped,
}

pub struct ModalHost {
    content: AnyView,
    modal: Option<AnyView>,
    focus_handle: FocusHandle,
    backdrop_opacity: f32,
}

impl ModalHost {
    pub fn new(content: AnyView, cx: &mut Context<Self>) -> Self {
        Self {
            content,
            modal: None,
            focus_handle: cx.focus_handle(),
            backdrop_opacity: 0.5,
        }
    }

    pub fn present(&mut self, view: AnyView, cx: &mut Context<Self>) {
        self.modal = Some(view);
        cx.emit(ModalEvent::Presented);
        cx.notify();
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.modal = None;
        cx.emit(ModalEvent::Dismissed);
        cx.notify();
    }

    pub fn is_presented(&self) -> bool {
        self.modal.is_some()
    }

    pub fn set_backdrop_opacity(&mut self, opacity: f32) {
        self.backdrop_opacity = opacity;
    }
}

impl EventEmitter<ModalEvent> for ModalHost {}

impl Focusable for ModalHost {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ModalHost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = self.content.clone();
        let modal = self.modal.clone();
        let backdrop_opacity = self.backdrop_opacity;

        div()
            .track_focus(&self.focus_handle)
            .size_full()
            .child(content)
            .when_some(modal, |d, modal_view| {
                d.child(
                    deferred(
                        div()
                            .size_full()
                            // Backdrop
                            .child(
                                div()
                                    .absolute()
                                    .size_full()
                                    .bg(hsla(0.0, 0.0, 0.0, backdrop_opacity))
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _event, _window, cx| {
                                            cx.emit(ModalEvent::BackdropTapped);
                                            this.dismiss(cx);
                                        }),
                                    ),
                            )
                            // Modal content centered
                            .child(
                                div()
                                    .absolute()
                                    .size_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(modal_view),
                            ),
                    )
                    .with_priority(999),
                )
            })
    }
}
