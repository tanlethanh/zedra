//! App-specific JNI bridge for the downstream Android app.
//!
//! Framework-level JNI exports (surface lifecycle, touch / IME / fling, app
//! lifecycle, Choreographer-driven frames) live in `gpui_android::android::ffi`
//! and are reached via the framework's Kotlin classes
//! (`dev.zed.gpui.GpuiSurfaceView`, `dev.zed.gpui.GpuiRuntimeController`).
//!
//! What stays here:
//!   * Sheet surface lifecycle (`Java_dev_zedra_app_SheetHostView_*`).
//!   * App-specific JNI exports: deeplink, QR scanner, alerts, selection,
//!     text input, floating button, dictation preview, native notifications,
//!     sheet content position query.
//!   * Rust→Java callers reaching `dev.zedra.app.MainActivity` static methods
//!     (alerts, sheets, haptics, keyboard show/hide, etc.).

use jni::{
    JNIEnv, JavaVM,
    objects::{GlobalRef, JClass, JObject, JValue},
    sys::{jboolean, jfloat, jint, jlong},
};
use ndk::native_window::NativeWindow;
use std::sync::{Arc, Mutex, Once};

use crate::android::sheet;
use crate::install_panic_hook;
use crate::platform_bridge::{
    self, AlertButton, AlertButtonStyle, CustomSheetOptions, HapticFeedback, ListPickerItem,
    NativeDictationPreviewOptions, NativeEditMenuItem, NativeFloatingButtonOptions,
    NativeNotificationOptions, SoundEffect, SystemTheme,
};

// ============================================================================
// Globals (downstream-specific)
// ============================================================================

static JVM: Mutex<Option<Arc<JavaVM>>> = Mutex::new(None);
static INIT: Once = Once::new();
static FILES_DIR: Mutex<Option<String>> = Mutex::new(None);
static APP_VERSION: Mutex<Option<String>> = Mutex::new(None);
static APP_BUILD_NUMBER: Mutex<Option<String>> = Mutex::new(None);
static OS_VERSION: Mutex<Option<String>> = Mutex::new(None);
static DEVICE_NAME: Mutex<Option<String>> = Mutex::new(None);

/// Display density, soft keyboard height, and system insets are owned by the
/// `gpui_android` framework. These thin wrappers preserve the historical
/// `crate::android::jni::get_*` call sites.
pub fn get_density() -> f32 {
    gpui_android::display_scale()
}

pub fn get_keyboard_height() -> u32 {
    gpui_android::keyboard_height()
}

pub fn get_system_inset_top() -> u32 {
    gpui_android::system_inset_top()
}

pub fn get_system_inset_bottom() -> u32 {
    gpui_android::system_inset_bottom()
}

pub fn get_files_dir() -> Option<String> {
    FILES_DIR.lock().ok()?.clone()
}

pub fn get_app_version() -> String {
    APP_VERSION
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or_default()
}

pub fn get_app_build_number() -> String {
    APP_BUILD_NUMBER
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or_default()
}

pub fn get_os_version() -> String {
    OS_VERSION
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or_default()
}

pub fn get_delta_device_name() -> String {
    DEVICE_NAME
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or_default()
}

/// Initialize logging + panic hook. Idempotent; safe to call from multiple
/// JNI entry points.
pub fn init_logging() {
    INIT.call_once(|| {
        android_logger::init_once(
            android_logger::Config::default()
                .with_max_level(if cfg!(feature = "debug-logs") {
                    log::LevelFilter::Debug
                } else {
                    log::LevelFilter::Info
                })
                .with_tag("zedra")
                .with_filter(
                    android_logger::FilterBuilder::new()
                        .parse(if cfg!(feature = "debug-logs") {
                            "debug,tracing::span=off,tracing::span::active=off"
                        } else {
                            "info,tracing::span=off,tracing::span::active=off"
                        })
                        .build(),
                ),
        );

        crate::telemetry::init();
        install_panic_hook();
    });
}

// ============================================================================
// Sheet surface lifecycle
// ============================================================================

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_SheetHostView_nativeSheetSurfaceCreated(
    env: JNIEnv,
    _this: JObject,
    surface: JObject,
) {
    let native_window = match unsafe { NativeWindow::from_surface(env.get_raw(), surface.as_raw()) }
    {
        Some(w) => w,
        None => {
            tracing::error!("jni: failed to obtain ANativeWindow from sheet Surface");
            return;
        }
    };
    let width = native_window.width() as u32;
    let height = native_window.height() as u32;
    sheet::handle_surface_created(native_window, width, height);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_SheetHostView_nativeSheetSurfaceChanged(
    _env: JNIEnv,
    _this: JObject,
    _format: jint,
    width: jint,
    height: jint,
) {
    sheet::handle_surface_changed(width.max(0) as u32, height.max(0) as u32);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_SheetHostView_nativeSheetSurfaceDestroyed(
    _env: JNIEnv,
    _this: JObject,
) {
    sheet::handle_surface_destroyed();
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_SheetHostView_nativeSheetTouchEvent(
    _env: JNIEnv,
    _this: JObject,
    action: jint,
    x: jfloat,
    y: jfloat,
    _pointer_id: jint,
) {
    sheet::handle_touch(action, x, y);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_SheetHostView_nativeSheetFlingEvent(
    _env: JNIEnv,
    _this: JObject,
    velocity_x: jfloat,
    velocity_y: jfloat,
) {
    sheet::handle_fling(velocity_x, velocity_y);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_SheetHostView_nativeSheetWindowHandle(
    _env: JNIEnv,
    _this: JObject,
) -> jlong {
    // Opaque selection-routing handle of the sheet's GPUI window. Fetched (not
    // generated in Kotlin) so it stays correct across sheet surface re-creation.
    gpui_android::with_platform(|platform| platform.sheet_window_handle()).unwrap_or(0) as jlong
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_SheetHostView_nativeSheetProcessSurfaceCommands(
    _env: JNIEnv,
    _this: JObject,
) {
    gpui_android::with_platform(|platform| {
        platform.process_pending_tasks();
        platform.process_fling();
        platform.request_frame_for_all_windows();
    });
}

// ============================================================================
// MainActivity initialization (downstream — gathers JVM, files dir, hands
// JVM/activity to the framework)
// ============================================================================

/// Called from `MainActivity.onCreate` via
/// `MainActivity.bootstrap(activity, appVersion, appBuildNumber, osVersion, deviceName)`.
///
/// Captures the JVM (for Rust→Java callbacks), the files directory, and native
/// app/device metadata. Pushing metadata in this direction (Java→Rust)
/// avoids deeply-nested Rust→Java JNI calls during render which can manifest
/// as `StackOverflowError` once GPUI's element tree gets large.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_MainActivity_bootstrap(
    mut env: JNIEnv,
    _class: JClass,
    activity: JObject,
    app_version: jni::objects::JString,
    app_build_number: jni::objects::JString,
    os_version: jni::objects::JString,
    device_name: jni::objects::JString,
) {
    init_logging();

    if let Ok(jvm) = env.get_java_vm() {
        if let Ok(mut guard) = JVM.lock() {
            *guard = Some(Arc::new(jvm));
        }
    } else {
        tracing::error!("jni: bootstrap failed to obtain JavaVM");
    }

    if let Ok(jni::objects::JValueGen::Object(file_obj)) =
        env.call_method(&activity, "getFilesDir", "()Ljava/io/File;", &[])
    {
        if let Ok(jni::objects::JValueGen::Object(path_str)) =
            env.call_method(&file_obj, "getAbsolutePath", "()Ljava/lang/String;", &[])
        {
            let jstr = jni::objects::JString::from(path_str);
            if let Ok(path) = env.get_string(&jstr) {
                let path: String = path.into();
                if let Ok(mut guard) = FILES_DIR.lock() {
                    *guard = Some(path);
                }
            }
        }
    }

    if let Ok(version) = env.get_string(&app_version) {
        let version: String = version.into();
        if let Ok(mut guard) = APP_VERSION.lock() {
            *guard = Some(version);
        }
    }

    if let Ok(build) = env.get_string(&app_build_number) {
        let build: String = build.into();
        if let Ok(mut guard) = APP_BUILD_NUMBER.lock() {
            *guard = Some(build);
        }
    }

    if let Ok(version) = env.get_string(&os_version) {
        let version: String = version.into();
        if let Ok(mut guard) = OS_VERSION.lock() {
            *guard = Some(version);
        }
    }

    if let Ok(name) = env.get_string(&device_name) {
        let name: String = name.into();
        if let Ok(mut guard) = DEVICE_NAME.lock() {
            *guard = Some(name);
        }
    }
}

// ============================================================================
// App-specific JNI exports — deeplink, QR scanner, alerts, selection,
// text input, floating button, dictation preview, notifications, sheet
// position query
// ============================================================================

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_QRScannerActivity_nativeOnQrCodeScanned(
    mut env: JNIEnv,
    _class: JClass,
    data: jni::objects::JString,
) {
    let qr_data: String = match env.get_string(&data) {
        Ok(s) => s.into(),
        Err(error) => {
            tracing::error!(?error, "jni: failed to read QR data");
            return;
        }
    };
    dispatch_deeplink(qr_data);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_MainActivity_nativeDeeplinkReceived(
    mut env: JNIEnv,
    _class: JClass,
    url: jni::objects::JString,
) {
    let deeplink_url: String = match env.get_string(&url) {
        Ok(s) => s.into(),
        Err(error) => {
            tracing::error!(?error, "jni: failed to read deeplink URL");
            return;
        }
    };
    dispatch_deeplink(deeplink_url);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_MainActivity_nativeAlertResult(
    _env: JNIEnv,
    _class: JClass,
    callback_id: jint,
    button_index: jint,
) {
    if callback_id <= 0 || button_index < 0 {
        return;
    }
    platform_bridge::dispatch_alert_result(callback_id as u32, button_index as usize);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_MainActivity_nativeAlertDismiss(
    _env: JNIEnv,
    _class: JClass,
    callback_id: jint,
) {
    if callback_id <= 0 {
        return;
    }
    platform_bridge::dispatch_alert_dismiss(callback_id as u32);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_MainActivity_nativeSelectionResult(
    _env: JNIEnv,
    _class: JClass,
    callback_id: jint,
    button_index: jint,
) {
    if callback_id <= 0 || button_index < 0 {
        return;
    }
    platform_bridge::dispatch_selection_result(callback_id as u32, button_index as usize);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_MainActivity_nativeSelectionDismiss(
    _env: JNIEnv,
    _class: JClass,
    callback_id: jint,
) {
    if callback_id <= 0 {
        return;
    }
    platform_bridge::dispatch_selection_dismiss(callback_id as u32);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_MainActivity_nativeEditMenuResult(
    _env: JNIEnv,
    _class: JClass,
    callback_id: jint,
    item_index: jint,
) {
    if callback_id <= 0 || item_index < 0 {
        return;
    }
    if let Some(app_cell) = crate::android::entry::app_cell() {
        let mut app = app_cell.borrow_mut();
        platform_bridge::dispatch_native_edit_menu_result(
            callback_id as u32,
            item_index as usize,
            &mut **app,
        );
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_MainActivity_nativeEditMenuDismiss(
    _env: JNIEnv,
    _class: JClass,
    callback_id: jint,
) {
    if callback_id <= 0 {
        return;
    }
    platform_bridge::dispatch_native_edit_menu_dismiss(callback_id as u32);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_MainActivity_nativeTextInputResult(
    mut env: JNIEnv,
    _class: JClass,
    callback_id: jint,
    value: jni::objects::JString,
) {
    if callback_id <= 0 {
        return;
    }
    let value: String = match env.get_string(&value) {
        Ok(value) => value.into(),
        Err(error) => {
            tracing::error!(?error, "jni: failed to read text input result");
            return;
        }
    };
    platform_bridge::dispatch_text_input_result(callback_id as u32, value);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_MainActivity_nativeTextInputDismiss(
    _env: JNIEnv,
    _class: JClass,
    callback_id: jint,
) {
    if callback_id <= 0 {
        return;
    }
    platform_bridge::dispatch_text_input_dismiss(callback_id as u32);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_MainActivity_nativeFloatingButtonPressed(
    _env: JNIEnv,
    _class: JClass,
    callback_id: jint,
) {
    if callback_id <= 0 {
        return;
    }
    if let Some(app_cell) = crate::android::entry::app_cell() {
        let mut app = app_cell.borrow_mut();
        platform_bridge::dispatch_native_floating_button_press(callback_id as u32, &mut **app);
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_MainActivity_nativeDictationPreviewDismiss(
    _env: JNIEnv,
    _class: JClass,
    preview_id: jint,
) {
    if preview_id <= 0 {
        return;
    }
    if let Some(app_cell) = crate::android::entry::app_cell() {
        let mut app = app_cell.borrow_mut();
        platform_bridge::dispatch_native_dictation_preview_dismiss(preview_id as u32, &mut **app);
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_MainActivity_nativeNotificationAction(
    _env: JNIEnv,
    _class: JClass,
    callback_id: jint,
) {
    if callback_id <= 0 {
        return;
    }
    platform_bridge::dispatch_native_notification_action(callback_id as u32);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_MainActivity_nativeNotificationDismiss(
    _env: JNIEnv,
    _class: JClass,
    callback_id: jint,
) {
    if callback_id <= 0 {
        return;
    }
    platform_bridge::dispatch_native_notification_dismiss(callback_id as u32);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_MainActivity_nativeSheetContentIsAtTop(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    crate::native_presentation::sheet_content_is_at_top() as jboolean
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_MainActivity_nativeKeyboardAccessoryKey(
    mut env: JNIEnv,
    _class: JClass,
    key: jni::objects::JString,
) {
    let key_name: String = match env.get_string(&key) {
        Ok(key) => key.into(),
        Err(error) => {
            tracing::error!(?error, "jni: failed to read keyboard accessory key");
            return;
        }
    };
    let handled = gpui_android::with_platform(|platform| {
        platform.handle_keyboard_accessory_action(&key_name)
    })
    .unwrap_or(false);
    if !handled {
        tracing::debug!(
            action = key_name.as_str(),
            "jni: keyboard accessory action was not handled"
        );
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_MainActivity_nativeKeyboardAccessoryVisible(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    gpui_android::with_platform(|platform| platform.has_active_keyboard_accessory())
        .unwrap_or(false) as jboolean
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_MainActivity_nativeSystemBackPressed(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    crate::android::entry::handle_system_back() as jboolean
}

/// Foreground/background state from the Android activity lifecycle
/// (onResume → true, onStop → false). Mirrors the iOS bridge so the host can
/// decide between RPC-only delivery and Delta push notifications.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_MainActivity_nativeSetAppForeground(
    _env: JNIEnv,
    _class: JClass,
    foreground: jboolean,
) {
    platform_bridge::set_app_in_foreground(foreground != 0);
}

fn dispatch_deeplink(url: String) {
    tracing::info!(url = &url[..url.len().min(80)], "jni: deeplink");
    match crate::deeplink::parse(&url) {
        Ok(action) => crate::deeplink::enqueue(action),
        Err(error) => tracing::error!(?error, "jni: invalid deeplink URL"),
    }
}

// ============================================================================
// Rust → Java helpers (call MainActivity static methods)
// ============================================================================

pub(crate) fn jni_call(name: &'static str, f: impl FnOnce() + std::panic::UnwindSafe) {
    if let Err(e) = std::panic::catch_unwind(f) {
        tracing::error!(name, err = ?e, "jni: panic");
    }
}

pub(crate) fn with_class(
    name: &'static str,
    class_name: &'static str,
    f: impl for<'local> FnOnce(&mut JNIEnv<'local>, &JClass<'local>),
) {
    let jvm = match JVM.lock() {
        Ok(guard) => match guard.as_ref() {
            Some(jvm) => jvm.clone(),
            None => {
                tracing::error!(name, "JVM not available");
                return;
            }
        },
        Err(error) => {
            tracing::error!(name, ?error, "Failed to lock JVM mutex");
            return;
        }
    };

    let mut env = match jvm.get_env() {
        Ok(env) => env,
        Err(_) => match jvm.attach_current_thread_as_daemon() {
            Ok(env) => env,
            Err(error) => {
                tracing::error!(name, ?error, "Failed to attach thread");
                return;
            }
        },
    };

    let class = match env.find_class(class_name) {
        Ok(class) => class,
        Err(error) => {
            tracing::error!(name, class_name, ?error, "Failed to find JNI class");
            if env.exception_check().unwrap_or(false) {
                env.exception_describe().ok();
                env.exception_clear().ok();
            }
            return;
        }
    };

    f(&mut env, &class);

    if env.exception_check().unwrap_or(false) {
        env.exception_describe().ok();
        env.exception_clear().ok();
    }
}

fn with_main_activity_class(
    name: &'static str,
    f: impl for<'local> FnOnce(&mut JNIEnv<'local>, &JClass<'local>),
) {
    with_class(name, "dev/zedra/app/MainActivity", f);
}

pub fn show_keyboard() {
    jni_call("show_keyboard", || {
        with_main_activity_class("show_keyboard", |env, class| {
            if let Err(e) = env.call_static_method(class, "showKeyboard", "()V", &[]) {
                tracing::error!("jni: showKeyboard failed: {:?}", e);
            }
        });
    });
}

pub fn hide_keyboard() {
    jni_call("hide_keyboard", || {
        with_main_activity_class("hide_keyboard", |env, class| {
            if let Err(e) = env.call_static_method(class, "hideKeyboard", "()V", &[]) {
                tracing::error!("jni: hideKeyboard failed: {:?}", e);
            }
        });
    });
}

pub fn launch_qr_scanner() {
    jni_call("launch_qr_scanner", || {
        with_main_activity_class("launch_qr_scanner", |env, class| {
            if let Err(e) = env.call_static_method(class, "launchQrScanner", "()V", &[]) {
                tracing::error!("jni: launchQrScanner failed: {:?}", e);
            }
        });
    });
}

pub fn open_url(url: &str) {
    let url_owned = url.to_string();
    jni_call("open_url", move || {
        with_main_activity_class("open_url", |env, class| {
            let j_url = match env.new_string(&url_owned) {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!("jni: new_string url failed: {:?}", e);
                    return;
                }
            };
            if let Err(e) = env.call_static_method(
                class,
                "openUrl",
                "(Ljava/lang/String;)V",
                &[(&j_url).into()],
            ) {
                tracing::error!("jni: openUrl failed: {:?}", e);
            }
        });
    });
}

pub fn show_alert(id: u32, title: &str, message: &str, buttons: &[AlertButton]) {
    let title = title.to_string();
    let message = message.to_string();
    let labels: Vec<String> = buttons.iter().map(|b| b.label.clone()).collect();
    let styles: Vec<jint> = buttons
        .iter()
        .map(|b| match b.style {
            AlertButtonStyle::Default => 0,
            AlertButtonStyle::Cancel => 1,
            AlertButtonStyle::Destructive => 2,
        })
        .collect();
    jni_call("show_alert", move || {
        present_buttons("showAlert", id, title, message, labels, styles);
    });
}

pub fn show_selection(id: u32, title: &str, message: &str, buttons: &[AlertButton]) {
    let title = title.to_string();
    let message = message.to_string();
    let labels: Vec<String> = buttons.iter().map(|b| b.label.clone()).collect();
    let styles: Vec<jint> = buttons
        .iter()
        .map(|b| match b.style {
            AlertButtonStyle::Default => 0,
            AlertButtonStyle::Cancel => 1,
            AlertButtonStyle::Destructive => 2,
        })
        .collect();
    let image_names: Vec<String> = buttons
        .iter()
        .map(|b| b.image_name.clone().unwrap_or_default())
        .collect();
    jni_call("show_selection", move || {
        with_main_activity_class("show_selection", |env, class| {
            let title_value = match env.new_string(&title) {
                Ok(value) => value,
                Err(error) => {
                    tracing::error!(?error, "jni: show_selection title string failed");
                    return;
                }
            };
            let message_value = match env.new_string(&message) {
                Ok(value) => value,
                Err(error) => {
                    tracing::error!(?error, "jni: show_selection message string failed");
                    return;
                }
            };
            let string_class = match env.find_class("java/lang/String") {
                Ok(class) => class,
                Err(error) => {
                    tracing::error!(?error, "jni: show_selection find String class failed");
                    return;
                }
            };
            let label_array =
                match env.new_object_array(labels.len() as i32, &string_class, JObject::null()) {
                    Ok(array) => array,
                    Err(error) => {
                        tracing::error!(?error, "jni: show_selection label array failed");
                        return;
                    }
                };
            for (index, label) in labels.iter().enumerate() {
                let label_value = match env.new_string(label) {
                    Ok(value) => value,
                    Err(error) => {
                        tracing::error!(?error, "jni: show_selection label string failed");
                        return;
                    }
                };
                if let Err(error) =
                    env.set_object_array_element(&label_array, index as i32, label_value)
                {
                    tracing::error!(?error, "jni: show_selection set label failed");
                    return;
                }
            }
            let style_array = match env.new_int_array(styles.len() as i32) {
                Ok(array) => array,
                Err(error) => {
                    tracing::error!(?error, "jni: show_selection style array failed");
                    return;
                }
            };
            if let Err(error) = env.set_int_array_region(&style_array, 0, &styles) {
                tracing::error!(?error, "jni: show_selection populate styles failed");
                return;
            }
            let image_array = match env.new_object_array(
                image_names.len() as i32,
                &string_class,
                JObject::null(),
            ) {
                Ok(array) => array,
                Err(error) => {
                    tracing::error!(?error, "jni: show_selection image array failed");
                    return;
                }
            };
            for (index, name) in image_names.iter().enumerate() {
                let name_value = match env.new_string(name) {
                    Ok(value) => value,
                    Err(error) => {
                        tracing::error!(?error, "jni: show_selection image name string failed");
                        return;
                    }
                };
                if let Err(error) =
                    env.set_object_array_element(&image_array, index as i32, name_value)
                {
                    tracing::error!(?error, "jni: show_selection set image name failed");
                    return;
                }
            }
            if let Err(error) = env.call_static_method(
                class,
                "showSelection",
                "(ILjava/lang/String;Ljava/lang/String;[Ljava/lang/String;[I[Ljava/lang/String;)V",
                &[
                    JValue::Int(id as i32),
                    JValue::Object(&title_value),
                    JValue::Object(&message_value),
                    JValue::Object(&label_array),
                    JValue::Object(&style_array),
                    JValue::Object(&image_array),
                ],
            ) {
                tracing::error!(?error, "jni: showSelection call failed");
            }
        });
    });
}

pub fn show_list_picker(id: u32, title: &str, message: &str, items: &[ListPickerItem]) {
    let title = title.to_string();
    let message = message.to_string();
    let labels: Vec<String> = items.iter().map(|item| item.label.clone()).collect();
    let subtitles: Vec<String> = items
        .iter()
        .map(|item| item.subtitle.clone().unwrap_or_default())
        .collect();
    let image_names: Vec<String> = items
        .iter()
        .map(|item| item.image_name.clone().unwrap_or_default())
        .collect();
    jni_call("show_list_picker", move || {
        with_main_activity_class("show_list_picker", |env, class| {
            let title_value = match env.new_string(&title) {
                Ok(value) => value,
                Err(error) => {
                    tracing::error!(?error, "jni: show_list_picker title string failed");
                    return;
                }
            };
            let message_value = match env.new_string(&message) {
                Ok(value) => value,
                Err(error) => {
                    tracing::error!(?error, "jni: show_list_picker message string failed");
                    return;
                }
            };
            let string_class = match env.find_class("java/lang/String") {
                Ok(class) => class,
                Err(error) => {
                    tracing::error!(?error, "jni: show_list_picker find String class failed");
                    return;
                }
            };
            let label_array =
                match env.new_object_array(labels.len() as i32, &string_class, JObject::null()) {
                    Ok(array) => array,
                    Err(error) => {
                        tracing::error!(?error, "jni: show_list_picker label array failed");
                        return;
                    }
                };
            for (index, label) in labels.iter().enumerate() {
                let label_value = match env.new_string(label) {
                    Ok(value) => value,
                    Err(error) => {
                        tracing::error!(?error, "jni: show_list_picker label string failed");
                        return;
                    }
                };
                if let Err(error) =
                    env.set_object_array_element(&label_array, index as i32, label_value)
                {
                    tracing::error!(?error, "jni: show_list_picker set label failed");
                    return;
                }
            }
            let subtitle_array = match env.new_object_array(
                subtitles.len() as i32,
                &string_class,
                JObject::null(),
            ) {
                Ok(array) => array,
                Err(error) => {
                    tracing::error!(?error, "jni: show_list_picker subtitle array failed");
                    return;
                }
            };
            for (index, subtitle) in subtitles.iter().enumerate() {
                let subtitle_value = match env.new_string(subtitle) {
                    Ok(value) => value,
                    Err(error) => {
                        tracing::error!(?error, "jni: show_list_picker subtitle string failed");
                        return;
                    }
                };
                if let Err(error) =
                    env.set_object_array_element(&subtitle_array, index as i32, subtitle_value)
                {
                    tracing::error!(?error, "jni: show_list_picker set subtitle failed");
                    return;
                }
            }
            let image_array = match env.new_object_array(
                image_names.len() as i32,
                &string_class,
                JObject::null(),
            ) {
                Ok(array) => array,
                Err(error) => {
                    tracing::error!(?error, "jni: show_list_picker image array failed");
                    return;
                }
            };
            for (index, image) in image_names.iter().enumerate() {
                let image_value = match env.new_string(image) {
                    Ok(value) => value,
                    Err(error) => {
                        tracing::error!(?error, "jni: show_list_picker image string failed");
                        return;
                    }
                };
                if let Err(error) =
                    env.set_object_array_element(&image_array, index as i32, image_value)
                {
                    tracing::error!(?error, "jni: show_list_picker set image failed");
                    return;
                }
            }
            if let Err(error) = env.call_static_method(
                class,
                "showListPicker",
                "(ILjava/lang/String;Ljava/lang/String;[Ljava/lang/String;[Ljava/lang/String;[Ljava/lang/String;)V",
                &[
                    JValue::Int(id as i32),
                    JValue::Object(&title_value),
                    JValue::Object(&message_value),
                    JValue::Object(&label_array),
                    JValue::Object(&subtitle_array),
                    JValue::Object(&image_array),
                ],
            ) {
                tracing::error!(?error, "jni: showListPicker call failed");
            }
        });
    });
}

fn new_string_array<'local>(
    env: &mut JNIEnv<'local>,
    string_class: &JClass<'local>,
    values: &[String],
) -> Result<jni::objects::JObjectArray<'local>, jni::errors::Error> {
    let array = env.new_object_array(values.len() as i32, string_class, JObject::null())?;
    for (index, value) in values.iter().enumerate() {
        let value = env.new_string(value)?;
        env.set_object_array_element(&array, index as i32, value)?;
    }
    Ok(array)
}

pub fn show_native_edit_menu(
    id: u32,
    position: gpui::Point<gpui::Pixels>,
    items: &[NativeEditMenuItem],
) {
    let x = f32::from(position.x);
    let y = f32::from(position.y);
    let labels: Vec<String> = items.iter().map(|item| item.label.clone()).collect();
    let image_names: Vec<String> = items
        .iter()
        .map(|item| item.image_name.clone().unwrap_or_default())
        .collect();
    jni_call("show_native_edit_menu", move || {
        with_main_activity_class("show_native_edit_menu", |env, class| {
            let string_class = match env.find_class("java/lang/String") {
                Ok(class) => class,
                Err(error) => {
                    tracing::error!(
                        ?error,
                        "jni: show_native_edit_menu find String class failed"
                    );
                    return;
                }
            };
            let label_array = match new_string_array(env, &string_class, &labels) {
                Ok(array) => array,
                Err(error) => {
                    tracing::error!(?error, "jni: show_native_edit_menu label array failed");
                    return;
                }
            };
            let image_array = match new_string_array(env, &string_class, &image_names) {
                Ok(array) => array,
                Err(error) => {
                    tracing::error!(?error, "jni: show_native_edit_menu image array failed");
                    return;
                }
            };
            if let Err(error) = env.call_static_method(
                class,
                "showNativeEditMenu",
                "(IFF[Ljava/lang/String;[Ljava/lang/String;)V",
                &[
                    (id as jint).into(),
                    x.into(),
                    y.into(),
                    JValue::Object(&label_array),
                    JValue::Object(&image_array),
                ],
            ) {
                tracing::error!(?error, "jni: showNativeEditMenu call failed");
            }
        });
    });
}

fn present_buttons(
    method_name: &str,
    id: u32,
    title: String,
    message: String,
    button_labels: Vec<String>,
    button_styles: Vec<jint>,
) {
    with_main_activity_class("present_buttons", |env, class| {
        let title_value = match env.new_string(&title) {
            Ok(value) => value,
            Err(error) => {
                tracing::error!(?error, "jni: title string failed");
                return;
            }
        };
        let message_value = match env.new_string(&message) {
            Ok(value) => value,
            Err(error) => {
                tracing::error!(?error, "jni: message string failed");
                return;
            }
        };
        let string_class = match env.find_class("java/lang/String") {
            Ok(class) => class,
            Err(error) => {
                tracing::error!(?error, "jni: find String class failed");
                return;
            }
        };
        let label_array =
            match env.new_object_array(button_labels.len() as i32, string_class, JObject::null()) {
                Ok(array) => array,
                Err(error) => {
                    tracing::error!(?error, "jni: label array failed");
                    return;
                }
            };
        for (index, label) in button_labels.iter().enumerate() {
            let label_value = match env.new_string(label) {
                Ok(value) => value,
                Err(error) => {
                    tracing::error!(?error, "jni: label string failed");
                    return;
                }
            };
            if let Err(error) =
                env.set_object_array_element(&label_array, index as i32, &label_value)
            {
                tracing::error!(?error, "jni: populate label array failed");
                return;
            }
        }
        let style_array = match env.new_int_array(button_styles.len() as i32) {
            Ok(array) => array,
            Err(error) => {
                tracing::error!(?error, "jni: style array failed");
                return;
            }
        };
        if let Err(error) = env.set_int_array_region(&style_array, 0, &button_styles) {
            tracing::error!(?error, "jni: populate style array failed");
            return;
        }
        if let Err(error) = env.call_static_method(
            class,
            method_name,
            "(ILjava/lang/String;Ljava/lang/String;[Ljava/lang/String;[I)V",
            &[
                (id as jint).into(),
                (&title_value).into(),
                (&message_value).into(),
                (&label_array).into(),
                (&style_array).into(),
            ],
        ) {
            tracing::error!(
                method = method_name,
                ?error,
                "jni: call_static_method failed"
            );
        }
    });
}

// =============================================================================
// Delta notification bridge (push token registration + in-app banners)
// =============================================================================

/// Acquire a `JNIEnv` for the current thread plus the `MainActivity` class, then
/// run `f`. Logs and clears any pending Java exception. Keeps the Delta bridge
/// calls below concise instead of repeating the JVM/env/class boilerplate.
fn with_main_activity<R>(
    name: &'static str,
    f: impl FnOnce(&mut JNIEnv, &JClass) -> Result<R, jni::errors::Error>,
) -> Option<R> {
    let jvm = match JVM.lock() {
        Ok(guard) => guard.as_ref()?.clone(),
        Err(e) => {
            tracing::error!(name, "jni: lock JVM failed: {:?}", e);
            return None;
        }
    };

    let mut env = match jvm.get_env() {
        Ok(env) => env,
        Err(_) => match jvm.attach_current_thread_as_daemon() {
            Ok(env) => env,
            Err(e) => {
                tracing::error!(name, "jni: attach thread failed: {:?}", e);
                return None;
            }
        },
    };

    let class = match env.find_class("dev/zedra/app/MainActivity") {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(name, "jni: find MainActivity failed: {:?}", e);
            if env.exception_check().unwrap_or(false) {
                env.exception_describe().ok();
                env.exception_clear().ok();
            }
            return None;
        }
    };

    match f(&mut env, &class) {
        Ok(value) => Some(value),
        Err(e) => {
            tracing::error!(name, "jni: call failed: {:?}", e);
            if env.exception_check().unwrap_or(false) {
                env.exception_describe().ok();
                env.exception_clear().ok();
            }
            None
        }
    }
}

/// Convert a (possibly null) Java string into an owned Rust `String`.
fn jstring_to_string(env: &mut JNIEnv, value: &jni::objects::JString) -> Option<String> {
    if value.is_null() {
        return None;
    }
    env.get_string(value).ok().map(|value| value.into())
}

/// Present a native in-app notification banner.
pub fn show_native_notification(id: u32, options: &NativeNotificationOptions) {
    let title = options.title.clone();
    let message = options.message.clone().unwrap_or_default();
    let kind = options.kind.as_i32();
    let duration_secs = options.duration_secs;
    let auto_close = options.auto_close;
    jni_call("show_native_notification", move || {
        with_main_activity("show_native_notification", |env, class| {
            let title = env.new_string(&title)?;
            let message = env.new_string(&message)?;
            env.call_static_method(
                class,
                "showNativeNotification",
                "(ILjava/lang/String;Ljava/lang/String;IFZ)V",
                &[
                    (id as jint).into(),
                    (&title).into(),
                    (&message).into(),
                    (kind as jint).into(),
                    (duration_secs as jfloat).into(),
                    auto_close.into(),
                ],
            )?;
            Ok(())
        });
    });
}

/// Request the FCM registration token for Delta push notifications.
///
/// The result is delivered asynchronously via `nativeDeltaPushTokenResult` or
/// `nativeDeltaPushTokenError`.
pub fn request_delta_push_token(id: u32) {
    jni_call("request_delta_push_token", move || {
        with_main_activity("request_delta_push_token", |env, class| {
            env.call_static_method(
                class,
                "requestDeltaPushToken",
                "(I)V",
                &[(id as jint).into()],
            )?;
            Ok(())
        });
    });
}

/// Delivered from `MainActivity` once the FCM token resolves.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_MainActivity_nativeDeltaPushTokenResult(
    mut env: JNIEnv,
    _class: JClass,
    callback_id: jint,
    provider: jni::objects::JString,
    token: jni::objects::JString,
    environment: jni::objects::JString,
) {
    if callback_id <= 0 {
        return;
    }
    let provider = jstring_to_string(&mut env, &provider).unwrap_or_else(|| "fcm".to_string());
    let token = jstring_to_string(&mut env, &token).unwrap_or_default();
    if token.is_empty() {
        platform_bridge::dispatch_delta_push_token_error(
            callback_id as u32,
            "Push registration did not return a token".to_string(),
        );
        return;
    }
    let environment = jstring_to_string(&mut env, &environment).filter(|value| !value.is_empty());
    platform_bridge::dispatch_delta_push_token_result(
        callback_id as u32,
        provider,
        token,
        environment,
    );
}

/// Delivered from `MainActivity` when FCM token registration fails.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_MainActivity_nativeDeltaPushTokenError(
    mut env: JNIEnv,
    _class: JClass,
    callback_id: jint,
    message: jni::objects::JString,
) {
    if callback_id <= 0 {
        return;
    }
    let message = jstring_to_string(&mut env, &message)
        .unwrap_or_else(|| "Push registration failed".to_string());
    platform_bridge::dispatch_delta_push_token_error(callback_id as u32, message);
}

/// Trigger Google Sign-In via the Android Credential Manager.
///
/// The result is delivered asynchronously via `nativeDeltaGoogleSignInResult`
/// or `nativeDeltaGoogleSignInError`.
pub fn start_delta_google_sign_in(id: u32) {
    jni_call("start_delta_google_sign_in", move || {
        with_main_activity("start_delta_google_sign_in", |env, class| {
            env.call_static_method(
                class,
                "startDeltaGoogleSignIn",
                "(I)V",
                &[(id as jint).into()],
            )?;
            Ok(())
        });
    });
}

/// Delivered from `MainActivity` once Google Sign-In succeeds.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_MainActivity_nativeDeltaGoogleSignInResult(
    mut env: JNIEnv,
    _class: JClass,
    callback_id: jint,
    id_token: jni::objects::JString,
    email: jni::objects::JString,
) {
    if callback_id <= 0 {
        return;
    }
    let id_token = match jstring_to_string(&mut env, &id_token) {
        Some(t) if !t.is_empty() => t,
        _ => {
            platform_bridge::dispatch_delta_google_sign_in_error(
                callback_id as u32,
                "Google sign-in did not return an ID token".to_string(),
            );
            return;
        }
    };
    let email = jstring_to_string(&mut env, &email).filter(|e| !e.is_empty());
    platform_bridge::dispatch_delta_google_sign_in_result(callback_id as u32, id_token, email);
}

/// Delivered from `MainActivity` when Google Sign-In fails.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_MainActivity_nativeDeltaGoogleSignInError(
    mut env: JNIEnv,
    _class: JClass,
    callback_id: jint,
    message: jni::objects::JString,
) {
    if callback_id <= 0 {
        return;
    }
    let message = jstring_to_string(&mut env, &message)
        .unwrap_or_else(|| "Google sign-in failed".to_string());
    platform_bridge::dispatch_delta_google_sign_in_error(callback_id as u32, message);
}

pub fn show_text_input(id: u32, title: &str, placeholder: &str, initial_value: &str) {
    let title = title.to_string();
    let placeholder = placeholder.to_string();
    let initial_value = initial_value.to_string();
    jni_call("show_text_input", move || {
        with_main_activity_class("show_text_input", |env, class| {
            let title = match env.new_string(&title) {
                Ok(value) => value,
                Err(error) => {
                    tracing::error!(?error, "jni: title string failed");
                    return;
                }
            };
            let placeholder = match env.new_string(&placeholder) {
                Ok(value) => value,
                Err(error) => {
                    tracing::error!(?error, "jni: placeholder string failed");
                    return;
                }
            };
            let initial_value = match env.new_string(&initial_value) {
                Ok(value) => value,
                Err(error) => {
                    tracing::error!(?error, "jni: initial string failed");
                    return;
                }
            };
            if let Err(error) = env.call_static_method(
                class,
                "showTextInput",
                "(ILjava/lang/String;Ljava/lang/String;Ljava/lang/String;)V",
                &[
                    (id as jint).into(),
                    (&title).into(),
                    (&placeholder).into(),
                    (&initial_value).into(),
                ],
            ) {
                tracing::error!(?error, "jni: showTextInput failed");
            }
        });
    });
}

pub fn present_custom_sheet(options: &CustomSheetOptions) {
    let detents = options
        .detents
        .iter()
        .map(|detent| detent.to_i32() as jint)
        .collect::<Vec<_>>();
    let initial_detent = options.initial_detent.to_i32() as jint;
    let shows_grabber = options.shows_grabber;
    let expands_on_scroll_edge = options.expands_on_scroll_edge;
    let modal_in_presentation = options.modal_in_presentation;
    let corner_radius = options.corner_radius.unwrap_or(-1.0);

    jni_call("present_custom_sheet", move || {
        with_main_activity_class("present_custom_sheet", |env, class| {
            let detent_array = match env.new_int_array(detents.len() as i32) {
                Ok(array) => array,
                Err(error) => {
                    tracing::error!(?error, "jni: detent array failed");
                    return;
                }
            };
            if let Err(error) = env.set_int_array_region(&detent_array, 0, &detents) {
                tracing::error!(?error, "jni: populate detents failed");
                return;
            }

            if let Err(error) = env.call_static_method(
                class,
                "presentCustomSheet",
                "([IIZZZF)V",
                &[
                    (&detent_array).into(),
                    initial_detent.into(),
                    shows_grabber.into(),
                    expands_on_scroll_edge.into(),
                    modal_in_presentation.into(),
                    corner_radius.into(),
                ],
            ) {
                tracing::error!(?error, "jni: presentCustomSheet failed");
            }
        });
    });
}

pub fn dismiss_custom_sheet() {
    jni_call("dismiss_custom_sheet", move || {
        with_main_activity_class("dismiss_custom_sheet", |env, class| {
            if let Err(error) = env.call_static_method(class, "dismissCustomSheet", "()V", &[]) {
                tracing::error!(?error, "jni: dismissCustomSheet failed");
            }
        });
    });
}

pub fn update_native_floating_button(id: u32, options: &NativeFloatingButtonOptions) {
    let image_name = options.system_image_name.clone();
    let accessibility_label = options.accessibility_label.clone();
    let x = f32::from(options.bounds.origin.x);
    let y = f32::from(options.bounds.origin.y);
    let width = f32::from(options.bounds.size.width);
    let height = f32::from(options.bounds.size.height);
    let icon_size = options.icon_size_pts;
    let icon_weight = options.icon_weight.as_i32() as jint;

    jni_call("update_native_floating_button", move || {
        with_main_activity_class("update_native_floating_button", |env, class| {
            let image_name = match env.new_string(&image_name) {
                Ok(value) => value,
                Err(error) => {
                    tracing::error!(?error, "jni: image name string failed");
                    return;
                }
            };
            let accessibility_label = match env.new_string(&accessibility_label) {
                Ok(value) => value,
                Err(error) => {
                    tracing::error!(?error, "jni: accessibility string failed");
                    return;
                }
            };
            if let Err(error) = env.call_static_method(
                class,
                "updateNativeFloatingButton",
                "(ILjava/lang/String;Ljava/lang/String;FFFFFI)V",
                &[
                    (id as jint).into(),
                    (&image_name).into(),
                    (&accessibility_label).into(),
                    x.into(),
                    y.into(),
                    width.into(),
                    height.into(),
                    icon_size.into(),
                    icon_weight.into(),
                ],
            ) {
                tracing::error!(?error, "jni: updateNativeFloatingButton failed");
            }
        });
    });
}

pub fn hide_native_floating_button(id: u32) {
    jni_call("hide_native_floating_button", move || {
        with_main_activity_class("hide_native_floating_button", |env, class| {
            if let Err(error) = env.call_static_method(
                class,
                "hideNativeFloatingButton",
                "(I)V",
                &[(id as jint).into()],
            ) {
                tracing::error!(?error, "jni: hideNativeFloatingButton failed");
            }
        });
    });
}

pub fn update_native_dictation_preview(id: u32, options: &NativeDictationPreviewOptions) {
    let text = options.text.clone();
    let bottom_offset = options.bottom_offset_pts;
    jni_call("update_native_dictation_preview", move || {
        with_main_activity_class("update_native_dictation_preview", |env, class| {
            let text = match env.new_string(&text) {
                Ok(value) => value,
                Err(error) => {
                    tracing::error!(?error, "jni: dictation string failed");
                    return;
                }
            };
            if let Err(error) = env.call_static_method(
                class,
                "updateNativeDictationPreview",
                "(ILjava/lang/String;F)V",
                &[(id as jint).into(), (&text).into(), bottom_offset.into()],
            ) {
                tracing::error!(?error, "jni: updateNativeDictationPreview failed");
            }
        });
    });
}

pub fn hide_native_dictation_preview(id: u32) {
    jni_call("hide_native_dictation_preview", move || {
        with_main_activity_class("hide_native_dictation_preview", |env, class| {
            if let Err(error) = env.call_static_method(
                class,
                "hideNativeDictationPreview",
                "(I)V",
                &[(id as jint).into()],
            ) {
                tracing::error!(?error, "jni: hideNativeDictationPreview failed");
            }
        });
    });
}

pub fn present_native_notification(id: u32, options: &NativeNotificationOptions) {
    let title = options.title.clone();
    let message = options.message.clone().unwrap_or_default();
    let image_name = options.image_name.clone().unwrap_or_default();
    let kind = options.kind.as_i32() as jint;
    let duration_secs = options.duration_secs;
    let auto_close = options.auto_close;

    jni_call("present_native_notification", move || {
        with_main_activity_class("present_native_notification", |env, class| {
            let title = match env.new_string(&title) {
                Ok(value) => value,
                Err(error) => {
                    tracing::error!(?error, "jni: notification title failed");
                    return;
                }
            };
            let message = match env.new_string(&message) {
                Ok(value) => value,
                Err(error) => {
                    tracing::error!(?error, "jni: notification message failed");
                    return;
                }
            };
            let image_name = match env.new_string(&image_name) {
                Ok(value) => value,
                Err(error) => {
                    tracing::error!(?error, "jni: notification image failed");
                    return;
                }
            };
            if let Err(error) = env.call_static_method(
                class,
                "showNativeNotification",
                "(ILjava/lang/String;Ljava/lang/String;Ljava/lang/String;IFZ)V",
                &[
                    (id as jint).into(),
                    (&title).into(),
                    (&message).into(),
                    (&image_name).into(),
                    kind.into(),
                    duration_secs.into(),
                    auto_close.into(),
                ],
            ) {
                tracing::error!(?error, "jni: showNativeNotification failed");
            }
        });
    });
}

pub fn trigger_haptic(feedback: HapticFeedback) {
    let kind = feedback.to_i32();
    jni_call("trigger_haptic", move || {
        with_main_activity_class("trigger_haptic", |env, class| {
            if let Err(e) = env.call_static_method(class, "triggerHaptic", "(I)V", &[(kind).into()])
            {
                tracing::error!("jni: triggerHaptic failed: {:?}", e);
            }
        });
    });
}

pub fn play_sound(sound: SoundEffect) {
    let kind = sound.to_i32();
    jni_call("play_sound", move || {
        with_main_activity_class("play_sound", |env, class| {
            if let Err(e) = env.call_static_method(class, "playSound", "(I)V", &[(kind).into()]) {
                tracing::error!("jni: playSound failed: {:?}", e);
            }
        });
    });
}

pub fn system_prefers_theme() -> SystemTheme {
    let mut theme = SystemTheme::Unknown;
    jni_call(
        "system_prefers_theme",
        std::panic::AssertUnwindSafe(|| {
            with_main_activity_class("system_prefers_theme", |env, class| {
                let Ok(value) = env.call_static_method(class, "systemInDarkTheme", "()I", &[])
                else {
                    return;
                };
                let Ok(code) = value.i() else {
                    return;
                };
                theme = match code {
                    1 => SystemTheme::Dark,
                    0 => SystemTheme::Light,
                    _ => SystemTheme::Unknown,
                };
            });
        }),
    );
    theme
}

pub fn set_native_theme(is_dark: bool) {
    jni_call("set_native_theme", move || {
        with_main_activity_class("set_native_theme", |env, class| {
            if let Err(error) =
                env.call_static_method(class, "setNativeTheme", "(Z)V", &[is_dark.into()])
            {
                tracing::error!(?error, "jni: setNativeTheme failed");
            }
        });
    });
}

// Suppress unused-import warning for GlobalRef, kept for potential future use.
#[allow(dead_code)]
fn _gref_marker(_: GlobalRef) {}
