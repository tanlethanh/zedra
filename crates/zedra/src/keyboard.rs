use crate::{mgpui, platform_bridge};

/// Keyboard show/hide handler factory.
///
/// Creates the closure used by `TerminalView::set_keyboard_request()`.
pub fn make_keyboard_handler() -> Box<dyn Fn(bool) + Send> {
    Box::new(|show| {
        if show && mgpui::is_drawer_overlay_visible() {
            return;
        }
        let bridge = platform_bridge::bridge();
        if show {
            bridge.show_keyboard();
        } else {
            bridge.hide_keyboard();
        }
    })
}

/// Keyboard visibility query factory.
///
/// Creates the closure used by `TerminalView::set_is_keyboard_visible_fn()`.
/// Reads actual platform state so tap-to-toggle stays in sync after external
/// keyboard dismissals (e.g. opening the drawer, tapping the quick-action button).
pub fn make_is_keyboard_visible() -> Box<dyn Fn() -> bool + Send> {
    Box::new(|| platform_bridge::bridge().is_keyboard_visible())
}
