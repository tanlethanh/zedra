//! Android launch entry point.
//!
//! Mirrors `crates/zedra/src/ios/app.rs::zedra_launch_gpui`. Called once from
//! Kotlin (`MainActivity.zedraLaunchGpui`) after `gpui_android::gpuiInit` has
//! stored the JVM and Activity but before the framework's
//! `gpui_android_did_finish_launching` fires.
//!
//! Steps:
//!   1. Set up logging, panic hook, telemetry.
//!   2. Register the `AndroidBridge` `PlatformBridge` impl.
//!   3. Construct `AndroidPlatform` via `gpui_android::create_platform()`.
//!   4. Build the GPUI `AppCell` and store it in a thread-local.
//!   5. Register the finish-launching callback that opens the root Zedra
//!      window. The callback fires later when `gpuiDidFinishLaunching` runs
//!      from Kotlin.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::*;
use jni::JNIEnv;
use jni::objects::JClass;

use crate::android::bridge::AndroidBridge;
use crate::{app, install_panic_hook, platform_bridge};

thread_local! {
    /// Kept alive so the GPUI runtime survives across Choreographer ticks.
    static ANDROID_APP_CELL: RefCell<Option<Rc<AppCell>>> = const { RefCell::new(None) };
    static ANDROID_WINDOW: RefCell<Option<AnyWindowHandle>> = const { RefCell::new(None) };
}

/// JNI entry point invoked from `MainActivity.zedraLaunchGpui`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_MainActivity_zedraLaunchGpui(
    _env: JNIEnv,
    _class: JClass,
) {
    super::jni::init_logging();
    crate::telemetry::init();
    install_panic_hook();

    tracing::info!("Zedra Android: creating GPUI application with AndroidPlatform");

    let platform: Rc<dyn Platform> = gpui_android::create_platform();
    let app_cell = app::init_platform_app(platform.clone(), AndroidBridge);

    let app_cell_for_callback = app_cell.clone();
    platform.run(Box::new(move || {
        tracing::info!("Zedra Android: finish-launching — opening root window");
        let cx = &mut *app_cell_for_callback.borrow_mut();

        let scale = platform_bridge::bridge().density();
        let window_options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(0.0), px(0.0)),
                // Bounds get overwritten by the first surfaceChanged callback;
                // pick something sane based on the framework's display scale.
                size: size(px(1080.0 / scale), px(1920.0 / scale)),
            })),
            focus: true,
            show: true,
            // Match iOS — subpixel text rendering is gated on opaque windows
            // (`window.rs:3800-3802`). Keeping this Opaque means GPUI tries the
            // RGB subpixel path on GPUs that support dual-source blending.
            window_background: WindowBackgroundAppearance::Opaque,
            ..Default::default()
        };

        match app::open_zedra_window(cx, window_options) {
            Ok(handle) => {
                ANDROID_WINDOW.with(|window| *window.borrow_mut() = Some(handle));
            }
            Err(error) => tracing::error!("Zedra Android: open_zedra_window failed: {error:?}"),
        }
    }));

    ANDROID_APP_CELL.with(|cell| *cell.borrow_mut() = Some(app_cell));
}

/// Returns the root `AppCell` if `zedra_launch_gpui` has run on this thread.
pub(crate) fn app_cell() -> Option<Rc<AppCell>> {
    ANDROID_APP_CELL.with(|cell| cell.borrow().clone())
}

pub(crate) fn handle_system_back() -> bool {
    let Some(app_cell) = app_cell() else {
        return false;
    };
    let Some(any_window) = ANDROID_WINDOW.with(|window| *window.borrow()) else {
        return false;
    };
    let Some(window) = any_window.downcast::<app::ZedraApp>() else {
        return false;
    };

    let mut app = app_cell.borrow_mut();
    let cx: &mut App = &mut app;
    window
        .update(cx, |view, window, cx| view.handle_system_back(window, cx))
        .unwrap_or(false)
}
