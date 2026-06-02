// Zedra — universal mobile application (Android + iOS)
// Platform-specific code is gated with #[cfg(target_os)]

// Shared Zedra app (screen navigation + connection)
pub mod app;
pub mod app_action;

// Generic async→main-thread channel
pub mod pending;

// Components
pub mod agent;
pub mod agent_detail;
pub mod agent_manage;
pub mod agent_picker;
pub mod agent_sessions;
mod agent_ui;
pub mod button;
pub mod docs_tree;
pub mod editor;
pub mod fonts;
pub mod placeholder;
pub mod settings;
pub mod theme;
pub mod ui;

// Sceens
pub mod home_view;
pub mod settings_view;

// Semantic components
pub mod file_explorer;
pub mod file_preview_view;
pub mod file_search;
pub mod git_panel;
pub mod quick_action_panel;
pub mod session_panel;
pub mod sheet_demo_state;
pub mod sheet_demo_view;
pub mod sheet_host_view;
pub mod terminal_card;
pub mod terminal_panel;
pub mod terminal_state;
pub mod transport_badge;

// Per-session workspace
pub mod workspace;
pub mod workspace_action;
pub mod workspace_connecting;
pub mod workspace_drawer;
pub mod workspace_editor;
pub mod workspace_gitdiff;
pub mod workspace_state;
pub mod workspace_terminal;
pub mod workspaces;

pub mod deeplink;
pub mod native_presentation;
pub mod platform_bridge;
pub mod telemetry;

// Embedded assets (SVG icons) — shared across platforms
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "assets"]
#[include = "icons/*.svg"]
pub struct ZedraAssets;

/// GPUI's SVG renderer loads these paths when rasterizing diagram labels.
#[derive(RustEmbed)]
#[folder = "../../vendor/zed/assets"]
#[include = "fonts/**"]
struct ZedraSvgFonts;

impl gpui::AssetSource for ZedraAssets {
    fn load(&self, path: &str) -> gpui::Result<Option<std::borrow::Cow<'static, [u8]>>> {
        if let Some(bytes) = editor::mermaid::load_mermaid_svg(path) {
            return Ok(Some(std::borrow::Cow::Owned(bytes.to_vec())));
        }
        if let Some(file) = Self::get(path) {
            return Ok(Some(file.data));
        }
        Ok(ZedraSvgFonts::get(path).map(|file| file.data))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<gpui::SharedString>> {
        let mut names = Self::iter()
            .filter(|name| name.starts_with(path))
            .map(|name| name.into())
            .collect::<Vec<_>>();
        names.extend(
            ZedraSvgFonts::iter()
                .filter(|name| name.starts_with(path))
                .map(|name| name.into()),
        );
        Ok(names)
    }
}

/// Install a panic hook that logs panics via `tracing::error!`.
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

        tracing::error!("PANIC at {}: {}", location, payload);

        // Forward to Crashlytics as a non-fatal event.
        // In release builds (panic = "abort") this line is never reached —
        // the native abort signal is captured directly by the Crashlytics NDK
        // / iOS crash handler as a fatal crash with a full native stack trace.
        telemetry::record_panic(&payload, &location);
    }));
}

// --- Android-only modules ---

#[cfg(target_os = "android")]
pub mod android;

// --- iOS-only modules ---

#[cfg(target_os = "ios")]
pub mod ios;
