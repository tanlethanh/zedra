use std::path::PathBuf;

use gpui::{App, Context, Entity, EventEmitter, Global, WeakEntity};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::theme::{ThemeBundle, ThemePreference};

const STORE_DIR: &str = "zedra";
const SETTINGS_FILE: &str = "settings.json";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct AppSettings {
    /// Set when the user picks a theme in Settings; `None` follows the system on next launch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    theme_preference: Option<ThemePreference>,
    /// Opt-out flag for anonymous telemetry. `None`/absent = enabled (default-on).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    telemetry_enabled: Option<bool>,
    /// Water droplet effect. `None`/absent = enabled (default-on).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    droplet_enabled: Option<bool>,
}

pub enum ThemeStateEvent {
    Changed,
}

impl EventEmitter<ThemeStateEvent> for ThemeState {}

pub struct ThemeState {
    preference: ThemePreference,
    bundle: ThemeBundle,
}

impl ThemeState {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        let preference = Self::load_preference();
        let bundle = ThemeBundle::for_preference(preference);
        Self::sync_native_theme(preference);
        Self { preference, bundle }
    }

    pub fn preference(&self) -> ThemePreference {
        self.preference
    }

    pub fn bundle(&self) -> &ThemeBundle {
        &self.bundle
    }

    pub fn palette(&self) -> &crate::theme::ThemePalette {
        &self.bundle.ui
    }

    pub fn set_preference(&mut self, preference: ThemePreference, cx: &mut Context<Self>) {
        if self.preference == preference {
            return;
        }
        self.preference = preference;
        self.bundle = ThemeBundle::for_preference(preference);
        Self::sync_native_theme(preference);
        Self::save_preference(preference);
        cx.emit(ThemeStateEvent::Changed);
        cx.notify();
    }

    pub fn register_global(entity: WeakEntity<Self>, cx: &mut App) {
        cx.set_global(ThemeStateHandle(entity));
    }

    fn load_preference() -> ThemePreference {
        match read_settings() {
            Ok(settings) => settings.theme_preference.unwrap_or_default(),
            Err(err) => {
                info!(err = %err, "settings: using default theme preference");
                ThemePreference::default()
            }
        }
    }

    pub(crate) fn preference_from_system() -> ThemePreference {
        match crate::platform_bridge::bridge().system_prefers_theme() {
            crate::platform_bridge::SystemTheme::Dark => ThemePreference::Dark,
            crate::platform_bridge::SystemTheme::Light => ThemePreference::Light,
            crate::platform_bridge::SystemTheme::Unknown => ThemePreference::default(),
        }
    }

    fn save_preference(preference: ThemePreference) {
        let mut settings = read_settings().unwrap_or_default();
        settings.theme_preference = Some(preference);
        if let Err(err) = write_settings(&settings) {
            warn!(err = %err, "settings: failed to save theme preference");
        }
    }

    fn sync_native_theme(preference: ThemePreference) {
        crate::platform_bridge::bridge().set_native_theme(preference == ThemePreference::Dark);
    }
}

#[derive(Clone)]
pub struct ThemeStateHandle(WeakEntity<ThemeState>);

impl Global for ThemeStateHandle {}

pub fn theme_state(cx: &App) -> Option<Entity<ThemeState>> {
    cx.try_global::<ThemeStateHandle>()
        .and_then(|handle| handle.0.upgrade())
}

pub fn palette(cx: &App) -> crate::theme::ThemePalette {
    theme_state(cx)
        .map(|theme| theme.read(cx).palette().clone())
        .unwrap_or_else(|| ThemeBundle::dark().ui)
}

pub fn bundle(cx: &App) -> ThemeBundle {
    theme_state(cx)
        .map(|theme| theme.read(cx).bundle().clone())
        .unwrap_or_else(ThemeBundle::dark)
}

fn data_directory() -> Option<PathBuf> {
    crate::platform_bridge::bridge()
        .data_directory()
        .map(PathBuf::from)
}

fn settings_path() -> Option<PathBuf> {
    let dir = data_directory()?.join(STORE_DIR);
    if !dir.exists() {
        std::fs::create_dir_all(&dir).ok()?;
    }
    Some(dir.join(SETTINGS_FILE))
}

fn read_settings() -> Result<AppSettings, String> {
    let path = settings_path().ok_or_else(|| "settings path unavailable".to_string())?;
    if !path.exists() {
        return Ok(AppSettings::default());
    }
    let contents = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    serde_json::from_str(&contents).map_err(|e| e.to_string())
}

fn write_settings(settings: &AppSettings) -> Result<(), String> {
    let path = settings_path().ok_or_else(|| "settings path unavailable".to_string())?;
    let contents = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    std::fs::write(path, contents).map_err(|e| e.to_string())
}

/// Whether anonymous telemetry is enabled. Absent/unreadable settings default to
/// enabled (opt-out model), matching the persisted `None` case.
pub fn read_telemetry_enabled() -> bool {
    match read_settings() {
        Ok(settings) => settings.telemetry_enabled.unwrap_or(true),
        Err(err) => {
            info!(err = %err, "settings: using default telemetry preference");
            true
        }
    }
}

/// Update the persisted telemetry preference and the shared runtime gate.
pub fn set_telemetry_enabled(enabled: bool) {
    // Update the shared runtime gate first so the new state takes effect
    // immediately, even if the settings write fails.
    zedra_telemetry::set_enabled(enabled);

    let mut settings = read_settings().unwrap_or_default();
    settings.telemetry_enabled = Some(enabled);
    if let Err(err) = write_settings(&settings) {
        warn!(err = %err, "settings: failed to save telemetry preference");
    }
}

/// Whether the water droplet effect is enabled. Default on.
pub fn read_droplet_enabled() -> bool {
    match read_settings() {
        Ok(settings) => settings.droplet_enabled.unwrap_or(true),
        Err(err) => {
            info!(err = %err, "settings: using default droplet preference");
            true
        }
    }
}

pub fn set_droplet_enabled(enabled: bool) {
    let mut settings = read_settings().unwrap_or_default();
    settings.droplet_enabled = Some(enabled);
    if let Err(err) = write_settings(&settings) {
        warn!(err = %err, "settings: failed to save droplet preference");
    }
}

#[cfg(test)]
mod tests {
    use super::ThemeState;
    use crate::theme::{ThemeBundle, ThemePalette, ThemePreference};

    #[test]
    fn default_preference_is_dark() {
        assert_eq!(ThemePreference::default(), ThemePreference::Dark);
    }

    #[test]
    fn preference_from_system_falls_back_to_dark_when_unknown() {
        // StubBridge returns Unknown for system_prefers_theme.
        assert_eq!(ThemeState::preference_from_system(), ThemePreference::Dark);
    }

    #[test]
    fn bundle_matches_preference() {
        assert_eq!(
            ThemeBundle::for_preference(ThemePreference::Light)
                .ui
                .bg_primary,
            ThemePalette::light().bg_primary
        );
    }
}
