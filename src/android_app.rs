/// Android GPUI Application Bridge
///
/// This module provides the main-thread-only interface for running GPUI apps on Android.
/// It processes commands from the thread-safe queue and manages the GPUI App lifecycle.
use anyhow::Result;
use gpui::{AndroidPlatform, *};
use jni::{objects::GlobalRef, JavaVM};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use crate::android_command_queue::AndroidCommand;
use crate::zedra_app::ZedraApp;

/// Android app state - must only be accessed from the main UI thread
pub struct AndroidApp {
    /// Whether the platform has been initialized
    platform_initialized: bool,
    /// Reference to the AndroidPlatform for triggering frame requests
    platform: Option<Rc<AndroidPlatform>>,
    /// The GPUI AppCell (root context)
    app_cell: Option<Rc<AppCell>>,
    /// Window handle for the ZedraApp
    window: Option<WindowHandle<ZedraApp>>,
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
            AndroidCommand::Resume => self.handle_resume(),
            AndroidCommand::Pause => self.handle_pause(),
            AndroidCommand::Destroy => self.handle_destroy(),
            AndroidCommand::RequestFrame => self.handle_frame_request(),
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

        // Create liveness tracking
        let liveness = Arc::new(());
        let liveness_weak = Arc::downgrade(&liveness);

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
            AndroidPlatform::new(liveness_weak, jvm_owned, activity_owned)
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

        // Store a reference to the platform for frame requests
        self.platform = Some(platform.clone());

        // Create the GPUI AppCell
        let app_cell_start = std::time::Instant::now();
        log::info!("[TIMING] Starting App::new_app()...");
        let app_cell = App::new_app(
            platform,
            liveness,
            Arc::new(()),                             // Unit type implements AssetSource
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

                // Get actual screen dimensions (1080x2400 pixels, scale 3.0)
                // For now use hardcoded values - TODO: get from JNI/display metrics
                let screen_width_px = 1080.0;
                let screen_height_px = 2400.0;
                let scale = 3.0;

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

                // Open window with ZedraApp view
                match app.open_window(window_options, |window, cx| {
                    let view = cx.new(|_cx| ZedraApp::new());
                    window.refresh();
                    view
                }) {
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
            if let Some(native_window) = crate::android_jni::take_native_window() {
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

    /// Handle touch event
    fn handle_touch(&mut self, action: i32, x: f32, y: f32, pointer_id: i32) -> Result<()> {
        let action_name = match action {
            0 => "DOWN",
            1 => "UP",
            2 => "MOVE",
            3 => "CANCEL",
            _ => "UNKNOWN",
        };

        // TODO: Convert to GPUI mouse event and dispatch to window

        Ok(())
    }

    /// Handle key event
    fn handle_key(&mut self, _action: i32, _key_code: i32, _unicode: i32) -> Result<()> {
        // TODO: Convert to GPUI key event and dispatch to window
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
        // Request frames on all windows via the AndroidPlatform
        // This triggers GPUI's rendering pipeline
        if let Some(ref platform) = self.platform {
            platform.request_frame_for_all_windows();
        }
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

/// Process commands from the queue on the main thread
/// Called by Choreographer at 60 FPS
pub fn process_commands_from_queue() -> Result<()> {
    let commands = crate::android_command_queue::drain_commands();

    ANDROID_APP.with(|app_cell| {
        let mut app_opt = app_cell.borrow_mut();

        if let Some(app) = app_opt.as_mut() {
            // Process queued commands
            for command in commands {
                if let Err(e) = app.process_command(command) {
                    log::error!("Error processing command: {}", e);
                }
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
