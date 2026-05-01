use gpui::{Refineable as _, *};

pub use crate::platform_bridge::NativeFloatingButtonIconWeight;
use crate::platform_bridge::{self, NativeFloatingButtonOptions};
use crate::theme;

const DEFAULT_NATIVE_FLOATING_BUTTON_ICON_SIZE: f32 = 16.0;

/// An outlined button - bordered, centered text.
pub fn outline_button(id: impl Into<ElementId>, label: &str) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .justify_center()
        .py(px(10.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(rgb(theme::BORDER_DEFAULT))
        .cursor_pointer()
        .text_color(rgb(theme::TEXT_PRIMARY))
        .text_size(px(theme::FONT_BODY))
        .font_weight(FontWeight::MEDIUM)
        .child(label.to_string())
}

struct NativeFloatingButtonState {
    native_id: NativeFloatingButtonId,
}

impl Drop for NativeFloatingButtonState {
    fn drop(&mut self) {
        hide_native_floating_button(self.native_id);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeFloatingButtonId(u32);

pub fn native_floating_button_id() -> NativeFloatingButtonId {
    NativeFloatingButtonId(platform_bridge::allocate_native_floating_button_id())
}

pub fn hide_native_floating_button(id: NativeFloatingButtonId) {
    platform_bridge::remove_native_floating_button(id.0);
}

pub struct NativeFloatingButton {
    id: ElementId,
    native_id: NativeFloatingButtonId,
    system_image_name: SharedString,
    accessibility_label: SharedString,
    icon_size_pts: f32,
    icon_weight: NativeFloatingButtonIconWeight,
    on_press: Option<Box<dyn FnMut(&mut App)>>,
    style: StyleRefinement,
}

pub fn native_floating_button(
    id: impl Into<ElementId>,
    native_id: NativeFloatingButtonId,
    system_image_name: impl Into<SharedString>,
    accessibility_label: impl Into<SharedString>,
    on_press: impl FnMut(&mut App) + 'static,
) -> NativeFloatingButton {
    NativeFloatingButton {
        id: id.into(),
        native_id,
        system_image_name: system_image_name.into(),
        accessibility_label: accessibility_label.into(),
        icon_size_pts: DEFAULT_NATIVE_FLOATING_BUTTON_ICON_SIZE,
        icon_weight: NativeFloatingButtonIconWeight::default(),
        on_press: Some(Box::new(on_press)),
        style: StyleRefinement::default(),
    }
}

impl NativeFloatingButton {
    pub fn icon_size(mut self, size_pts: f32) -> Self {
        self.icon_size_pts = size_pts;
        self
    }

    pub fn icon_weight(mut self, weight: NativeFloatingButtonIconWeight) -> Self {
        self.icon_weight = weight;
        self
    }
}

impl Element for NativeFloatingButton {
    type RequestLayoutState = Style;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.refine(&self.style);
        let layout_id = window.request_layout(style.clone(), [], cx);
        (layout_id, style)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let Some(id) = id else {
            return;
        };

        let native_id =
            window.with_element_state(id, |state: Option<NativeFloatingButtonState>, _window| {
                let state = match state {
                    Some(mut state) => {
                        if state.native_id != self.native_id {
                            hide_native_floating_button(state.native_id);
                            state.native_id = self.native_id;
                        }
                        state
                    }
                    None => NativeFloatingButtonState {
                        native_id: self.native_id,
                    },
                };
                (state.native_id, state)
            });

        if let Some(on_press) = self.on_press.take() {
            platform_bridge::set_native_floating_button_callback(native_id.0, on_press);
        }

        let options = NativeFloatingButtonOptions {
            system_image_name: self.system_image_name.to_string(),
            accessibility_label: self.accessibility_label.to_string(),
            bounds,
            icon_size_pts: self.icon_size_pts,
            icon_weight: self.icon_weight,
        };
        cx.defer(move |_| {
            platform_bridge::update_native_floating_button(native_id.0, options);
        });
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }
}

impl Styled for NativeFloatingButton {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl IntoElement for NativeFloatingButton {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}
