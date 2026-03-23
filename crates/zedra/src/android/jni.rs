use jni::{
    JNIEnv, JavaVM,
    objects::{GlobalRef, JClass, JObject},
    sys::{jfloat, jint, jlong},
};
use ndk::native_window::NativeWindow;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, Once};

use crate::android::{
    app,
    command_queue::{AndroidCommand, get_command_sender},
};
use crate::install_panic_hook;
use crate::platform_bridge::{self, AlertButton, AlertButtonStyle};

// Global storage for JavaVM to enable Rust→Java callbacks
static JVM: Mutex<Option<Arc<JavaVM>>> = Mutex::new(None);

// Static initialization for logging
static INIT: Once = Once::new();

// Global storage for the NativeWindow (only accessed from main thread)
static NATIVE_WINDOW: Mutex<Option<NativeWindow>> = Mutex::new(None);

// Global storage for display density (set from Java, read by Rust)
static DISPLAY_DENSITY: Mutex<f32> = Mutex::new(3.0);

// Global storage for soft keyboard height in physical pixels (0 = hidden)
static KEYBOARD_HEIGHT: AtomicU32 = AtomicU32::new(0);

// Global storage for system bar insets in physical pixels
static SYSTEM_INSET_TOP: AtomicU32 = AtomicU32::new(0);
static SYSTEM_INSET_BOTTOM: AtomicU32 = AtomicU32::new(0);

// Guard against gpuiDestroy being called more than once (would double-free the Arc).
static DESTROYED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

// Global storage for the app's internal files directory
static FILES_DIR: Mutex<Option<String>> = Mutex::new(None);

/// Get the stored display density
pub fn get_density() -> f32 {
    *DISPLAY_DENSITY.lock().unwrap()
}

/// Get the current soft keyboard height in physical pixels (0 = hidden)
pub fn get_keyboard_height() -> u32 {
    KEYBOARD_HEIGHT.load(Ordering::Relaxed)
}

/// Get the system status bar inset in physical pixels
pub fn get_system_inset_top() -> u32 {
    SYSTEM_INSET_TOP.load(Ordering::Relaxed)
}

/// Get the system navigation bar inset in physical pixels
pub fn get_system_inset_bottom() -> u32 {
    SYSTEM_INSET_BOTTOM.load(Ordering::Relaxed)
}

/// Get the app's internal files directory (set during gpuiInit).
pub fn get_files_dir() -> Option<String> {
    FILES_DIR.lock().ok()?.clone()
}

/// Get the stored NativeWindow (must be called from main thread)
pub fn take_native_window() -> Option<NativeWindow> {
    NATIVE_WINDOW.lock().unwrap().take()
}

/// Internal handle to the Android platform
struct AndroidPlatformHandle {
    _jvm: Arc<JavaVM>,
    _activity: Arc<Mutex<GlobalRef>>,
}

/// Initialize logging and panic hook for Android
fn init_logging() {
    INIT.call_once(|| {
        android_logger::init_once(
            android_logger::Config::default()
                .with_max_level(log::LevelFilter::Info)
                .with_tag("zedra")
                .with_filter(
                    android_logger::FilterBuilder::new()
                        .parse("info,tracing::span=off,tracing::span::active=off")
                        .build(),
                ),
        );

        install_panic_hook();
    });
}

/// Initialize the GPUI Android platform
///
/// Called from MainActivity.onCreate()
///
/// # Arguments
/// * `env` - JNI environment
/// * `activity` - The Android Activity object
///
/// # Returns
/// Platform handle as jlong (pointer) or 0 on failure
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_MainActivity_gpuiInit(
    env: JNIEnv,
    _class: JClass,
    activity: JObject,
) -> jlong {
    init_logging();

    // Get the JavaVM
    let jvm = match env.get_java_vm() {
        Ok(vm) => Arc::new(vm),
        Err(e) => {
            tracing::error!("Failed to get JavaVM: {:?}", e);
            return 0;
        }
    };

    // Store JVM globally for Rust→Java callbacks (keyboard, etc.)
    *JVM.lock().unwrap() = Some(jvm.clone());

    // Create a global reference to the activity
    let activity_ref = match env.new_global_ref(activity) {
        Ok(r) => Arc::new(Mutex::new(r)),
        Err(e) => {
            tracing::error!("Failed to create global ref to activity: {:?}", e);
            return 0;
        }
    };

    // Fetch and store the app's internal files directory
    {
        let files_dir_obj = env.call_method(&activity, "getFilesDir", "()Ljava/io/File;", &[]);
        if let Ok(jni::objects::JValueGen::Object(file_obj)) = files_dir_obj {
            let path_obj =
                env.call_method(&file_obj, "getAbsolutePath", "()Ljava/lang/String;", &[]);
            if let Ok(jni::objects::JValueGen::Object(path_str)) = path_obj {
                let jstr = jni::objects::JString::from(path_str);
                if let Ok(path) = env.get_string(&jstr) {
                    let path: String = path.into();
                    tracing::debug!(path = %path, "jni: files dir");
                    *FILES_DIR.lock().unwrap() = Some(path);
                }
            }
        }
    }

    // Create the platform handle
    let handle = Arc::new(AndroidPlatformHandle {
        _jvm: jvm.clone(),
        _activity: activity_ref.clone(),
    });

    // Send initialize command to the queue (will be processed on main thread)
    let sender = get_command_sender();
    if let Err(e) = sender.send(AndroidCommand::Initialize {
        jvm: jvm.clone(),
        activity: activity_ref.clone(),
    }) {
        tracing::error!("Failed to send Initialize command: {:?}", e);
        return 0;
    }

    // Return the handle pointer
    let handle_ptr = Arc::into_raw(handle) as jlong;
    tracing::debug!(handle = handle_ptr, "jni: init");
    handle_ptr
}

/// Initialize the AndroidApp on the main thread (called once)
///
/// This must be called from the main UI thread before processing commands
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_MainActivity_gpuiInitMainThread(
    _env: JNIEnv,
    _class: JClass,
) {
    tracing::debug!("jni: init main thread");
    app::init_android_app();
}

/// Process commands from the queue on the main thread
///
/// This should be called periodically from the main UI thread (e.g., via Choreographer)
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_MainActivity_gpuiProcessCommands(
    _env: JNIEnv,
    _class: JClass,
) {
    // Process all pending commands
    if let Err(e) = app::process_commands_from_queue() {
        tracing::error!("Error processing commands: {:?}", e);
    }
}

/// Process critical initialization commands immediately (not waiting for Choreographer)
///
/// This processes Initialize and SurfaceCreated commands immediately for faster startup.
/// Call this right after gpuiInit() and after surface creation to reduce rendering delay.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_MainActivity_gpuiProcessCriticalCommands(
    _env: JNIEnv,
    _class: JClass,
) {
    if let Err(e) = app::process_commands_from_queue() {
        tracing::error!("jni: process critical commands failed: {:?}", e);
    }
}

/// Cleanup and destroy the GPUI platform
///
/// Called from MainActivity.onDestroy()
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_MainActivity_gpuiDestroy(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) {
    if handle == 0 {
        tracing::warn!("jni: destroy called with null handle");
        return;
    }

    // Send destroy command to queue
    let sender = get_command_sender();
    if let Err(e) = sender.send(AndroidCommand::Destroy) {
        tracing::error!("jni: send Destroy failed: {:?}", e);
    }

    // Guard against double-free: Arc::from_raw on the same pointer twice is UB.
    if DESTROYED.swap(true, Ordering::SeqCst) {
        tracing::warn!("jni: destroy called more than once — skipping Arc drop");
        return;
    }

    tracing::debug!("jni: destroy");

    // SAFETY: `handle` was produced by Arc::into_raw in gpuiCreate and has not
    // been reconstructed before (DESTROYED flag ensures single call).
    unsafe {
        let _ = Arc::from_raw(handle as *const AndroidPlatformHandle);
    }
}

/// Surface created callback
///
/// Called when the Android Surface is created and ready for rendering
///
/// # Arguments
/// * `surface` - The Android Surface object (will be converted to ANativeWindow)
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_GpuiSurfaceView_nativeSurfaceCreated(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    surface: JObject,
) {
    if handle == 0 {
        tracing::error!("jni: surface created with null handle");
        return;
    }

    // Get ANativeWindow from Surface
    let native_window = match unsafe { NativeWindow::from_surface(env.get_raw(), surface.as_raw()) }
    {
        Some(window) => window,
        None => {
            tracing::error!("jni: failed to get ANativeWindow from Surface");
            return;
        }
    };

    let width = native_window.width() as u32;
    let height = native_window.height() as u32;

    // Store the native window globally so it can be retrieved when processing the command
    *NATIVE_WINDOW.lock().unwrap() = Some(native_window);

    // Send command to queue
    let sender = get_command_sender();
    if let Err(e) = sender.send(AndroidCommand::SurfaceCreated { width, height }) {
        tracing::error!("jni: send SurfaceCreated failed: {:?}", e);
    }
}

/// Process surface commands immediately (called from GpuiSurfaceView callbacks)
///
/// This allows immediate processing of surface creation/changes for faster initial render
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_GpuiSurfaceView_nativeProcessSurfaceCommands(
    _env: JNIEnv,
    _class: JClass,
) {
    if let Err(e) = app::process_commands_from_queue() {
        tracing::error!("Error processing surface commands: {:?}", e);
    }
}

/// Surface changed callback
///
/// Called when the surface size or format changes (e.g., rotation)
///
/// # Arguments
/// * `format` - The new surface format
/// * `width` - The new surface width
/// * `height` - The new surface height
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_GpuiSurfaceView_nativeSurfaceChanged(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
    format: jint,
    width: jint,
    height: jint,
) {
    if handle == 0 {
        tracing::error!("jni: surface changed with null handle");
        return;
    }

    tracing::debug!(width, height, format, "jni: surface changed");

    // Send command to queue
    let sender = get_command_sender();
    if let Err(e) = sender.send(AndroidCommand::SurfaceChanged {
        width: width as u32,
        height: height as u32,
    }) {
        tracing::error!("jni: send SurfaceChanged failed: {:?}", e);
    }
}

/// Surface destroyed callback
///
/// Called when the surface is destroyed and can no longer be used for rendering
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_GpuiSurfaceView_nativeSurfaceDestroyed(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) {
    if handle == 0 {
        tracing::error!("jni: surface destroyed with null handle");
        return;
    }

    tracing::debug!("jni: surface destroyed");

    // Send command to queue
    let sender = get_command_sender();
    if let Err(e) = sender.send(AndroidCommand::SurfaceDestroyed) {
        tracing::error!("jni: send SurfaceDestroyed failed: {:?}", e);
    }
}

/// Touch event callback
///
/// Called when a touch event occurs
///
/// # Arguments
/// * `action` - The touch action (DOWN=0, UP=1, MOVE=2, CANCEL=3)
/// * `x` - The X coordinate
/// * `y` - The Y coordinate
/// * `pointer_id` - The pointer ID for multi-touch
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_GpuiSurfaceView_nativeTouchEvent(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
    action: jint,
    x: jfloat,
    y: jfloat,
    pointer_id: jint,
) {
    if handle == 0 {
        return;
    }

    tracing::trace!(action, x, y, pointer_id, "jni: touch");

    // Send command to queue
    let sender = get_command_sender();
    let _ = sender.send(AndroidCommand::Touch {
        action,
        x,
        y,
        pointer_id,
    });
}

/// Key event callback
///
/// Called when a hardware key event occurs
///
/// # Arguments
/// * `action` - The key action (DOWN=0, UP=1)
/// * `key_code` - The Android KeyCode
/// * `unicode` - The unicode character (0 if none)
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_GpuiSurfaceView_nativeKeyEvent(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
    action: jint,
    key_code: jint,
    unicode: jint,
) {
    if handle == 0 {
        return;
    }

    tracing::debug!(action, key_code, unicode, "jni: key event");

    // Send command to queue
    let sender = get_command_sender();
    let _ = sender.send(AndroidCommand::Key {
        action,
        key_code,
        unicode,
    });
}

/// Resume callback
///
/// Called when the activity resumes
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_MainActivity_gpuiResume(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) {
    if handle == 0 {
        tracing::error!("jni: resume called with null handle");
        return;
    }

    tracing::debug!("jni: resume");

    // Send command to queue
    let sender = get_command_sender();
    if let Err(e) = sender.send(AndroidCommand::Resume) {
        tracing::error!("jni: send Resume failed: {:?}", e);
    }
}

/// Pause callback
///
/// Called when the activity pauses
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_MainActivity_gpuiPause(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) {
    if handle == 0 {
        tracing::error!("jni: pause called with null handle");
        return;
    }

    tracing::debug!("jni: pause");

    // Send command to queue
    let sender = get_command_sender();
    if let Err(e) = sender.send(AndroidCommand::Pause) {
        tracing::error!("jni: send Pause failed: {:?}", e);
    }
}

/// Get display density
///
/// Returns the display density scale factor
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_MainActivity_getDisplayDensity(
    mut env: JNIEnv,
    _class: JClass,
    activity: JObject,
) -> jfloat {
    // Get DisplayMetrics via JNI
    let result: Result<jfloat, jni::errors::Error> = (|| {
        // Get the WindowManager
        let window_manager = env.call_method(
            activity,
            "getWindowManager",
            "()Landroid/view/WindowManager;",
            &[],
        )?;
        let window_manager = window_manager.l()?;

        // Get the default Display
        let display = env.call_method(
            window_manager,
            "getDefaultDisplay",
            "()Landroid/view/Display;",
            &[],
        )?;
        let display = display.l()?;

        // Create DisplayMetrics object
        let metrics_class = env.find_class("android/util/DisplayMetrics")?;
        let metrics = env.new_object(metrics_class, "()V", &[])?;

        // Get metrics from display
        env.call_method(
            display,
            "getMetrics",
            "(Landroid/util/DisplayMetrics;)V",
            &[(&metrics).into()],
        )?;

        // Read the density field
        let density = env.get_field(metrics, "density", "F")?;
        Ok(density.f()?)
    })();

    match result {
        Ok(density) => {
            tracing::debug!(density, "jni: display density");
            *DISPLAY_DENSITY.lock().unwrap() = density;
            // Also update the terminal crate's global for keyboard resize calculations
            zedra_terminal::set_display_density(density);
            density
        }
        Err(e) => {
            tracing::error!("Failed to get display density: {:?}", e);
            1.0 // Default density
        }
    }
}

/// QR code scanned callback from QRScannerActivity
///
/// Called when a zedra:// QR code is successfully scanned
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_QRScannerActivity_nativeOnQrCodeScanned(
    mut env: JNIEnv,
    _class: JClass,
    data: jni::objects::JString,
) {
    let qr_data: String = match env.get_string(&data) {
        Ok(s) => s.into(),
        Err(e) => {
            tracing::error!("Failed to get QR data string: {:?}", e);
            return;
        }
    };

    tracing::info!(url = &qr_data[..qr_data.len().min(50)], "jni: qr scanned");

    // Route through the unified deeplink path
    let sender = get_command_sender();
    let _ = sender.send(AndroidCommand::Deeplink { url: qr_data });
}

/// Deeplink received callback
///
/// Called when the app is opened via a zedra:// URL (system intent)
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_MainActivity_nativeDeeplinkReceived(
    mut env: JNIEnv,
    _class: JClass,
    url: jni::objects::JString,
) {
    let deeplink_url: String = match env.get_string(&url) {
        Ok(s) => s.into(),
        Err(e) => {
            tracing::error!("Failed to get deeplink URL string: {:?}", e);
            return;
        }
    };

    tracing::info!(
        url = &deeplink_url[..deeplink_url.len().min(80)],
        "jni: deeplink"
    );

    let sender = get_command_sender();
    let _ = sender.send(AndroidCommand::Deeplink { url: deeplink_url });
}

/// Native alert result callback from MainActivity.
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
    zedra_session::push_callback(Box::new(|| {}));
}

/// Native alert dismiss callback from MainActivity.
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
    zedra_session::push_callback(Box::new(|| {}));
}

/// Native selection result callback from MainActivity.
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
    zedra_session::push_callback(Box::new(|| {}));
}

/// Native selection dismiss callback from MainActivity.
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
    zedra_session::push_callback(Box::new(|| {}));
}

/// Fling event callback
///
/// Called when a fling gesture is detected (velocity from Android VelocityTracker)
///
/// # Arguments
/// * `velocity_x` - Horizontal velocity in pixels/second
/// * `velocity_y` - Vertical velocity in pixels/second
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_GpuiSurfaceView_nativeFlingEvent(
    _env: JNIEnv,
    _class: JClass,
    _handle: jlong,
    velocity_x: jfloat,
    velocity_y: jfloat,
) {
    tracing::debug!(velocity_x, velocity_y, "jni: fling");

    let sender = get_command_sender();
    let _ = sender.send(AndroidCommand::Fling {
        velocity_x,
        velocity_y,
    });
}

/// System insets changed callback
///
/// Called when the system bar insets change (status bar top, navigation bar bottom).
/// Values are in physical pixels.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_GpuiSurfaceView_nativeSystemInsetsChanged(
    _env: JNIEnv,
    _class: JClass,
    top: jint,
    bottom: jint,
) {
    let t = top.max(0) as u32;
    let b = bottom.max(0) as u32;
    SYSTEM_INSET_TOP.store(t, Ordering::Relaxed);
    SYSTEM_INSET_BOTTOM.store(b, Ordering::Relaxed);
    tracing::debug!(top = t, bottom = b, "jni: system insets");
}

/// Keyboard height changed callback
///
/// Called when the soft keyboard appears, resizes, or disappears.
/// Height is in physical pixels (0 = keyboard hidden).
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_GpuiSurfaceView_nativeKeyboardHeightChanged(
    _env: JNIEnv,
    _class: JClass,
    height: jint,
) {
    let h = height.max(0) as u32;
    KEYBOARD_HEIGHT.store(h, Ordering::Relaxed);
    // Also update the terminal crate's global so TerminalView can read it
    zedra_terminal::set_keyboard_height(h);
    tracing::debug!(height = h, "jni: keyboard height");

    let sender = get_command_sender();
    let _ = sender.send(AndroidCommand::KeyboardHeightChanged { height: h });
}

/// Show soft keyboard
///
/// Called to show the Android soft keyboard for terminal input
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_GpuiSurfaceView_nativeRequestShowKeyboard(
    _env: JNIEnv,
    _class: JClass,
    _handle: jlong,
) {
    // The Java side handles actually showing the keyboard via InputMethodManager
}

/// Hide soft keyboard
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_GpuiSurfaceView_nativeRequestHideKeyboard(
    _env: JNIEnv,
    _class: JClass,
    _handle: jlong,
) {
}

/// IME text input callback
///
/// Called when text is entered via the soft keyboard IME
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_GpuiSurfaceView_nativeImeInput(
    mut env: JNIEnv,
    _class: JClass,
    _handle: jlong,
    text: jni::objects::JString,
) {
    let input: String = match env.get_string(&text) {
        Ok(s) => s.into(),
        Err(e) => {
            tracing::error!("Failed to get IME text: {:?}", e);
            return;
        }
    };

    tracing::debug!(len = input.len(), "jni: ime input");

    // Send entire text as a single ImeText command to avoid reentrancy issues
    let sender = get_command_sender();
    let _ = sender.send(AndroidCommand::ImeText { text: input });
}

// =============================================================================
// Public Rust API for keyboard control (Rust → Java callbacks)
// =============================================================================

/// Wrap `f` in `catch_unwind` and log any panic as an error.
///
/// Prevents Rust panics from propagating through the JNI boundary (undefined
/// behaviour if a Rust panic unwinds into C/Java frames).
fn jni_call(name: &'static str, f: impl FnOnce() + std::panic::UnwindSafe) {
    if let Err(e) = std::panic::catch_unwind(f) {
        tracing::error!(name, err = ?e, "jni: panic");
    }
}

/// Show the Android soft keyboard
///
/// Call this when a text input gains focus
pub fn show_keyboard() {
    jni_call("show_keyboard", show_keyboard_inner);
}

fn show_keyboard_inner() {
    let jvm = match JVM.lock() {
        Ok(guard) => match guard.as_ref() {
            Some(jvm) => jvm.clone(),
            None => {
                tracing::error!("JVM not available for keyboard call");
                return;
            }
        },
        Err(e) => {
            tracing::error!("Failed to lock JVM mutex: {:?}", e);
            return;
        }
    };

    // Use get_env for the main thread which should already be attached
    let mut env = match jvm.get_env() {
        Ok(env) => env,
        Err(_) => {
            // Fall back to attach if not already attached
            match jvm.attach_current_thread_as_daemon() {
                Ok(env) => env,
                Err(e) => {
                    tracing::error!("Failed to attach thread for keyboard: {:?}", e);
                    return;
                }
            }
        }
    };

    // Call MainActivity.showKeyboard() static method
    let class = match env.find_class("dev/zedra/app/MainActivity") {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to find MainActivity class: {:?}", e);
            // Check for pending exception
            if env.exception_check().unwrap_or(false) {
                env.exception_describe().ok();
                env.exception_clear().ok();
            }
            return;
        }
    };

    if let Err(e) = env.call_static_method(&class, "showKeyboard", "()V", &[]) {
        tracing::error!("jni: showKeyboard failed: {:?}", e);
        if env.exception_check().unwrap_or(false) {
            env.exception_describe().ok();
            env.exception_clear().ok();
        }
    } else {
        tracing::debug!("jni: show keyboard");
    }
}

/// Launch the QR scanner activity
///
/// Call this to open the camera for QR code scanning
pub fn launch_qr_scanner() {
    jni_call("launch_qr_scanner", launch_qr_scanner_inner);
}

fn launch_qr_scanner_inner() {
    let jvm = match JVM.lock() {
        Ok(guard) => match guard.as_ref() {
            Some(jvm) => jvm.clone(),
            None => {
                tracing::error!("JVM not available for QR scanner");
                return;
            }
        },
        Err(e) => {
            tracing::error!("Failed to lock JVM mutex: {:?}", e);
            return;
        }
    };

    let mut env = match jvm.get_env() {
        Ok(env) => env,
        Err(_) => match jvm.attach_current_thread_as_daemon() {
            Ok(env) => env,
            Err(e) => {
                tracing::error!("Failed to attach thread for QR scanner: {:?}", e);
                return;
            }
        },
    };

    let class = match env.find_class("dev/zedra/app/MainActivity") {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to find MainActivity class: {:?}", e);
            if env.exception_check().unwrap_or(false) {
                env.exception_describe().ok();
                env.exception_clear().ok();
            }
            return;
        }
    };

    if let Err(e) = env.call_static_method(&class, "launchQrScanner", "()V", &[]) {
        tracing::error!("jni: launchQrScanner failed: {:?}", e);
        if env.exception_check().unwrap_or(false) {
            env.exception_describe().ok();
            env.exception_clear().ok();
        }
    } else {
        tracing::debug!("jni: launch qr scanner");
    }
}

/// Open a URL in the system browser
pub fn open_url(url: &str) {
    let url_owned = url.to_string();
    jni_call("open_url", move || open_url_inner(url_owned));
}

fn open_url_inner(url: String) {
    let jvm = match JVM.lock() {
        Ok(guard) => match guard.as_ref() {
            Some(jvm) => jvm.clone(),
            None => {
                tracing::error!("JVM not available for open_url");
                return;
            }
        },
        Err(e) => {
            tracing::error!("Failed to lock JVM mutex: {:?}", e);
            return;
        }
    };

    let mut env = match jvm.get_env() {
        Ok(env) => env,
        Err(_) => match jvm.attach_current_thread_as_daemon() {
            Ok(env) => env,
            Err(e) => {
                tracing::error!("Failed to attach thread for open_url: {:?}", e);
                return;
            }
        },
    };

    let class = match env.find_class("dev/zedra/app/MainActivity") {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to find MainActivity class: {:?}", e);
            if env.exception_check().unwrap_or(false) {
                env.exception_describe().ok();
                env.exception_clear().ok();
            }
            return;
        }
    };

    let j_url = match env.new_string(&url) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to create JString for URL: {:?}", e);
            return;
        }
    };

    if let Err(e) = env.call_static_method(
        &class,
        "openUrl",
        "(Ljava/lang/String;)V",
        &[(&j_url).into()],
    ) {
        tracing::error!("Failed to call openUrl: {:?}", e);
        if env.exception_check().unwrap_or(false) {
            env.exception_describe().ok();
            env.exception_clear().ok();
        }
    }
}

/// Present a native Android alert dialog.
pub fn show_alert(id: u32, title: &str, message: &str, buttons: &[AlertButton]) {
    let title = title.to_string();
    let message = message.to_string();
    let button_labels: Vec<String> = buttons.iter().map(|button| button.label.clone()).collect();
    let button_styles: Vec<jint> = buttons
        .iter()
        .map(|button| match button.style {
            AlertButtonStyle::Default => 0,
            AlertButtonStyle::Cancel => 1,
            AlertButtonStyle::Destructive => 2,
        })
        .collect();
    jni_call("show_alert", move || {
        show_alert_inner(id, title, message, button_labels, button_styles)
    });
}

/// Present a native Android selection sheet.
pub fn show_selection(id: u32, title: &str, message: &str, buttons: &[AlertButton]) {
    let title = title.to_string();
    let message = message.to_string();
    let button_labels: Vec<String> = buttons.iter().map(|button| button.label.clone()).collect();
    let button_styles: Vec<jint> = buttons
        .iter()
        .map(|button| match button.style {
            AlertButtonStyle::Default => 0,
            AlertButtonStyle::Cancel => 1,
            AlertButtonStyle::Destructive => 2,
        })
        .collect();
    jni_call("show_selection", move || {
        show_selection_inner(id, title, message, button_labels, button_styles)
    });
}

fn show_alert_inner(
    id: u32,
    title: String,
    message: String,
    button_labels: Vec<String>,
    button_styles: Vec<jint>,
) {
    let jvm = match JVM.lock() {
        Ok(guard) => match guard.as_ref() {
            Some(jvm) => jvm.clone(),
            None => {
                tracing::error!("JVM not available for alert call");
                return;
            }
        },
        Err(error) => {
            tracing::error!("Failed to lock JVM mutex: {:?}", error);
            return;
        }
    };

    let mut env = match jvm.get_env() {
        Ok(env) => env,
        Err(_) => match jvm.attach_current_thread_as_daemon() {
            Ok(env) => env,
            Err(error) => {
                tracing::error!("Failed to attach thread for alert: {:?}", error);
                return;
            }
        },
    };

    let class = match env.find_class("dev/zedra/app/MainActivity") {
        Ok(class) => class,
        Err(error) => {
            tracing::error!("Failed to find MainActivity class: {:?}", error);
            if env.exception_check().unwrap_or(false) {
                env.exception_describe().ok();
                env.exception_clear().ok();
            }
            return;
        }
    };

    let title_value = match env.new_string(&title) {
        Ok(value) => value,
        Err(error) => {
            tracing::error!("Failed to create alert title string: {:?}", error);
            return;
        }
    };
    let message_value = match env.new_string(&message) {
        Ok(value) => value,
        Err(error) => {
            tracing::error!("Failed to create alert message string: {:?}", error);
            return;
        }
    };
    let string_class = match env.find_class("java/lang/String") {
        Ok(class) => class,
        Err(error) => {
            tracing::error!("Failed to find String class: {:?}", error);
            return;
        }
    };
    let label_array =
        match env.new_object_array(button_labels.len() as i32, string_class, JObject::null()) {
            Ok(array) => array,
            Err(error) => {
                tracing::error!("Failed to create alert label array: {:?}", error);
                return;
            }
        };
    for (index, label) in button_labels.iter().enumerate() {
        let label_value = match env.new_string(label) {
            Ok(value) => value,
            Err(error) => {
                tracing::error!("Failed to create alert label string: {:?}", error);
                return;
            }
        };
        if let Err(error) = env.set_object_array_element(&label_array, index as i32, &label_value) {
            tracing::error!("Failed to populate alert label array: {:?}", error);
            return;
        }
    }

    let style_array = match env.new_int_array(button_styles.len() as i32) {
        Ok(array) => array,
        Err(error) => {
            tracing::error!("Failed to create alert style array: {:?}", error);
            return;
        }
    };
    if let Err(error) = env.set_int_array_region(&style_array, 0, &button_styles) {
        tracing::error!("Failed to populate alert style array: {:?}", error);
        return;
    }

    if let Err(error) = env.call_static_method(
        &class,
        "showAlert",
        "(ILjava/lang/String;Ljava/lang/String;[Ljava/lang/String;[I)V",
        &[
            (id as jint).into(),
            (&title_value).into(),
            (&message_value).into(),
            (&label_array).into(),
            (&style_array).into(),
        ],
    ) {
        tracing::error!("Failed to call showAlert: {:?}", error);
        if env.exception_check().unwrap_or(false) {
            env.exception_describe().ok();
            env.exception_clear().ok();
        }
    }
}

fn show_selection_inner(
    id: u32,
    title: String,
    message: String,
    button_labels: Vec<String>,
    button_styles: Vec<jint>,
) {
    let jvm = match JVM.lock() {
        Ok(guard) => match guard.as_ref() {
            Some(jvm) => jvm.clone(),
            None => {
                tracing::error!("JVM not available for selection call");
                return;
            }
        },
        Err(error) => {
            tracing::error!("Failed to lock JVM mutex: {:?}", error);
            return;
        }
    };

    let mut env = match jvm.get_env() {
        Ok(env) => env,
        Err(_) => match jvm.attach_current_thread_as_daemon() {
            Ok(env) => env,
            Err(error) => {
                tracing::error!("Failed to attach thread for selection: {:?}", error);
                return;
            }
        },
    };

    let class = match env.find_class("dev/zedra/app/MainActivity") {
        Ok(class) => class,
        Err(error) => {
            tracing::error!("Failed to find MainActivity class: {:?}", error);
            if env.exception_check().unwrap_or(false) {
                env.exception_describe().ok();
                env.exception_clear().ok();
            }
            return;
        }
    };

    let title_value = match env.new_string(&title) {
        Ok(value) => value,
        Err(error) => {
            tracing::error!("Failed to create selection title string: {:?}", error);
            return;
        }
    };
    let message_value = match env.new_string(&message) {
        Ok(value) => value,
        Err(error) => {
            tracing::error!("Failed to create selection message string: {:?}", error);
            return;
        }
    };
    let string_class = match env.find_class("java/lang/String") {
        Ok(class) => class,
        Err(error) => {
            tracing::error!("Failed to find String class: {:?}", error);
            return;
        }
    };
    let label_array =
        match env.new_object_array(button_labels.len() as i32, string_class, JObject::null()) {
            Ok(array) => array,
            Err(error) => {
                tracing::error!("Failed to create selection label array: {:?}", error);
                return;
            }
        };
    for (index, label) in button_labels.iter().enumerate() {
        let label_value = match env.new_string(label) {
            Ok(value) => value,
            Err(error) => {
                tracing::error!("Failed to create selection label string: {:?}", error);
                return;
            }
        };
        if let Err(error) = env.set_object_array_element(&label_array, index as i32, &label_value) {
            tracing::error!("Failed to populate selection label array: {:?}", error);
            return;
        }
    }

    let style_array = match env.new_int_array(button_styles.len() as i32) {
        Ok(array) => array,
        Err(error) => {
            tracing::error!("Failed to create selection style array: {:?}", error);
            return;
        }
    };
    if let Err(error) = env.set_int_array_region(&style_array, 0, &button_styles) {
        tracing::error!("Failed to populate selection style array: {:?}", error);
        return;
    }

    if let Err(error) = env.call_static_method(
        &class,
        "showSelection",
        "(ILjava/lang/String;Ljava/lang/String;[Ljava/lang/String;[I)V",
        &[
            (id as jint).into(),
            (&title_value).into(),
            (&message_value).into(),
            (&label_array).into(),
            (&style_array).into(),
        ],
    ) {
        tracing::error!("Failed to call showSelection: {:?}", error);
        if env.exception_check().unwrap_or(false) {
            env.exception_describe().ok();
            env.exception_clear().ok();
        }
    }
}

/// Hide the Android soft keyboard
///
/// Call this when a text input loses focus
pub fn hide_keyboard() {
    jni_call("hide_keyboard", hide_keyboard_inner);
}

fn hide_keyboard_inner() {
    let jvm = match JVM.lock() {
        Ok(guard) => match guard.as_ref() {
            Some(jvm) => jvm.clone(),
            None => {
                tracing::error!("JVM not available for keyboard call");
                return;
            }
        },
        Err(e) => {
            tracing::error!("Failed to lock JVM mutex: {:?}", e);
            return;
        }
    };

    // Use get_env for the main thread which should already be attached
    let mut env = match jvm.get_env() {
        Ok(env) => env,
        Err(_) => {
            // Fall back to attach if not already attached
            match jvm.attach_current_thread_as_daemon() {
                Ok(env) => env,
                Err(e) => {
                    tracing::error!("Failed to attach thread for keyboard: {:?}", e);
                    return;
                }
            }
        }
    };

    // Call MainActivity.hideKeyboard() static method
    let class = match env.find_class("dev/zedra/app/MainActivity") {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to find MainActivity class: {:?}", e);
            // Check for pending exception
            if env.exception_check().unwrap_or(false) {
                env.exception_describe().ok();
                env.exception_clear().ok();
            }
            return;
        }
    };

    if let Err(e) = env.call_static_method(&class, "hideKeyboard", "()V", &[]) {
        tracing::error!("jni: hideKeyboard failed: {:?}", e);
        if env.exception_check().unwrap_or(false) {
            env.exception_describe().ok();
            env.exception_clear().ok();
        }
    } else {
        tracing::debug!("jni: hide keyboard");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jni_exports_exist() {
        // This test just verifies the JNI exports compile
        // Actual testing requires a JVM
    }
}
