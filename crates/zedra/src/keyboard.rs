/// Keyboard show/hide handler factory.
///
/// Creates the closure used by `TerminalView::set_keyboard_request()`.
/// Deduplicates the identical closure that was previously inlined 3 times.

pub fn make_keyboard_handler() -> Box<dyn Fn(bool) + Send> {
    Box::new(|show| {
        if show && crate::mgpui::is_drawer_overlay_visible() {
            return;
        }
        let bridge = crate::platform_bridge::bridge();
        if show {
            bridge.show_keyboard();
        } else {
            bridge.hide_keyboard();
        }
    })
}
