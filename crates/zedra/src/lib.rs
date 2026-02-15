// Zedra — universal mobile application (Android + iOS)
// Platform-specific code is gated with #[cfg(target_os)]

// Shared color constants (Figma palette)
pub mod theme;

// Home screen (starting + projects)
pub mod home_view;

// App drawer (header + file tree + footer nav icons)
pub mod app_drawer;

// Text input component with keyboard support
pub mod input;

// Shared Zedra app (screen navigation + connection)
pub mod zedra_app;

// File explorer view
pub mod file_explorer;

// File preview card grid
pub mod file_preview_list;

// Project editor: split-pane file explorer + code editor
pub mod project_editor;

// Unified platform bridge (keyboard, QR scanner)
pub mod platform_bridge;

// --- Android-only modules ---

#[cfg(target_os = "android")]
pub mod android_jni;

#[cfg(target_os = "android")]
pub mod android_app;

#[cfg(target_os = "android")]
pub mod android_command_queue;

#[cfg(target_os = "android")]
mod legacy_jni {
    use jni::{JNIEnv, objects::JClass};

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_dev_zedra_app_MainActivity_rustOnResume(_: JNIEnv, _: JClass) {}

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_dev_zedra_app_MainActivity_rustOnPause(_: JNIEnv, _: JClass) {}
}

// --- iOS-only modules ---

#[cfg(target_os = "ios")]
pub mod gpui_app;

#[cfg(target_os = "ios")]
pub mod ios_ffi;

#[cfg(target_os = "ios")]
pub mod ios_app;

#[cfg(target_os = "ios")]
pub mod ios_command_queue;

#[cfg(target_os = "ios")]
pub mod pairing;
