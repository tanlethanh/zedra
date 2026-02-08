use gpui::prelude::FluentBuilder;
use gpui::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DrawerSide {
    Left,
}

impl Default for DrawerSide {
    fn default() -> Self {
        Self::Left
    }
}

#[derive(Clone, Debug)]
pub enum DrawerEvent {
    Opened,
    Closed,
    BackdropTapped,
}

pub struct DrawerHost {
    content: AnyView,
    drawer: Option<AnyView>,
    side: DrawerSide,
    width: Pixels,
    backdrop_opacity: f32,
    focus_handle: FocusHandle,
}

impl DrawerHost {
    pub fn new(content: AnyView, cx: &mut Context<Self>) -> Self {
        Self {
            content,
            drawer: None,
            side: DrawerSide::Left,
            width: px(280.0),
            backdrop_opacity: 0.4,
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn open(&mut self, view: AnyView, cx: &mut Context<Self>) {
        self.drawer = Some(view);
        cx.emit(DrawerEvent::Opened);
        cx.notify();
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.drawer = None;
        cx.emit(DrawerEvent::Closed);
        cx.notify();
    }

    pub fn is_open(&self) -> bool {
        self.drawer.is_some()
    }

    pub fn set_side(&mut self, side: DrawerSide) {
        self.side = side;
    }

    pub fn set_width(&mut self, width: Pixels) {
        self.width = width;
    }

    pub fn set_backdrop_opacity(&mut self, opacity: f32) {
        self.backdrop_opacity = opacity;
    }
}

impl EventEmitter<DrawerEvent> for DrawerHost {}

impl Focusable for DrawerHost {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for DrawerHost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = self.content.clone();
        let drawer = self.drawer.clone();
        let backdrop_opacity = self.backdrop_opacity;
        let drawer_width = self.width;

        div()
            .track_focus(&self.focus_handle)
            .size_full()
            .child(content)
            .when_some(drawer, |d, drawer_view| {
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
                                            cx.emit(DrawerEvent::BackdropTapped);
                                            this.close(cx);
                                        }),
                                    ),
                            )
                            // Drawer panel from left edge
                            .child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .left_0()
                                    .h_full()
                                    .w(drawer_width)
                                    .bg(rgb(0x21252b))
                                    .border_r_1()
                                    .border_color(rgb(0x3e4451))
                                    .child(drawer_view),
                            ),
                    )
                    .with_priority(998),
                )
            })
    }
}
