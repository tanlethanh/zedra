// Zedra Android application - GPUI on Android via wgpu/Vulkan

// Generic async→main-thread channel
pub mod pending;

// Keyboard show/hide handler factory
pub mod keyboard;

// Shared color constants (Figma palette)
pub mod theme;

// Editor: text buffer, syntax highlighting, code editor view
pub mod editor;

// Mobile GPUI primitives: navigation, gestures, input
pub mod mgpui;

// Home screen
pub mod home_view;

// App drawer (header + file tree + footer nav icons)
pub mod app_drawer;

// Extracted drawer tab panels
pub mod terminal_panel;
pub mod session_panel;

// Transport badge (P2P / Relay indicator) + format_bytes utility
pub mod transport_badge;

// Shared Zedra app (screen navigation + connection)
pub mod app;

// File explorer view
pub mod file_explorer;

// Platform abstraction (trait + StubBridge fallback)
pub mod platform_bridge;

// Android platform integration (JNI, command queue, app, bridge)
#[cfg(target_os = "android")]
pub mod android;
