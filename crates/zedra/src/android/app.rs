/// Android GPUI Application Bridge
///
/// This module provides the main-thread-only interface for running GPUI apps on Android.
/// It processes commands from the thread-safe queue and manages the GPUI App lifecycle.
use anyhow::Result;
use gpui::*;
use gpui_android::AndroidPlatform;
use jni::{JavaVM, objects::GlobalRef};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use crate::android::{bridge::AndroidBridge, command_queue, command_queue::AndroidCommand, jni};
use crate::{app, deeplink, platform_bridge};
use crate::ZedraAssets;

/// Android app state - must only be accessed from the main UI thread
pub struct AndroidApp {
    /// Whether the platform has been initialized
    platform_initialized: bool,
    /// Reference to the AndroidPlatform for triggering frame requests
    platform: Option<Rc<AndroidPlatform>>,
    /// The GPUI AppCell (root context)
    app_cell: Option<Rc<AppCell>>,
    /// Window handle (ZedraApp or PreviewApp depending on `preview` feature)
    window: Option<AnyWindowHandle>,
    /// Whether a surface is currently available
    surface_available: bool,
}

impl AndroidApp {
    pub fn new() -> Self {
        Self {
            platform_initialized: false,
            platform: None,
            app_cell: None,
            window: None,
            surface_available: false,
        }
    }

    /// Process a command on the main thread
    pub fn process_command(&mut self, command: AndroidCommand) -> Result<()> {
        match command {
            AndroidCommand::Initialize { jvm, activity } => self.handle_initialize(jvm, activity),
            AndroidCommand::SurfaceCreated { width, height } => {
                self.handle_surface_created(width, height)
            }
            AndroidCommand::SurfaceChanged { width, height } => {
                self.handle_surface_changed(width, height)
            }
            AndroidCommand::SurfaceDestroyed => self.handle_surface_destroyed(),
            AndroidCommand::Touch {
                action,
                x,
                y,
                pointer_id,
            } => self.handle_touch(action, x, y, pointer_id),
            AndroidCommand::Key {
                action,
                key_code,
                unicode,
            } => self.handle_key(action, key_code, unicode),
            AndroidCommand::ImeText { text } => self.handle_ime_text(&text),
            AndroidCommand::Resume => self.handle_resume(),
            AndroidCommand::Pause => self.handle_pause(),
            AndroidCommand::Destroy => self.handle_destroy(),
            AndroidCommand::RequestFrame => self.handle_frame_request(),
            AndroidCommand::ConnectToHost { host_id } => self.handle_connect_to_host(host_id),
            AndroidCommand::Fling {
                velocity_x,
                velocity_y,
            } => self.handle_fling(velocity_x, velocity_y),
            AndroidCommand::KeyboardHeightChanged { height } => self.handle_keyboard_height(height),
            AndroidCommand::Deeplink { url } => self.handle_deeplink_url(url),
        }
    }

    /// Initialize the GPUI platform and App
    fn handle_initialize(
        &mut self,
        jvm: Arc<JavaVM>,
        activity: Arc<Mutex<GlobalRef>>,
    ) -> Result<()> {
        let start = std::time::Instant::now();

        if self.platform_initialized {
            log::warn!("AndroidApp already initialized");
            return Ok(());
        }

        // Extract owned values from Arc
        let extract_start = std::time::Instant::now();
        let jvm_owned = Arc::try_unwrap(jvm).unwrap_or_else(|arc| {
            // If Arc has multiple references, we need to work around JavaVM not being Clone
            log::warn!("JVM Arc has multiple references");
            unsafe { std::ptr::read(&*arc) }
        });

        let activity_owned = {
            let guard = activity.lock().unwrap();
            guard.clone()
        };
        log::info!(
            "[TIMING] Extract JVM/Activity: {:?}",
            extract_start.elapsed()
        );

        // Create the Android platform
        let platform_start = std::time::Instant::now();
        log::info!("[TIMING] Starting AndroidPlatform::new()...");
        let android_platform = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            AndroidPlatform::new(jvm_owned, activity_owned)
        })) {
            Ok(platform) => platform,
            Err(e) => {
                let msg = if let Some(s) = e.downcast_ref::<&str>() {
                    s.to_string()
                } else if let Some(s) = e.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "Unknown panic".to_string()
                };
                log::error!("Panic while creating AndroidPlatform: {}", msg);
                return Err(anyhow::anyhow!("Failed to create AndroidPlatform: {}", msg));
            }
        };
        log::info!(
            "[TIMING] AndroidPlatform::new() completed: {:?}",
            platform_start.elapsed()
        );

        // Wrap in Rc - no need to specify dyn Platform since we're passing it directly
        let platform = Rc::new(android_platform);

        // Register the AndroidBridge as the global PlatformBridge before anything reads density.
        platform_bridge::set_bridge(AndroidBridge);

        // Set the actual display scale from Android DisplayMetrics before opening any windows.
        // The platform defaults to 3.0 but the actual device density may differ (e.g. 2.75).
        let density = platform_bridge::bridge().density();
        platform.set_display_scale(density);
        log::info!("Set platform display_scale to {}", density);

        // Store a reference to the platform for frame requests
        self.platform = Some(platform.clone());

        // Create the GPUI AppCell
        let app_cell_start = std::time::Instant::now();
        log::info!("[TIMING] Starting App::new_app()...");
        let app_cell = App::new_app(
            platform,
            Arc::new(ZedraAssets),                    // Embedded SVG icons
            Arc::new(http_client::BlockedHttpClient), // Use BlockedHttpClient
        );
        log::info!(
            "[TIMING] App::new_app() completed: {:?}",
            app_cell_start.elapsed()
        );

        self.app_cell = Some(app_cell);
        self.platform_initialized = true;

        log::info!("[TIMING] Total handle_initialize: {:?}", start.elapsed());
        Ok(())
    }

    /// Handle surface created - create window with ZedraApp and attach native surface
    fn handle_surface_created(&mut self, width: u32, height: u32) -> Result<()> {
        let start = std::time::Instant::now();
        log::info!("Surface created: {}x{}", width, height);

        if !self.platform_initialized {
            log::error!("Cannot create surface before platform initialization");
            return Err(anyhow::anyhow!("Platform not initialized"));
        }

        self.surface_available = true;

        // Create window if not already created (first time only)
        if self.window.is_none() {
            let window_start = std::time::Instant::now();
            log::info!("[TIMING] Starting window creation...");

            if let Some(app_cell) = &self.app_cell {
                let mut app = app_cell.borrow_mut();

                // Use actual screen dimensions from native window and display density
                let screen_width_px = width as f32;
                let screen_height_px = height as f32;
                let scale = platform_bridge::bridge().density();
                log::info!(
                    "Window dimensions: {}x{} physical, scale={}, logical={}x{}",
                    screen_width_px,
                    screen_height_px,
                    scale,
                    screen_width_px / scale,
                    screen_height_px / scale
                );

                let window_bounds = WindowBounds::Windowed(Bounds {
                    origin: point(px(0.0), px(0.0)),
                    size: size(px(screen_width_px / scale), px(screen_height_px / scale)),
                });

                // Configure window options
                let window_options = WindowOptions {
                    window_bounds: Some(window_bounds),
                    focus: true,
                    show: true,
                    window_background: WindowBackgroundAppearance::Transparent,
                    ..Default::default()
                };

                match app::open_zedra_window(&mut app, window_options) {
                    Ok(window_handle) => {
                        self.window = Some(window_handle);
                        log::info!(
                            "[TIMING] Window creation completed: {:?}",
                            window_start.elapsed()
                        );
                    }
                    Err(e) => {
                        log::error!("Failed to open window: {:?}", e);
                        return Err(e);
                    }
                }
            } else {
                log::error!("AppCell not available");
                return Err(anyhow::anyhow!("AppCell not available"));
            }
        }

        // ALWAYS attach the native window when surface is created
        // This handles both initial creation and recreation after background/foreground cycle
        let attach_start = std::time::Instant::now();
        log::info!("[TIMING] Starting native window attachment...");

        if let Some(platform) = &self.platform {
            if let Some(native_window) = jni::take_native_window() {
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    platform.attach_native_window(native_window)
                })) {
                    Ok(Ok(())) => {
                        log::info!(
                            "[TIMING] Native window attachment completed: {:?}",
                            attach_start.elapsed()
                        );
                    }
                    Ok(Err(e)) => {
                        log::error!("Failed to attach native window: {:?}", e);
                        return Err(e);
                    }
                    Err(panic_info) => {
                        let msg = if let Some(s) = panic_info.downcast_ref::<&str>() {
                            s.to_string()
                        } else if let Some(s) = panic_info.downcast_ref::<String>() {
                            s.clone()
                        } else {
                            "Unknown panic".to_string()
                        };
                        log::error!("Panic while attaching native window: {}", msg);
                        return Err(anyhow::anyhow!("Panic during surface attachment: {}", msg));
                    }
                }
            } else {
                log::error!("Native window not found in global storage");
                return Err(anyhow::anyhow!("Native window not available"));
            }
        }

        log::info!(
            "[TIMING] Total handle_surface_created: {:?}",
            start.elapsed()
        );
        Ok(())
    }

    /// Handle surface changed (resize/rotation)
    fn handle_surface_changed(&mut self, width: u32, height: u32) -> Result<()> {
        log::info!("Surface changed: {}x{}", width, height);

        // Resize the AndroidWindow's blade renderer surface
        if let Some(platform) = &self.platform {
            platform.handle_surface_resize(width, height)?;
        } else {
            log::warn!("Platform not available for surface resize");
        }

        Ok(())
    }

    /// Handle surface destroyed
    fn handle_surface_destroyed(&mut self) -> Result<()> {
        log::info!("Surface destroyed");
        self.surface_available = false;

        // Notify the platform to destroy the renderer
        // The window persists, but the renderer must be destroyed
        if let Some(platform) = &self.platform {
            platform.detach_native_window();
            log::info!("Native window detached, renderer destroyed");
        }

        Ok(())
    }

    /// Handle touch event — delegates to AndroidPlatform
    fn handle_touch(&mut self, action: i32, x: f32, y: f32, _pointer_id: i32) -> Result<()> {
        if let Some(platform) = &self.platform {
            platform.handle_touch(action, x, y);
        }
        Ok(())
    }

    /// Handle key event - convert to GPUI keystroke and dispatch
    fn handle_key(&mut self, action: i32, key_code: i32, unicode: i32) -> Result<()> {
        let platform = match &self.platform {
            Some(p) => p,
            None => return Ok(()),
        };

        // Only handle key down events for now
        if action != 0 {
            return Ok(());
        }

        let keystroke = android_keycode_to_keystroke(key_code, unicode);
        if let Some(keystroke) = keystroke {
            let input = PlatformInput::KeyDown(KeyDownEvent {
                keystroke,
                is_held: false,
                prefer_character_input: false,
            });
            platform.dispatch_input(input);
        }

        Ok(())
    }

    /// Handle IME text input - dispatches text as individual key events
    /// but defers to next frame to avoid reentrancy issues
    fn handle_ime_text(&mut self, text: &str) -> Result<()> {
        let platform = match &self.platform {
            Some(p) => p,
            None => return Ok(()),
        };

        log::debug!("IME text: {}", text);

        // Dispatch each character as a key event
        for ch in text.chars() {
            let keystroke = Keystroke {
                modifiers: Modifiers::default(),
                key: ch.to_lowercase().to_string(),
                key_char: Some(ch.to_string()),
            };
            let input = PlatformInput::KeyDown(KeyDownEvent {
                keystroke,
                is_held: false,
                prefer_character_input: true, // Prefer character input for IME
            });
            platform.dispatch_input(input);
        }

        Ok(())
    }

    /// Handle fling gesture — delegates to AndroidPlatform
    fn handle_fling(&mut self, velocity_x: f32, velocity_y: f32) -> Result<()> {
        if let Some(platform) = &self.platform {
            platform.handle_fling(velocity_x, velocity_y);
        }
        Ok(())
    }

    /// Process active fling — delegates to AndroidPlatform
    fn process_fling(&mut self) {
        if let Some(platform) = &self.platform {
            platform.process_fling();
        }
    }

    /// Handle keyboard height change — trigger a re-render so TerminalView picks up the new height
    fn handle_keyboard_height(&mut self, height: u32) -> Result<()> {
        log::info!("Keyboard height changed: {}px", height);
        if let (Some(app_cell), Some(window)) = (&self.app_cell, self.window) {
            let mut borrow = app_cell.borrow_mut();
            let _ = window.update(&mut **borrow, |_, window, _| window.refresh());
        }
        if let Some(ref platform) = self.platform {
            platform.request_frame_forced();
        }
        Ok(())
    }

    /// Handle app resume
    fn handle_resume(&mut self) -> Result<()> {
        log::info!("App resumed");
        Ok(())
    }

    /// Handle app pause
    fn handle_pause(&mut self) -> Result<()> {
        log::info!("App paused");
        platform_bridge::clear_pending_alerts();
        Ok(())
    }

    /// Handle app destruction
    fn handle_destroy(&mut self) -> Result<()> {
        // Clean up in reverse order
        self.window = None;
        self.app_cell = None;
        self.platform_initialized = false;
        self.surface_available = false;

        Ok(())
    }

    /// Handle frame request (called at ~60 FPS)
    fn handle_frame_request(&mut self) -> Result<()> {
        // Process active fling momentum scrolling
        self.process_fling();

        let fling_active = self.platform.as_ref().map_or(false, |p| p.has_active_fling());

        // Drain and execute any main-thread callbacks from the session runtime.
        // Non-empty drain means something signaled a forced render (terminal data, etc.).
        let callbacks = zedra_session::drain_callbacks();
        let callbacks_pending = !callbacks.is_empty();
        for cb in callbacks {
            cb();
        }

        // When PTY data arrived, call window.refresh() so all views re-render.
        // request_frame_forced() bypasses the window-level dirty gate but does NOT
        // bypass GPUI's per-view render cache — without refresh(), TerminalView::render()
        // is skipped because dirty_views is empty and window.refreshing is false.
        if callbacks_pending {
            if let (Some(app_cell), Some(window)) = (&self.app_cell, self.window) {
                let mut borrow = app_cell.borrow_mut();
                let _ = window.update(&mut **borrow, |_, window, _| window.refresh());
            }
        }

        // Request frames on all windows via the AndroidPlatform
        // This triggers GPUI's rendering pipeline
        if let Some(ref platform) = self.platform {
            if callbacks_pending || fling_active {
                // Force render when terminal data is pending or fling is active
                platform.request_frame_forced();
            } else {
                platform.request_frame_for_all_windows();
            }
        }
        Ok(())
    }

    fn handle_deeplink_url(&mut self, url: String) -> Result<()> {
        log::info!("Deeplink received: {}", &url[..url.len().min(80)]);
        match deeplink::parse(&url) {
            Ok(action) => deeplink::enqueue(action),
            Err(e) => log::error!("Invalid deeplink URL: {}", e),
        }
        Ok(())
    }

    fn handle_connect_to_host(&mut self, host_id: String) -> Result<()> {
        log::info!("Connect to host requested: {}", host_id);
        Ok(())
    }
}

impl Default for AndroidApp {
    fn default() -> Self {
        Self::new()
    }
}

// Thread-local storage for the AndroidApp instance
thread_local! {
    static ANDROID_APP: std::cell::RefCell<Option<AndroidApp>> = std::cell::RefCell::new(None);
}

/// Initialize the AndroidApp on the main thread
pub fn init_android_app() {
    ANDROID_APP.with(|app| {
        if app.borrow().is_none() {
            *app.borrow_mut() = Some(AndroidApp::new());
        }
    });
}

/// Convert an Android keycode + unicode to a GPUI Keystroke
fn android_keycode_to_keystroke(key_code: i32, unicode: i32) -> Option<Keystroke> {
    // Android KeyEvent constants
    const KEYCODE_DEL: i32 = 67; // Backspace
    const KEYCODE_FORWARD_DEL: i32 = 112;
    const KEYCODE_ENTER: i32 = 66;
    const KEYCODE_TAB: i32 = 61;
    const KEYCODE_SPACE: i32 = 62;
    const KEYCODE_ESCAPE: i32 = 111;
    const KEYCODE_DPAD_UP: i32 = 19;
    const KEYCODE_DPAD_DOWN: i32 = 20;
    const KEYCODE_DPAD_LEFT: i32 = 21;
    const KEYCODE_DPAD_RIGHT: i32 = 22;

    let key = match key_code {
        KEYCODE_DEL => "backspace".to_string(),
        KEYCODE_FORWARD_DEL => "delete".to_string(),
        KEYCODE_ENTER => "enter".to_string(),
        KEYCODE_TAB => "tab".to_string(),
        KEYCODE_SPACE => "space".to_string(),
        KEYCODE_ESCAPE => "escape".to_string(),
        KEYCODE_DPAD_UP => "up".to_string(),
        KEYCODE_DPAD_DOWN => "down".to_string(),
        KEYCODE_DPAD_LEFT => "left".to_string(),
        KEYCODE_DPAD_RIGHT => "right".to_string(),
        _ => {
            // Use unicode character if available
            if unicode > 0 {
                if let Some(ch) = char::from_u32(unicode as u32) {
                    ch.to_lowercase().to_string()
                } else {
                    return None;
                }
            } else {
                return None;
            }
        }
    };

    let key_char = if unicode > 0 {
        char::from_u32(unicode as u32).map(|c| c.to_string())
    } else {
        None
    };

    Some(Keystroke {
        modifiers: Modifiers::default(),
        key,
        key_char,
    })
}

/// Process commands from the queue on the main thread
/// Called by Choreographer at 60 FPS
pub fn process_commands_from_queue() -> Result<()> {
    let commands = command_queue::drain_commands();

    ANDROID_APP.with(|app_cell| {
        let mut app_opt = app_cell.borrow_mut();

        if let Some(app) = app_opt.as_mut() {
            // Process queued commands
            for command in commands {
                if let Err(e) = app.process_command(command) {
                    log::error!("Error processing command: {}", e);
                }
            }

            // Drain foreground executor task queue (spawned async tasks, timers, etc.)
            if let Some(platform) = &app.platform {
                platform.process_pending_tasks();
            }

            // Request frame refresh (called every Choreographer frame @ 60 FPS)
            if let Err(e) = app.handle_frame_request() {
                log::error!("Error in frame request: {}", e);
            }

            Ok(())
        } else {
            Err(anyhow::anyhow!("AndroidApp not initialized"))
        }
    })
}
