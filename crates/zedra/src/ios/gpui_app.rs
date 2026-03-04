/// GPUI-based iOS app — renders via Metal through gpui_ios.
///
/// Lifecycle (called from Obj-C app delegate in main.m):
///   1. gpui_ios_initialize()         — set up GPUI FFI state
///   2. zedra_launch_gpui()           — create AppCell + register window callback
///   3. gpui_ios_did_finish_launching — invokes callback → opens Metal window
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

    crate::platform_bridge::set_bridge(super::bridge::IosBridge);

    log::info!("Zedra iOS: Creating GPUI application with IosPlatform");

    let platform: Rc<dyn Platform> = Rc::new(IosPlatform::new());

    let app_cell = App::new_app(
        platform.clone(),
        Arc::new(crate::ZedraAssets),
        Arc::new(http_client::BlockedHttpClient),
    );

    // Register the finish-launching callback via platform.run().
    // On iOS this does NOT block — it stores the callback in the FFI layer.
    // When main.m calls gpui_ios_did_finish_launching(), the callback fires
    // and opens the Metal window with ZedraApp.
    let app_cell_for_callback = app_cell.clone();
    platform.run(Box::new(move || {
        log::info!("Zedra iOS: finish-launching callback — opening window");
        let cx = &mut *app_cell_for_callback.borrow_mut();

        let window_options = WindowOptions {
            focus: true,
            show: true,
            ..Default::default()
        };

        let result: Result<AnyWindowHandle, _> = if cfg!(feature = "preview") {
            cx.open_window(window_options, |window, cx| {
                let view = cx.new(|cx| crate::app_preview::PreviewApp::new(window, cx));
                window.refresh();
                view
            })
            .map(|h| h.into())
        } else {
            cx.open_window(window_options, |window, cx| {
                let view = cx.new(|cx| ZedraApp::new(window, cx));
                window.refresh();
                view
            })
            .map(|h| h.into())
        };

        match result {
            Ok(handle) => log::info!("Zedra iOS: Window opened: {:?}", handle),
            Err(err) => log::error!("Zedra iOS: Failed to open window: {:?}", err),
        }
    }));

    log::info!("Zedra iOS: Callback registered, waiting for didFinishLaunching");

    // Keep the AppCell alive — UIKit owns the run loop on iOS.
    std::mem::forget(app_cell);
}
