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

use crate::{
    ZedraAssets, app, install_panic_hook, platform_bridge, sheet_host_view::SheetHostView,
};
use gpui::AnyView;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

thread_local! {
    /// Kept alive so window.refresh() can be called from zedra_ios_check_pending_frame.
    static IOS_APP_CELL: RefCell<Option<Rc<AppCell>>> = const { RefCell::new(None) };
    static IOS_WINDOW: RefCell<Option<AnyWindowHandle>> = const { RefCell::new(None) };
    static IOS_SHEET_WINDOW: RefCell<Option<WindowHandle<SheetHostView>>> = const { RefCell::new(None) };
    static IOS_SHEET_WINDOW_PTR: RefCell<*mut std::ffi::c_void> = const { RefCell::new(std::ptr::null_mut()) };
}

unsafe extern "C" {
    fn gpui_ios_set_next_embedded_parent(
        parent_view_ptr: *mut std::ffi::c_void,
        width_pts: f32,
        height_pts: f32,
    );
    fn gpui_ios_get_window() -> *mut std::ffi::c_void;
    fn gpui_ios_attach_embedded_view(
        window_ptr: *mut std::ffi::c_void,
        parent_view_ptr: *mut std::ffi::c_void,
        width_pts: f32,
        height_pts: f32,
    );
    fn gpui_ios_detach_embedded_view(window_ptr: *mut std::ffi::c_void);
}

/// Called each frame from main.m before gpui_ios_request_frame.
///
/// Returns `false` — GPUI polling tasks handle view notifications internally
/// via `cx.notify()`, so forced frames are no longer needed.
#[unsafe(no_mangle)]
pub extern "C" fn zedra_ios_check_pending_frame() -> bool {
    false
}

#[unsafe(no_mangle)]
pub extern "C" fn zedra_launch_gpui() {
    let log_level = if cfg!(feature = "debug-logs") {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    };
    super::logger::IosLogger::init(log_level);

    crate::telemetry::init();
    install_panic_hook();

    platform_bridge::set_bridge(super::bridge::IosBridge);

    tracing::info!("Zedra iOS: Creating GPUI application with IosPlatform");

    let platform: Rc<dyn Platform> = Rc::new(IosPlatform::new());

    let app_cell = App::new_app(
        platform.clone(),
        Arc::new(ZedraAssets),
        Arc::new(http_client::BlockedHttpClient),
    );

    // Register the finish-launching callback via platform.run().
    // On iOS this does NOT block — it stores the callback in the FFI layer.
    // When main.m calls gpui_ios_did_finish_launching(), the callback fires
    // and opens the Metal window with ZedraApp.
    let app_cell_for_callback = app_cell.clone();
    platform.run(Box::new(move || {
        tracing::info!("Zedra iOS: finish-launching callback — opening window");
        let cx = &mut *app_cell_for_callback.borrow_mut();

        let window_options = WindowOptions {
            focus: true,
            show: true,
            ..Default::default()
        };

        match app::open_zedra_window(cx, window_options) {
            Ok(handle) => {
                tracing::info!("Zedra iOS: Window opened: {:?}", handle);
                IOS_WINDOW.with(|w| *w.borrow_mut() = Some(handle));
            }
            Err(err) => tracing::error!("Zedra iOS: Failed to open window: {:?}", err),
        }
    }));

    tracing::info!("Zedra iOS: Callback registered, waiting for didFinishLaunching");

    // Store the AppCell in a thread-local so window.refresh() can be called from
    // zedra_ios_check_pending_frame(). UIKit owns the run loop on iOS; keeping it
    // in a thread-local (rather than std::mem::forget) lets us access it each frame.
    IOS_APP_CELL.with(|cell| *cell.borrow_mut() = Some(app_cell));
}

#[unsafe(no_mangle)]
pub extern "C" fn zedra_ios_mount_custom_sheet_content(
    parent_view_ptr: *mut std::ffi::c_void,
    width_pts: f32,
    height_pts: f32,
) -> *mut std::ffi::c_void {
    if parent_view_ptr.is_null() {
        return std::ptr::null_mut();
    }

    IOS_APP_CELL.with(|cell| {
        let Some(app_cell) = cell.borrow().as_ref().cloned() else {
            return std::ptr::null_mut();
        };
        let Some(sheet_view) = platform_bridge::take_pending_custom_sheet_view() else {
            tracing::error!("Zedra iOS: no pending custom sheet GPUI view");
            return std::ptr::null_mut();
        };

        let mut app = app_cell.borrow_mut();
        let cx: &mut App = &mut app;

        if let Some(window_ptr) = IOS_SHEET_WINDOW_PTR.with(|ptr| {
            let ptr = *ptr.borrow();
            if ptr.is_null() { None } else { Some(ptr) }
        }) {
            IOS_SHEET_WINDOW.with(|sheet_window| {
                if let Some(handle) = sheet_window.borrow().as_ref() {
                    let _ = handle.update(cx, |host, _window, cx| {
                        host.set_content(sheet_view.clone(), cx);
                    });
                }
            });
            unsafe {
                gpui_ios_attach_embedded_view(window_ptr, parent_view_ptr, width_pts, height_pts);
            }
            return window_ptr;
        }

        unsafe {
            gpui_ios_set_next_embedded_parent(parent_view_ptr, width_pts, height_pts);
        }

        let window_options = WindowOptions::default();
        match cx.open_window(window_options, |_window, cx| {
            let sheet_view = sheet_view.clone();
            cx.new(|cx| SheetHostView::new(sheet_view, cx))
        }) {
            Ok(handle) => {
                IOS_SHEET_WINDOW.with(|sheet_window| {
                    *sheet_window.borrow_mut() = Some(handle.clone());
                });
                let window_ptr = unsafe { gpui_ios_get_window() };
                IOS_SHEET_WINDOW_PTR.with(|ptr| *ptr.borrow_mut() = window_ptr);
                window_ptr
            }
            Err(err) => {
                tracing::error!("Zedra iOS: failed to mount custom sheet content: {:?}", err);
                std::ptr::null_mut()
            }
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn zedra_ios_unmount_custom_sheet_content() {
    IOS_SHEET_WINDOW_PTR.with(|ptr| {
        let ptr = *ptr.borrow();
        if !ptr.is_null() {
            unsafe {
                gpui_ios_detach_embedded_view(ptr);
            }
        }
    });
}
