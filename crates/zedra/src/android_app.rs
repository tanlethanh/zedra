/// Android GPUI Application Bridge
///
/// This module provides the main-thread-only interface for running GPUI apps on Android.
/// It processes commands from the thread-safe queue and manages the GPUI App lifecycle.
use anyhow::Result;
use gpui::{AndroidPlatform, *};
use jni::{JavaVM, objects::GlobalRef};
use rust_embed::RustEmbed;
use std::borrow::Cow;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use crate::android_command_queue::AndroidCommand;
use crate::gesture::{GestureArena, GestureKind};
use crate::zedra_app::ZedraApp;

/// Embedded assets for Zedra (SVG icons, etc.)
#[derive(RustEmbed)]
#[folder = "assets"]
#[include = "icons/*.svg"]
struct ZedraAssets;

impl gpui::AssetSource for ZedraAssets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(Self::get(path).map(|f| f.data))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(Self::iter()
            .filter(|name| name.starts_with(path))
            .map(|name| name.into())
            .collect())
    }
}

/// Active fling state for momentum scrolling
struct FlingState {
    velocity_x: f32,
    velocity_y: f32,
    last_time: std::time::Instant,
    /// Last touch position (logical pixels) for dispatching scroll events
    position: Point<Pixels>,
}

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
    /// Last touch position for scroll delta calculation (logical pixels)
    last_touch_position: Option<(f32, f32)>,
    /// Touch-down origin for tap detection (logical pixels)
    touch_down_position: Option<(f32, f32)>,
    /// Whether the current touch gesture has moved beyond tap threshold
    touch_is_drag: bool,
    /// Active fling for momentum scrolling
    fling_state: Option<FlingState>,
    /// Gesture arena for drawer-vs-scroll disambiguation
    gesture_arena: GestureArena,
}

impl AndroidApp {
    pub fn new() -> Self {
        Self {
            platform_initialized: false,
            platform: None,
            app_cell: None,
            window: None,
            surface_available: false,
            last_touch_position: None,
            touch_down_position: None,
            touch_is_drag: false,
            fling_state: None,
            gesture_arena: GestureArena::default_drawer_scroll(),
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
            AndroidCommand::PairViaQr { qr_data } => self.handle_scan_via_qr(qr_data),
            AndroidCommand::ConnectToHost { host_id } => self.handle_connect_to_host(host_id),
            AndroidCommand::Fling {
                velocity_x,
                velocity_y,
            } => self.handle_fling(velocity_x, velocity_y),
            AndroidCommand::KeyboardHeightChanged { height } => {
                self.handle_keyboard_height(height)
            }
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

        // Set the actual display scale from Android DisplayMetrics before opening any windows.
        // The platform defaults to 3.0 but the actual device density may differ (e.g. 2.75).
        let density = crate::android_jni::get_density();
        platform.set_display_scale(density);
        log::info!("Set platform display_scale to {}", density);

        // Store a reference to the platform for frame requests
        self.platform = Some(platform.clone());

        // Create the GPUI AppCell
        let app_cell_start = std::time::Instant::now();
        log::info!("[TIMING] Starting App::new_app()...");
        let app_cell = App::new_app(
            platform,
            liveness,
            Arc::new(ZedraAssets),                     // Embedded SVG icons
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
                let scale = crate::android_jni::get_density();
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

                // Open window with ZedraApp view
                match app.open_window(window_options, |window, cx| {
                    let view = cx.new(|cx| ZedraApp::new(window, cx));
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

    /// Handle touch event - convert to GPUI mouse/scroll events and dispatch
    fn handle_touch(&mut self, action: i32, x: f32, y: f32, _pointer_id: i32) -> Result<()> {
        let platform = match &self.platform {
            Some(p) => p,
            None => return Ok(()),
        };

        // Convert physical pixels to logical pixels
        let scale = crate::android_jni::get_density();
        let logical_x = x / scale;
        let logical_y = y / scale;
        let position = point(px(logical_x), px(logical_y));

        log::trace!(
            "handle_touch: action={}, pos=({:.1}, {:.1})",
            action,
            logical_x,
            logical_y
        );

        // Tap slop threshold in logical pixels — movement beyond this means a drag, not a tap
        const TAP_SLOP: f32 = 10.0;

        match action {
            0 => {
                // ACTION_DOWN — cancel any active fling, record origin for tap detection
                self.fling_state = None;
                self.last_touch_position = Some((logical_x, logical_y));
                self.touch_down_position = Some((logical_x, logical_y));
                self.touch_is_drag = false;
                self.gesture_arena.reset();
                zedra_nav::reset_drawer_gesture();
            }
            1 => {
                // ACTION_UP — if the finger didn't move beyond TAP_SLOP, treat as tap
                if !self.touch_is_drag {
                    platform.dispatch_input(PlatformInput::MouseDown(MouseDownEvent {
                        button: MouseButton::Left,
                        position,
                        modifiers: Modifiers::default(),
                        click_count: 1,
                        first_mouse: false,
                    }));
                }
                // Always dispatch MouseUp so gesture-driven UI (drawer snap) works
                platform.dispatch_input(PlatformInput::MouseUp(MouseUpEvent {
                    button: MouseButton::Left,
                    position,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                }));
                self.last_touch_position = None;
                self.touch_down_position = None;
                self.touch_is_drag = false;
                // Don't reset arena here — handle_fling() needs the winner.
                // Arena resets on next ACTION_DOWN.
            }
            3 => {
                // ACTION_CANCEL — clean up, no tap
                self.last_touch_position = None;
                self.touch_down_position = None;
                self.touch_is_drag = false;
            }
            2 => {
                // ACTION_MOVE — check tap slop, then feed gesture arena
                if !self.touch_is_drag {
                    if let Some((down_x, down_y)) = self.touch_down_position {
                        let dx = logical_x - down_x;
                        let dy = logical_y - down_y;
                        if (dx * dx + dy * dy).sqrt() > TAP_SLOP {
                            self.touch_is_drag = true;
                        }
                    }
                }

                if self.touch_is_drag {
                    if let Some((last_x, last_y)) = self.last_touch_position {
                        let delta_x = logical_x - last_x;
                        let delta_y = logical_y - last_y;

                        // Feed the gesture arena
                        self.gesture_arena.on_move(delta_x, delta_y);

                        match self.gesture_arena.winner() {
                            Some(GestureKind::DrawerPan) => {
                                // Push to drawer bridge — bypasses GPUI scroll
                                // dispatch so it can't conflict with content scroll.
                                zedra_nav::push_drawer_pan_delta(delta_x);
                                platform.request_frame_forced();
                            }
                            Some(GestureKind::Scroll) => {
                                // Vertical only — for content scroll
                                platform.dispatch_input(PlatformInput::ScrollWheel(
                                    ScrollWheelEvent {
                                        position,
                                        delta: ScrollDelta::Pixels(point(
                                            px(0.0),
                                            px(delta_y),
                                        )),
                                        modifiers: Modifiers::default(),
                                        touch_phase: TouchPhase::Moved,
                                    },
                                ));
                            }
                            None => {
                                // Still undetermined — don't dispatch (buffer phase)
                            }
                        }
                    }
                }
                self.last_touch_position = Some((logical_x, logical_y));
            }
            _ => {}
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

    /// Handle fling gesture — start momentum scrolling.
    /// Filters velocity to match the gesture arena winner's axis.
    fn handle_fling(&mut self, velocity_x: f32, velocity_y: f32) -> Result<()> {
        // Convert physical velocity to logical pixels/second
        let scale = crate::android_jni::get_density();
        let vx = velocity_x / scale;
        let vy = velocity_y / scale;

        // Filter fling velocity to the winning gesture's axis.
        // DrawerPan: skip fling entirely — the drawer snap animation handles it.
        // Scroll: vertical only.
        let (fling_vx, fling_vy) = match self.gesture_arena.winner() {
            Some(GestureKind::DrawerPan) => return Ok(()),
            Some(GestureKind::Scroll) => (0.0, vy),
            None => (vx, vy),
        };

        // Use last known touch position for dispatching fling scroll events
        let pos = self
            .last_touch_position
            .map(|(x, y)| point(px(x), px(y)))
            .unwrap_or_else(|| point(px(0.0), px(0.0)));

        // Only start fling if velocity is significant
        if fling_vx.abs() > 50.0 || fling_vy.abs() > 50.0 {
            self.fling_state = Some(FlingState {
                velocity_x: fling_vx,
                velocity_y: fling_vy,
                last_time: std::time::Instant::now(),
                position: pos,
            });
        }
        Ok(())
    }

    /// Process active fling — apply friction and dispatch scroll events
    fn process_fling(&mut self) {
        let platform = match &self.platform {
            Some(p) => p,
            None => return,
        };

        let fling = match &mut self.fling_state {
            Some(f) => f,
            None => return,
        };

        let now = std::time::Instant::now();
        let dt = now.duration_since(fling.last_time).as_secs_f32();
        fling.last_time = now;

        // Apply frame-rate independent friction: velocity *= 0.95^(dt * 60)
        let friction = 0.95_f32.powf(dt * 60.0);
        fling.velocity_x *= friction;
        fling.velocity_y *= friction;

        let vx = fling.velocity_x;
        let vy = fling.velocity_y;

        // Stop fling when velocity is below threshold
        if vx.abs() < 50.0 && vy.abs() < 50.0 {
            self.fling_state = None;
            return;
        }

        // Dispatch scroll event with velocity-based delta at original touch position
        let delta_x = vx * dt;
        let delta_y = vy * dt;

        platform.dispatch_input(PlatformInput::ScrollWheel(ScrollWheelEvent {
            position: fling.position,
            delta: ScrollDelta::Pixels(point(px(delta_x), px(delta_y))),
            modifiers: Modifiers::default(),
            touch_phase: TouchPhase::Moved,
        }));
    }

    /// Handle keyboard height change — trigger a re-render so TerminalView picks up the new height
    fn handle_keyboard_height(&mut self, height: u32) -> Result<()> {
        log::info!("Keyboard height changed: {}px", height);
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

        // Check if terminal has pending data from RPC session
        let terminal_data_pending = zedra_session::check_and_clear_terminal_data();
        let fling_active = self.fling_state.is_some();

        // Drain and execute any main-thread callbacks from the session runtime
        for cb in zedra_session::drain_callbacks() {
            cb();
        }

        // Request frames on all windows via the AndroidPlatform
        // This triggers GPUI's rendering pipeline
        if let Some(ref platform) = self.platform {
            if terminal_data_pending || fling_active {
                // Force render when terminal data is pending or fling is active
                platform.request_frame_forced();
            } else {
                platform.request_frame_for_all_windows();
            }
        }
        Ok(())
    }

    fn handle_scan_via_qr(&mut self, qr_data: String) -> Result<()> {
        log::info!(
            "QR pairing requested: {}",
            &qr_data[..qr_data.len().min(50)]
        );

        // Parse the QR URI into a PairingPayloadV3, then convert to PeerInfo
        match zedra_transport::pairing::parse_pairing_uri(&qr_data) {
            Ok(payload) => {
                let peer_info = payload.to_peer_info();
                log::info!(
                    "QR parsed: hostname={}, addrs={:?}, pubkey={}",
                    peer_info.hostname,
                    peer_info.host_addrs,
                    &peer_info.host_pubkey[..8]
                );
                crate::zedra_app::set_pending_qr_peer_info(peer_info);
                // Signal re-render so ZedraApp picks up the pending PeerInfo
                zedra_session::signal_terminal_data();
            }
            Err(e) => {
                log::error!("Failed to parse QR pairing URI: {}", e);
            }
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
