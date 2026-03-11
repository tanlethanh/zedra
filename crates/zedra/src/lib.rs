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

// Workspace drawer (tabs: Files / Git / Terminal / Session)
pub mod workspace_drawer;

// Extracted drawer tab panels
pub mod session_panel;
pub mod terminal_panel;

// Per-session workspace view (DrawerHost + WorkspaceContent + WorkspaceDrawer)
pub mod workspace_view;

// Quick-action right overlay (workspace switcher + home button)
pub mod quick_action_panel;

// Transport badge (P2P / Relay indicator) + format_bytes utility
pub mod transport_badge;

// Firebase Analytics + Crashlytics (platform-agnostic API)
pub mod analytics;

// Shared Zedra app (screen navigation + connection)
pub mod app;

// Standalone preview app for GPU stress-testing
pub mod app_preview;

// File explorer view
pub mod file_explorer;

// Platform abstraction (trait + StubBridge fallback)
pub mod platform_bridge;

// Workspace persistence (load/save workspace connection data)
pub mod workspace_store;

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

/// Install a panic hook that logs panics via `log::error!`.
/// Call this once during platform initialization, after the logger is set up.
pub fn install_panic_hook() {
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

        // Forward to Crashlytics as a non-fatal event.
        // In release builds (panic = "abort") this line is never reached —
        // the native abort signal is captured directly by the Crashlytics NDK
        // / iOS crash handler as a fatal crash with a full native stack trace.
        crate::analytics::record_panic(&payload, &location);
    }));
}

// --- Android-only modules ---

#[cfg(target_os = "android")]
pub mod android;

// --- iOS-only modules ---

#[cfg(target_os = "ios")]
pub mod ios;
