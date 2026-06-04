//! Sheet surface lifecycle helpers. Called from the downstream
//! `Java_dev_zedra_app_SheetHostView_*` JNI exports — the sheet stays in this
//! crate (per the refactor plan) while the root `GpuiSurfaceView` lives in
//! `gpui_android`.

use std::cell::RefCell;

use gpui::*;
use ndk::native_window::NativeWindow;

use crate::sheet_host_view::SheetHostView;
use crate::{android::entry, platform_bridge};

thread_local! {
    static SHEET_WINDOW: RefCell<Option<WindowHandle<SheetHostView>>> =
        const { RefCell::new(None) };
}

pub(crate) fn handle_surface_created(native_window: NativeWindow, width: u32, height: u32) {
    let Some(app_cell) = entry::app_cell() else {
        tracing::error!("sheet: surface created without AppCell");
        return;
    };

    if let Some(result) =
        gpui_android::with_platform(|platform| platform.attach_sheet_native_window(native_window))
    {
        if let Err(error) = result {
            tracing::error!(?error, "sheet: attach_sheet_native_window failed");
            return;
        }
    } else {
        tracing::error!("sheet: platform not registered");
        return;
    }

    let pending_sheet_view = platform_bridge::take_pending_custom_sheet_view();
    let mut app = app_cell.borrow_mut();

    let existing = SHEET_WINDOW.with(|cell| cell.borrow().clone());
    if let Some(handle) = existing {
        if let Some(sheet_view) = pending_sheet_view {
            let _ = handle.update(&mut **app, |host, _window, cx| {
                host.set_content(sheet_view, cx);
            });
        }
    } else if let Some(sheet_view) = pending_sheet_view {
        let scale = platform_bridge::bridge().density();
        let window_options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(0.0), px(0.0)),
                size: size(px(width as f32 / scale), px(height as f32 / scale)),
            })),
            focus: false,
            show: true,
            window_background: WindowBackgroundAppearance::Transparent,
            ..Default::default()
        };

        gpui_android::with_platform(|platform| platform.prepare_embedded_window());

        match app.open_window(window_options, |_window, cx| {
            cx.new(|cx| SheetHostView::new(sheet_view.clone(), cx))
        }) {
            Ok(handle) => {
                SHEET_WINDOW.with(|cell| *cell.borrow_mut() = Some(handle));
            }
            Err(error) => tracing::error!(?error, "sheet: open_window failed"),
        }
    } else {
        tracing::error!("sheet: surface created without pending sheet view");
    }

    drop(app);

    gpui_android::with_platform(|platform| {
        if let Err(error) = platform.handle_sheet_surface_resize(width, height) {
            tracing::error!(?error, "sheet: handle_sheet_surface_resize failed");
        }
        platform.request_frame_forced();
    });
}

pub(crate) fn handle_surface_changed(width: u32, height: u32) {
    gpui_android::with_platform(|platform| {
        if let Err(error) = platform.handle_sheet_surface_resize(width, height) {
            tracing::error!(?error, "sheet: handle_sheet_surface_resize failed");
        }
    });
}

pub(crate) fn handle_surface_destroyed() {
    gpui_android::with_platform(|platform| platform.detach_sheet_native_window());
}

pub(crate) fn handle_touch(action: i32, x: f32, y: f32) {
    gpui_android::with_platform(|platform| platform.handle_sheet_touch(action, x, y));
}

pub(crate) fn handle_fling(velocity_x: f32, velocity_y: f32) {
    gpui_android::with_platform(|platform| platform.handle_sheet_fling(velocity_x, velocity_y));
}
