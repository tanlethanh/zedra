/// GPUI-based iOS app — renders via Metal through GPUI's blade renderer.
///
/// Launches the full ZedraApp (same UI as Android) with tabs, navigation,
/// terminal, editor, and transport layer.
///
/// Lifecycle (called from Obj-C app delegate):
///   1. gpui_ios_initialize()         — set up GPUI FFI state
///   2. zedra_launch_gpui()           — create Application, register window callback
///   3. gpui_ios_did_finish_launching — invoke callback -> opens window
///   4. gpui_ios_get_window()         — get window pointer for CADisplayLink
///   5. gpui_ios_request_frame()      — called each frame by CADisplayLink
use gpui::*;

use crate::zedra_app::ZedraApp;

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

    log::info!("Zedra iOS: Creating GPUI application");

    Application::new()
        .with_assets(crate::ZedraAssets)
        .run(|cx: &mut App| {
        log::info!("Zedra iOS: Opening main window");

        cx.open_window(
            WindowOptions {
                window_bounds: None,
                ..Default::default()
            },
            |window, cx| cx.new(|cx| ZedraApp::new(window, cx)),
        )
        .expect("Failed to open window");

        cx.activate(true);
        log::info!("Zedra iOS: Main window created with ZedraApp");
    });
}
