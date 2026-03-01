// Zedra — universal mobile application (Android + iOS)
// Platform-specific code is gated with #[cfg(target_os)]

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
pub mod session_panel;
pub mod terminal_panel;

// Transport badge (P2P / Relay indicator) + format_bytes utility
pub mod transport_badge;

// Shared Zedra app (screen navigation + connection)
pub mod app;

// Standalone preview app for GPU stress-testing
pub mod app_preview;

// File explorer view
pub mod file_explorer;

// Platform abstraction (trait + StubBridge fallback)
pub mod platform_bridge;

// Embedded assets (SVG icons) — shared across platforms
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "assets"]
#[include = "icons/*.svg"]
pub struct ZedraAssets;

impl gpui::AssetSource for ZedraAssets {
    fn load(&self, path: &str) -> gpui::Result<Option<std::borrow::Cow<'static, [u8]>>> {
        Ok(Self::get(path).map(|f| f.data))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<gpui::SharedString>> {
        Ok(Self::iter()
            .filter(|name| name.starts_with(path))
            .map(|name| name.into())
            .collect())
    }
}

// --- Android-only modules ---

#[cfg(target_os = "android")]
pub mod android;

// --- iOS-only modules ---

#[cfg(target_os = "ios")]
pub mod ios;
