use gpui::*;

use crate::platform_bridge::{self, HapticFeedback};
use crate::theme;

/// Back control for subscreen headers (manage agents, agent history, etc.).
pub fn chevron_back_button<C>(
    id: impl Into<ElementId>,
    cx: &mut Context<C>,
    on_press: impl Fn(&mut C, &PressEvent, &mut Window, &mut Context<C>) + 'static,
) -> Stateful<Div>
where
    C: 'static,
{
    div()
        .id(id)
        .flex_shrink_0()
        .cursor_pointer()
        .hit_slop(px(32.0))
        .on_press(cx.listener(on_press))
        .child(
            svg()
                .path("icons/chevron-left.svg")
                .size(px(theme::ICON_MD))
                .text_color(rgb(theme::text_muted(cx))),
        )
}

pub fn subscreen_refresh_button<C>(
    id: impl Into<ElementId>,
    cx: &mut Context<C>,
    on_press: impl Fn(&mut C, &PressEvent, &mut Window, &mut Context<C>) + 'static,
) -> Stateful<Div>
where
    C: 'static,
{
    div()
        .id(id)
        .absolute()
        .top_2()
        .right_0()
        .cursor_pointer()
        .hit_slop(px(28.0))
        .on_press(cx.listener(move |this, event, window, cx| {
            platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
            on_press(this, event, window, cx);
        }))
        .child(
            svg()
                .path("icons/refresh-ccw.svg")
                .size(px(14.0))
                .text_color(rgb(theme::text_muted(cx))),
        )
}
