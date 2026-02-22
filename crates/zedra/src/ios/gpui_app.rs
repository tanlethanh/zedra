/// GPUI-based iOS app — renders via Metal through gpui_ios.
///
/// Creates the GPUI AppCell with IosPlatform and opens a window.
/// The run loop is managed by iOS (UIApplicationMain), not by GPUI.
///
/// Lifecycle (called from Obj-C app delegate):
///   1. gpui_ios_initialize()         — set up GPUI FFI state
///   2. zedra_launch_gpui()           — create AppCell, IosPlatform runs
///   3. gpui_ios_did_finish_launching — callback opens window
///   4. gpui_ios_get_window()         — get window pointer for CADisplayLink
///   5. gpui_ios_request_frame()      — called each frame by CADisplayLink
use gpui::*;
use gpui_ios::IosPlatform;
use std::rc::Rc;
use std::sync::Arc;

use crate::app::ZedraApp;

#[unsafe(no_mangle)]
pub extern "C" fn zedra_launch_gpui() {
    oslog::OsLogger::new("dev.zedra.app")
        .level_filter(log::LevelFilter::Debug)
        .init()
        .ok();

    std::panic::set_hook(Box::new(|info| {
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "Unknown panic".to_string());

        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown".to_string());

        log::error!("PANIC at {}: {}", location, payload);
    }));

    // Register the iOS bridge for platform abstraction
    crate::platform_bridge::set_bridge(super::bridge::IosBridge);

    log::info!("Zedra iOS: Creating GPUI application with IosPlatform");

    let platform: Rc<dyn Platform> = Rc::new(IosPlatform::new());

    let app_cell = App::new_app(
        platform,
        Arc::new(crate::ZedraAssets),
        Arc::new(http_client::BlockedHttpClient),
    );

    // IosPlatform::run() is called by GPUI internals — it registers the
    // finish_launching callback. When iOS calls didFinishLaunching, GPUI
    // invokes the callback which opens our window.
    log::info!("Zedra iOS: AppCell created, waiting for didFinishLaunching");

    // Keep the AppCell alive — on iOS the run loop is owned by UIKit,
    // so we leak the Rc to prevent it from being dropped.
    std::mem::forget(app_cell);
}
