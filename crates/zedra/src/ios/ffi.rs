/// C FFI bridge for iOS — the equivalent of android/jni.rs
///
/// All functions are `extern "C"` with `#[no_mangle]` so cbindgen generates
/// a C header that Swift can import via the module map.
///
/// Memory convention:
///   - Strings returned to Swift are allocated with CString::into_raw()
///   - Swift MUST call zedra_free_string() to release them
///   - Strings passed from Swift are `*const c_char` and borrowed (not freed)
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::Once;

use super::command_queue::{IosCommand, get_command_sender};

static INIT: Once = Once::new();

// =============================================================================
// Initialization
// =============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn zedra_init() {
    INIT.call_once(|| {
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

        super::app::init_ios_app();

        // Register the iOS bridge for platform abstraction
        crate::platform_bridge::set_bridge(super::bridge::IosBridge);

        log::info!("Zedra iOS initialized");
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn zedra_init_screen(width: f32, height: f32, scale: f32) {
    let sender = get_command_sender();
    let _ = sender.send(IosCommand::Initialize {
        screen_width: width,
        screen_height: height,
        scale,
    });
}

// =============================================================================
// Frame Processing (called from CADisplayLink at 60 FPS)
// =============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn zedra_process_frame() {
    if let Err(e) = super::app::process_commands_from_queue() {
        log::error!("Error processing frame: {:?}", e);
    }
}

// =============================================================================
// Connection
// =============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn zedra_connect(host: *const c_char, port: u16) {
    let host = match unsafe { CStr::from_ptr(host) }.to_str() {
        Ok(s) => s.to_string(),
        Err(_) => {
            log::error!("Invalid UTF-8 in host string");
            return;
        }
    };

    let sender = get_command_sender();
    let _ = sender.send(IosCommand::Connect { host, port });
}

#[unsafe(no_mangle)]
pub extern "C" fn zedra_disconnect() {
    let sender = get_command_sender();
    let _ = sender.send(IosCommand::Disconnect);
}

#[unsafe(no_mangle)]
pub extern "C" fn zedra_pair_via_qr(data: *const c_char) {
    let qr_data = match unsafe { CStr::from_ptr(data) }.to_str() {
        Ok(s) => s.to_string(),
        Err(_) => {
            log::error!("Invalid UTF-8 in QR data");
            return;
        }
    };

    let sender = get_command_sender();
    let _ = sender.send(IosCommand::PairViaQr { qr_data });
}

// =============================================================================
// Terminal I/O
// =============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn zedra_send_input(text: *const c_char) {
    let text = match unsafe { CStr::from_ptr(text) }.to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return,
    };

    let sender = get_command_sender();
    let _ = sender.send(IosCommand::SendInput { text });
}

#[unsafe(no_mangle)]
pub extern "C" fn zedra_send_key(key_name: *const c_char) {
    let key = match unsafe { CStr::from_ptr(key_name) }.to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return,
    };

    let sender = get_command_sender();
    let _ = sender.send(IosCommand::KeyEvent { key_name: key });
}

#[unsafe(no_mangle)]
pub extern "C" fn zedra_get_terminal_output() -> *mut c_char {
    let output = super::app::take_terminal_output();
    if output.is_empty() {
        return std::ptr::null_mut();
    }
    CString::new(output)
        .map(|s| s.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

// =============================================================================
// Status Queries
// =============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn zedra_get_connection_status() -> i32 {
    use super::app::ConnectionStatus;
    match super::app::get_connection_status() {
        ConnectionStatus::Disconnected => 0,
        ConnectionStatus::Connecting => 1,
        ConnectionStatus::Connected => 2,
        ConnectionStatus::Error(_) => 3,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn zedra_get_connection_error() -> *mut c_char {
    use super::app::ConnectionStatus;
    match super::app::get_connection_status() {
        ConnectionStatus::Error(msg) => CString::new(msg)
            .map(|s| s.into_raw())
            .unwrap_or(std::ptr::null_mut()),
        _ => std::ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn zedra_get_transport_info() -> *mut c_char {
    let info = super::app::get_transport_info();
    if info.is_empty() {
        return std::ptr::null_mut();
    }
    CString::new(info)
        .map(|s| s.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

// =============================================================================
// Lifecycle
// =============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn zedra_on_resume() {
    let sender = get_command_sender();
    let _ = sender.send(IosCommand::Resume);
}

#[unsafe(no_mangle)]
pub extern "C" fn zedra_on_pause() {
    let sender = get_command_sender();
    let _ = sender.send(IosCommand::Pause);
}

// =============================================================================
// Memory Management
// =============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn zedra_free_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        unsafe { drop(CString::from_raw(ptr)) };
    }
}

// =============================================================================
// Touch Input
// =============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn zedra_touch_event(action: i32, x: f32, y: f32) {
    let sender = get_command_sender();
    let _ = sender.send(IosCommand::Touch { action, x, y });
}

#[unsafe(no_mangle)]
pub extern "C" fn zedra_view_resized(width: f32, height: f32) {
    let sender = get_command_sender();
    let _ = sender.send(IosCommand::ViewResized { width, height });
}
