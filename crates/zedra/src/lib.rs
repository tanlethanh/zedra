// Zedra Android application - GPUI on Android via Blade/Vulkan

// Shared color constants (Figma palette)
pub mod theme;

// Editor: text buffer, syntax highlighting, code editor view
pub mod text_buffer;
pub mod syntax_highlighter;
pub mod syntax_theme;
pub mod code_editor;
pub mod diff_view;
pub mod git_diff_view;
pub mod git_sidebar;
pub mod git_stack;

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

// Project editor: split-pane file explorer + code editor
pub mod project_editor;

// GPUI Android JNI bridge
pub mod android_jni;

// Android app bridge
pub mod android_app;

// Android command queue for threading
pub mod android_command_queue;

// Legacy JNI stubs (called by Java but no longer used)
mod legacy_jni {
    use jni::{JNIEnv, objects::JClass};

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_dev_zedra_app_MainActivity_rustOnResume(_: JNIEnv, _: JClass) {}

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_dev_zedra_app_MainActivity_rustOnPause(_: JNIEnv, _: JClass) {}
}
