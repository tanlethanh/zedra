// App telemetry — registers the platform Firebase backend with zedra-telemetry.
//
// Call `init()` once at app startup (before any events fire).
// After init, all crates can use `zedra_telemetry::send(Event::...)` etc.

#[cfg(target_os = "ios")]
use crate::ios::telemetry as ios_telemetry;

/// Platform-specific Firebase backend that implements TelemetryBackend.
struct FirebaseBackend;

impl zedra_telemetry::TelemetryBackend for FirebaseBackend {
    fn send(&self, event: &zedra_telemetry::Event) {
        let name = event.name();
        let params = event.to_params();
        #[cfg(feature = "debug-telemetry")]
        {
            let kv: Vec<String> = params.iter().map(|(k, v)| format!("{}={}", k, v)).collect();
            eprintln!("[telemetry] >> {} {}", name, kv.join(" "));
        }
        let param_refs: Vec<(&str, &str)> = params.iter().map(|(k, v)| (*k, v.as_str())).collect();
        #[cfg(target_os = "ios")]
        ios_telemetry::log_event(name, &param_refs);
        // Android Firebase not yet implemented; events are no-ops on Android.
        let _ = (name, param_refs);
    }

    fn record_error(&self, message: &str, file: &str, line: u32) {
        #[cfg(target_os = "ios")]
        ios_telemetry::record_error(message, file, line);
        let _ = (message, file, line);
    }

    fn record_panic(&self, message: &str, location: &str) {
        #[cfg(feature = "debug-telemetry")]
        eprintln!("[telemetry] panic: {} at {}", message, location);
        #[cfg(target_os = "ios")]
        ios_telemetry::record_panic(message, location);
        let _ = (message, location);
    }

    fn set_user_id(&self, id: &str) {
        #[cfg(target_os = "ios")]
        ios_telemetry::set_user_id(id);
        let _ = id;
    }

    fn set_custom_key(&self, key: &str, value: &str) {
        #[cfg(target_os = "ios")]
        ios_telemetry::set_custom_key(key, value);
        let _ = (key, value);
    }

    fn set_collection_enabled(&self, enabled: bool) {
        #[cfg(target_os = "ios")]
        ios_telemetry::set_collection_enabled(enabled);
        let _ = enabled;
    }
}

/// Register the Firebase backend with the shared telemetry crate.
/// Call once at app startup before any events fire.
pub fn init() {
    let _ = zedra_telemetry::init(Box::new(FirebaseBackend));
}

// Re-export for convenience so existing call sites don't need to change imports.
pub use zedra_telemetry::{
    is_enabled, record_error, record_panic, set_custom_key, set_enabled, set_user_id,
};

pub mod view_telemetry {
    use crate::editor::markdown::is_markdown_path;
    use crate::workspace_state::WorkspaceMainView;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct ViewDescriptor {
        pub screen: &'static str,
        pub screen_name: &'static str,
        pub screen_class: &'static str,
    }

    impl ViewDescriptor {
        pub const fn new(
            screen: &'static str,
            screen_name: &'static str,
            screen_class: &'static str,
        ) -> Self {
            Self {
                screen,
                screen_name,
                screen_class,
            }
        }
    }

    pub const HOME: ViewDescriptor = ViewDescriptor::new("home", "Home", "HomeView");
    pub const SETTINGS: ViewDescriptor =
        ViewDescriptor::new("settings", "Settings", "SettingsView");
    pub const QUICK_ACTIONS: ViewDescriptor =
        ViewDescriptor::new("quick_actions", "Quick Actions", "QuickActionPanel");

    pub const WORKSPACE_CONNECTING: ViewDescriptor = ViewDescriptor::new(
        "workspace_connecting",
        "Workspace Connecting",
        "WorkspaceConnecting",
    );
    pub const WORKSPACE_TERMINAL: ViewDescriptor = ViewDescriptor::new(
        "workspace_terminal",
        "Workspace Terminal",
        "WorkspaceTerminal",
    );
    pub const WORKSPACE_EDITOR: ViewDescriptor =
        ViewDescriptor::new("workspace_editor", "Workspace Editor", "WorkspaceEditor");
    pub const WORKSPACE_MARKDOWN: ViewDescriptor = ViewDescriptor::new(
        "workspace_markdown",
        "Workspace Markdown",
        "WorkspaceEditor",
    );
    pub const WORKSPACE_GIT_DIFF: ViewDescriptor = ViewDescriptor::new(
        "workspace_git_diff",
        "Workspace Git Diff",
        "WorkspaceGitdiff",
    );
    pub const WORKSPACE_NO_ACTIVE_TERMINAL: ViewDescriptor = ViewDescriptor::new(
        "workspace_no_active_terminal",
        "Workspace No Active Terminal",
        "NoActiveTerminalView",
    );

    pub const DRAWER_FILES: ViewDescriptor =
        ViewDescriptor::new("drawer_files", "Drawer Files", "WorkspaceDrawer");
    pub const DRAWER_DOCUMENTS: ViewDescriptor =
        ViewDescriptor::new("drawer_documents", "Drawer Documents", "WorkspaceDrawer");
    pub const DRAWER_GIT_DIFF: ViewDescriptor =
        ViewDescriptor::new("drawer_git_diff", "Drawer Git Diff", "WorkspaceDrawer");
    pub const DRAWER_TERMINALS: ViewDescriptor =
        ViewDescriptor::new("drawer_terminals", "Drawer Terminals", "WorkspaceDrawer");
    pub const DRAWER_SESSION: ViewDescriptor =
        ViewDescriptor::new("drawer_session", "Drawer Session", "WorkspaceDrawer");

    pub const CUSTOM_SHEET_EDITOR: ViewDescriptor = ViewDescriptor::new(
        "custom_sheet_editor",
        "Custom Sheet Editor",
        "TerminalPreviewView",
    );
    pub const CUSTOM_SHEET_MARKDOWN: ViewDescriptor = ViewDescriptor::new(
        "custom_sheet_markdown",
        "Custom Sheet Markdown",
        "TerminalPreviewView",
    );
    pub const CUSTOM_SHEET_DEMO: ViewDescriptor =
        ViewDescriptor::new("custom_sheet_demo", "Custom Sheet Demo", "SheetDemoView");

    pub fn record(screen: ViewDescriptor) {
        zedra_telemetry::send(zedra_telemetry::Event::ScreenView {
            screen: screen.screen,
            screen_name: screen.screen_name,
            screen_class: screen.screen_class,
        });
    }

    pub fn workspace_file(path: &str) -> ViewDescriptor {
        if is_markdown_path(path) {
            WORKSPACE_MARKDOWN
        } else {
            WORKSPACE_EDITOR
        }
    }

    pub fn workspace_main_view(view: &WorkspaceMainView) -> Option<ViewDescriptor> {
        match view {
            WorkspaceMainView::Default => None,
            WorkspaceMainView::File { path } => Some(workspace_file(path)),
            WorkspaceMainView::GitDiff { .. } => Some(WORKSPACE_GIT_DIFF),
            WorkspaceMainView::Terminal { .. } => Some(WORKSPACE_TERMINAL),
            WorkspaceMainView::NoActiveTerminal => Some(WORKSPACE_NO_ACTIVE_TERMINAL),
        }
    }

    pub fn custom_sheet_file(path: &str) -> ViewDescriptor {
        if is_markdown_path(path) {
            CUSTOM_SHEET_MARKDOWN
        } else {
            CUSTOM_SHEET_EDITOR
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn workspace_file_views_split_code_and_markdown() {
            assert_eq!(workspace_file("/repo/src/main.rs"), WORKSPACE_EDITOR);
            assert_eq!(workspace_file("/repo/README.md"), WORKSPACE_MARKDOWN);
            assert_eq!(workspace_file("/repo/readme"), WORKSPACE_MARKDOWN);
        }

        #[test]
        fn workspace_main_view_maps_to_logical_views() {
            assert_eq!(
                workspace_main_view(&WorkspaceMainView::File {
                    path: "/repo/README.md".into(),
                }),
                Some(WORKSPACE_MARKDOWN)
            );
            assert_eq!(
                workspace_main_view(&WorkspaceMainView::GitDiff {
                    path: "src/main.rs".into(),
                    section: 1,
                }),
                Some(WORKSPACE_GIT_DIFF)
            );
            assert_eq!(
                workspace_main_view(&WorkspaceMainView::Terminal {
                    id: "terminal-1".into(),
                }),
                Some(WORKSPACE_TERMINAL)
            );
            assert_eq!(
                workspace_main_view(&WorkspaceMainView::NoActiveTerminal),
                Some(WORKSPACE_NO_ACTIVE_TERMINAL)
            );
            assert_eq!(workspace_main_view(&WorkspaceMainView::Default), None);
        }

        #[test]
        fn custom_sheet_file_views_split_code_and_markdown() {
            assert_eq!(custom_sheet_file("/repo/src/main.rs"), CUSTOM_SHEET_EDITOR);
            assert_eq!(
                custom_sheet_file("/repo/docs/guide.markdown"),
                CUSTOM_SHEET_MARKDOWN
            );
        }
    }
}
