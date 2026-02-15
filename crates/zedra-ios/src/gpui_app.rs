/// GPUI-based iOS app — renders via Metal through GPUI's blade renderer.
///
/// This is the iOS equivalent of crates/zedra/src/zedra_app.rs on Android.
/// For the initial integration, it renders a simple branded view to prove
/// the GPUI → Blade → Metal pipeline works on iOS.
use gpui::*;

/// Root view for the Zedra iOS app.
pub struct ZedraIosApp {
    frame_count: usize,
}

impl ZedraIosApp {
    pub fn new() -> Self {
        Self { frame_count: 0 }
    }
}

impl Render for ZedraIosApp {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.frame_count += 1;

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x1e1e2e))
            .items_center()
            .justify_center()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_4()
                    .child(
                        div()
                            .text_color(rgb(0xcdd6f4))
                            .text_size(px(48.0))
                            .child("Zedra"),
                    )
                    .child(
                        div()
                            .text_color(rgb(0xa6adc8))
                            .text_size(px(18.0))
                            .child("GPUI + Metal on iOS"),
                    )
                    .child(
                        div()
                            .w(px(200.0))
                            .h(px(4.0))
                            .bg(rgb(0x89b4fa))
                            .rounded(px(2.0)),
                    )
                    .child(
                        div()
                            .mt_8()
                            .text_color(rgb(0x6c7086))
                            .text_size(px(14.0))
                            .child(format!("Frame #{}", self.frame_count)),
                    ),
            )
    }
}

/// Create the GPUI Application and open the main window.
///
/// Lifecycle (called from Obj-C app delegate):
///   1. gpui_ios_initialize()         — set up GPUI FFI state
///   2. zedra_launch_gpui()           — create Application, register window callback
///   3. gpui_ios_did_finish_launching — invoke callback → opens window
///   4. gpui_ios_get_window()         — get window pointer for CADisplayLink
///   5. gpui_ios_request_frame()      — called each frame by CADisplayLink
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

    // Application::new() creates IosPlatform. .run() stores the callback via
    // set_finish_launching_callback (which needs IOS_APP_STATE — the Obj-C side
    // must call gpui_ios_initialize() before this function).
    Application::new().run(|cx: &mut App| {
        log::info!("Zedra iOS: Opening main window");

        cx.open_window(
            WindowOptions {
                window_bounds: None,
                ..Default::default()
            },
            |_, cx| cx.new(|_| ZedraIosApp::new()),
        )
        .expect("Failed to open window");

        cx.activate(true);
        log::info!("Zedra iOS: Main window created");
    });
}
