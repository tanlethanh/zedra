// allow: SIZE_OK — app settings schema, preference persistence, and regression test suite
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI8, Ordering};

use gpui::{App, Context, Entity, EventEmitter, Global, WeakEntity};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::theme::{ThemeBundle, ThemePreference};

const STORE_DIR: &str = "zedra";
const SETTINGS_FILE: &str = "settings.json";

pub const TERMINAL_FONT_SIZE_DEFAULT: u8 = 12;
pub const TERMINAL_FONT_SIZE_MIN: u8 = 9;
pub const TERMINAL_FONT_SIZE_MAX: u8 = 24;

pub const fn normalize_terminal_font_size(value: u8) -> u8 {
    if value < TERMINAL_FONT_SIZE_MIN {
        TERMINAL_FONT_SIZE_MIN
    } else if value > TERMINAL_FONT_SIZE_MAX {
        TERMINAL_FONT_SIZE_MAX
    } else {
        value
    }
}

fn deserialize_terminal_font_size<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value: Option<serde_json::Value> = Option::<serde_json::Value>::deserialize(deserializer)?;
    let Some(value) = value else {
        return Ok(None);
    };
    match value {
        serde_json::Value::Number(n) if n.is_i64() => Ok(n.as_i64()),
        serde_json::Value::Number(n) if n.is_u64() => {
            let v = n.as_u64().unwrap_or(0);
            if v > i64::MAX as u64 {
                Ok(None)
            } else {
                Ok(Some(v as i64))
            }
        }
        _ => Ok(None),
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct AppSettings {
    /// Set when the user picks a theme in Settings; `None` follows the system on next launch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    theme_preference: Option<ThemePreference>,
    /// Opt-out flag for anonymous telemetry. `None`/absent = enabled (default-on).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    telemetry_enabled: Option<bool>,
    /// Water droplet effect. `None`/absent = disabled (default-off).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    droplet_enabled: Option<bool>,
    /// Keep the terminal key bar on screen while the keyboard is collapsed.
    /// `None`/absent = disabled (default-off).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    key_bar_always_visible: Option<bool>,
    /// Two-row keypad with modifiers and a composing field.
    /// `None`/absent = disabled (default-off).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    extended_keypad: Option<bool>,
    #[serde(
        default,
        deserialize_with = "deserialize_terminal_font_size",
        skip_serializing_if = "Option::is_none"
    )]
    terminal_font_size: Option<i64>,
    #[serde(flatten)]
    extra: BTreeMap<String, serde_json::Value>,
}

impl AppSettings {
    fn sanitize_extra(&mut self) {
        const KNOWN_KEYS: &[&str] = &[
            "theme_preference",
            "telemetry_enabled",
            "droplet_enabled",
            "key_bar_always_visible",
            "extended_keypad",
            "terminal_font_size",
        ];
        for key in KNOWN_KEYS {
            self.extra.remove(*key);
        }
    }
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

#[cfg(test)]
static TEST_DATA_DIRECTORY: std::sync::OnceLock<Mutex<Option<PathBuf>>> =
    std::sync::OnceLock::new();

#[cfg(test)]
fn test_data_directory() -> &'static Mutex<Option<PathBuf>> {
    TEST_DATA_DIRECTORY.get_or_init(|| Mutex::new(None))
}

fn data_directory() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(dir) = test_data_directory().lock().ok().and_then(|g| g.clone()) {
        return Some(dir);
    }

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
    let mut settings: AppSettings = serde_json::from_str(&contents).map_err(|e| e.to_string())?;
    settings.sanitize_extra();
    Ok(settings)
}

fn write_settings(settings: &AppSettings) -> Result<(), String> {
    let path = settings_path().ok_or_else(|| "settings path unavailable".to_string())?;
    let mut settings = settings.clone();
    settings.sanitize_extra();
    let contents = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    std::fs::write(path, contents).map_err(|e| e.to_string())
}

const fn terminal_font_size_from_raw(raw: Option<i64>) -> u8 {
    let Some(value) = raw else {
        return TERMINAL_FONT_SIZE_DEFAULT;
    };

    let clamped = if value < TERMINAL_FONT_SIZE_MIN as i64 {
        TERMINAL_FONT_SIZE_MIN
    } else if value > TERMINAL_FONT_SIZE_MAX as i64 {
        TERMINAL_FONT_SIZE_MAX
    } else {
        value as u8
    };
    normalize_terminal_font_size(clamped)
}

fn read_terminal_font_size_pref(settings: &AppSettings) -> u8 {
    terminal_font_size_from_raw(settings.terminal_font_size)
}

static TERMINAL_FONT_SIZE_CACHE: AtomicI8 = AtomicI8::new(-1);

pub fn read_terminal_font_size() -> u8 {
    match read_settings() {
        Ok(settings) => read_terminal_font_size_pref(&settings),
        Err(err) => {
            info!(err = %err, "settings: using default terminal font size");
            TERMINAL_FONT_SIZE_DEFAULT
        }
    }
}

pub fn terminal_font_size() -> u8 {
    match TERMINAL_FONT_SIZE_CACHE.load(Ordering::Relaxed) {
        -1 => {
            let size = read_terminal_font_size();
            TERMINAL_FONT_SIZE_CACHE.store(size as i8, Ordering::Relaxed);
            size
        }
        cached => normalize_terminal_font_size(cached as u8),
    }
}

static FONT_SIZE_LISTENERS: Mutex<Vec<Box<dyn Fn(u8) + Send + Sync + 'static>>> =
    Mutex::new(Vec::new());

pub fn on_terminal_font_size_changed(callback: impl Fn(u8) + Send + Sync + 'static) {
    if let Ok(mut listeners) = FONT_SIZE_LISTENERS.lock() {
        listeners.push(Box::new(callback));
    }
}

pub fn set_terminal_font_size(size: u8) -> bool {
    let accepted = normalize_terminal_font_size(size);
    let mut settings = read_settings().unwrap_or_default();
    let current = read_terminal_font_size_pref(&settings);
    if current == accepted {
        TERMINAL_FONT_SIZE_CACHE.store(accepted as i8, Ordering::Relaxed);
        return false;
    }
    TERMINAL_FONT_SIZE_CACHE.store(accepted as i8, Ordering::Relaxed);
    settings.terminal_font_size = Some(accepted as i64);
    if let Err(err) = write_settings(&settings) {
        warn!(err = %err, "settings: failed to save terminal font size");
        return false;
    }
    if let Ok(listeners) = FONT_SIZE_LISTENERS.lock() {
        for listener in listeners.iter() {
            listener(accepted);
        }
    }
    true
}

#[cfg(test)]
pub(crate) fn reset_terminal_font_size_cache_for_tests() {
    TERMINAL_FONT_SIZE_CACHE.store(-1, Ordering::Relaxed);
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

/// Whether the water droplet effect is enabled. Default off.
pub fn read_droplet_enabled() -> bool {
    match read_settings() {
        Ok(settings) => settings.droplet_enabled.unwrap_or(false),
        Err(err) => {
            info!(err = %err, "settings: using default droplet preference");
            false
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

/// Reads a boolean setting once and caches it. `-1` means "not loaded yet"; the
/// keypad and terminal read these on paths that run every frame, so the disk read
/// must not repeat.
fn cached_flag(cache: &AtomicI8, read: impl FnOnce() -> bool) -> bool {
    match cache.load(Ordering::Relaxed) {
        0 => false,
        1 => true,
        _ => {
            let enabled = read();
            cache.store(enabled as i8, Ordering::Relaxed);
            enabled
        }
    }
}

fn read_flag(name: &str, default: bool, field: impl FnOnce(&AppSettings) -> Option<bool>) -> bool {
    match read_settings() {
        Ok(settings) => field(&settings).unwrap_or(default),
        Err(err) => {
            info!(err = %err, setting = name, "settings: using default preference");
            default
        }
    }
}

static KEY_BAR_ALWAYS_VISIBLE: AtomicI8 = AtomicI8::new(-1);

/// Whether the terminal key bar stays on screen once the keyboard collapses.
/// Default off.
pub fn key_bar_always_visible() -> bool {
    cached_flag(&KEY_BAR_ALWAYS_VISIBLE, || {
        read_flag("key_bar_always_visible", false, |s| {
            s.key_bar_always_visible
        })
    })
}

static EXTENDED_KEYPAD: AtomicI8 = AtomicI8::new(-1);

/// Whether the terminal keypad shows the two-row layout with modifier keys and
/// the composing field. Default off.
pub fn extended_keypad() -> bool {
    cached_flag(&EXTENDED_KEYPAD, || {
        read_flag("extended_keypad", false, |s| s.extended_keypad)
    })
}

pub fn set_extended_keypad(enabled: bool) {
    EXTENDED_KEYPAD.store(enabled as i8, Ordering::Relaxed);
    let mut settings = read_settings().unwrap_or_default();
    settings.extended_keypad = Some(enabled);
    if let Err(err) = write_settings(&settings) {
        warn!(err = %err, "settings: failed to save extended keypad preference");
    }
}

pub fn set_key_bar_always_visible(enabled: bool) {
    KEY_BAR_ALWAYS_VISIBLE.store(enabled as i8, Ordering::Relaxed);
    let mut settings = read_settings().unwrap_or_default();
    settings.key_bar_always_visible = Some(enabled);
    if let Err(err) = write_settings(&settings) {
        warn!(err = %err, "settings: failed to save key bar preference");
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::theme::{ThemeBundle, ThemePalette, ThemePreference};

    static TEST_SERIAL_LOCK: Mutex<()> = Mutex::new(());

    struct TestDirectoryScope {
        dir: PathBuf,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl TestDirectoryScope {
        fn new(name: &str) -> Self {
            let lock = TEST_SERIAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "zedra_settings_{}_{}_{}",
                name,
                std::process::id(),
                id
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("create test temp dir");
            *test_data_directory()
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = Some(dir.clone());
            reset_terminal_font_size_cache_for_tests();
            Self { dir, _lock: lock }
        }

        fn settings_file(&self) -> PathBuf {
            self.dir.join(STORE_DIR).join(SETTINGS_FILE)
        }
    }

    impl Drop for TestDirectoryScope {
        fn drop(&mut self) {
            *test_data_directory()
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = None;
            reset_terminal_font_size_cache_for_tests();
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

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

    #[test]
    fn existing_optional_settings_round_trip_current_schema() {
        let settings: AppSettings = serde_json::from_value(serde_json::json!({
            "theme_preference": "light",
            "telemetry_enabled": false,
            "droplet_enabled": true,
            "key_bar_always_visible": true,
            "extended_keypad": true,
        }))
        .expect("known optional settings should load");

        let serialized =
            serde_json::to_value(settings).expect("known optional settings should save");
        assert_eq!(serialized["theme_preference"], "light");
        assert_eq!(serialized["telemetry_enabled"], false);
        assert_eq!(serialized["droplet_enabled"], true);
        assert_eq!(serialized["key_bar_always_visible"], true);
        assert_eq!(serialized["extended_keypad"], true);
    }

    #[test]
    fn terminal_font_size_defaults_to_12_when_missing() {
        let settings: AppSettings = serde_json::from_value(serde_json::json!({
            "telemetry_enabled": true,
        }))
        .expect("settings missing font size should load");
        assert_eq!(read_terminal_font_size_pref(&settings), 12);
    }

    #[test]
    fn terminal_font_size_clamps_below_min_to_9() {
        for below_min in [-100, -1, 0, 5, 8] {
            let settings: AppSettings = serde_json::from_value(serde_json::json!({
                "terminal_font_size": below_min,
            }))
            .expect("negative/small integers should parse without error");
            assert_eq!(read_terminal_font_size_pref(&settings), 9);
        }
    }

    #[test]
    fn terminal_font_size_clamps_above_max_to_24() {
        for above_max in [25, 30, 100, 1000] {
            let settings: AppSettings = serde_json::from_value(serde_json::json!({
                "terminal_font_size": above_max,
            }))
            .expect("oversized integers should parse without error");
            assert_eq!(read_terminal_font_size_pref(&settings), 24);
        }
    }

    #[test]
    fn terminal_font_size_tolerant_deserialization_handles_malformed_values() {
        let malformed_cases = vec![
            serde_json::json!("large"),
            serde_json::json!("15"),
            serde_json::json!({"custom_size": 15}),
            serde_json::json!([12, 14]),
            serde_json::json!(true),
            serde_json::json!(false),
            serde_json::json!(null),
            serde_json::json!(15.5),
        ];

        for malformed in malformed_cases {
            let settings: AppSettings = serde_json::from_value(serde_json::json!({
                "telemetry_enabled": false,
                "terminal_font_size": malformed,
                "unknown_flag": "preserved",
            }))
            .expect("malformed font size must never fail entire settings parse");

            assert_eq!(settings.telemetry_enabled, Some(false));
            assert_eq!(read_terminal_font_size_pref(&settings), 12);
            assert_eq!(
                settings.extra.get("unknown_flag"),
                Some(&serde_json::json!("preserved"))
            );
        }
    }

    #[test]
    fn terminal_font_size_normal_15_round_trips() {
        let settings: AppSettings = serde_json::from_value(serde_json::json!({
            "terminal_font_size": 15,
        }))
        .expect("integer font size 15 should parse");

        assert_eq!(read_terminal_font_size_pref(&settings), 15);
        let serialized = serde_json::to_value(settings).expect("save settings");
        assert_eq!(serialized["terminal_font_size"], 15);
    }

    #[test]
    fn unknown_keys_preserved_semantically_including_nested_structures() {
        let original_json = serde_json::json!({
            "telemetry_enabled": false,
            "custom_number": 42,
            "custom_text": "hello",
            "nested_dict": {
                "inner_list": [1, "two", {"deep": true}],
                "score": 9.75
            }
        });

        let mut settings: AppSettings =
            serde_json::from_value(original_json.clone()).expect("parse settings");
        settings.terminal_font_size = Some(15);
        let serialized = serde_json::to_value(settings).expect("serialize settings");

        assert_eq!(serialized["telemetry_enabled"], false);
        assert_eq!(serialized["terminal_font_size"], 15);
        assert_eq!(serialized["custom_number"], 42);
        assert_eq!(serialized["custom_text"], "hello");
        assert_eq!(
            serialized["nested_dict"],
            serde_json::json!({
                "inner_list": [1, "two", {"deep": true}],
                "score": 9.75
            })
        );
    }

    #[test]
    fn flattened_map_cannot_shadow_known_fields() {
        let mut settings = AppSettings {
            theme_preference: Some(ThemePreference::Light),
            telemetry_enabled: Some(true),
            terminal_font_size: Some(16),
            extra: BTreeMap::new(),
            ..Default::default()
        };
        settings
            .extra
            .insert("theme_preference".to_string(), serde_json::json!("dark"));
        settings
            .extra
            .insert("terminal_font_size".to_string(), serde_json::json!(999));
        settings.sanitize_extra();

        let serialized = serde_json::to_value(&settings).expect("serialize settings");
        assert_eq!(serialized["theme_preference"], "light");
        assert_eq!(serialized["terminal_font_size"], 16);
    }

    #[test]
    fn setter_writes_only_when_accepted_integer_changes() {
        let scope = TestDirectoryScope::new("setter_changed_value");
        assert_eq!(terminal_font_size(), 12);
        assert_eq!(set_terminal_font_size(12), false);
        assert!(!scope.settings_file().exists());

        assert_eq!(set_terminal_font_size(15), true);
        assert_eq!(terminal_font_size(), 15);
        assert!(scope.settings_file().exists());
        let on_disk_15: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(scope.settings_file()).unwrap()).unwrap();
        assert_eq!(on_disk_15["terminal_font_size"], 15);

        assert_eq!(set_terminal_font_size(15), false);

        assert_eq!(set_terminal_font_size(30), true);
        assert_eq!(terminal_font_size(), 24);
        let on_disk_24: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(scope.settings_file()).unwrap()).unwrap();
        assert_eq!(on_disk_24["terminal_font_size"], 24);

        assert_eq!(set_terminal_font_size(24), false);
        assert_eq!(set_terminal_font_size(25), false);

        assert_eq!(set_terminal_font_size(5), true);
        assert_eq!(terminal_font_size(), 9);
        let on_disk_9: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(scope.settings_file()).unwrap()).unwrap();
        assert_eq!(on_disk_9["terminal_font_size"], 9);

        assert_eq!(set_terminal_font_size(9), false);
        assert_eq!(set_terminal_font_size(7), false);
    }

    #[test]
    fn listener_notified_on_font_size_change() {
        let _scope = TestDirectoryScope::new("listener_notify");
        let notified = Arc::new(AtomicUsize::new(0));
        let notified_clone = notified.clone();
        on_terminal_font_size_changed(move |size| {
            notified_clone.store(size as usize, Ordering::Relaxed);
        });

        assert_eq!(set_terminal_font_size(18), true);
        assert_eq!(notified.load(Ordering::Relaxed), 18);
    }

    #[test]
    fn manual_qa_channel_with_temp_settings_json() {
        let scope = TestDirectoryScope::new("manual_qa");
        let initial_json = serde_json::json!({
            "theme_preference": "light",
            "telemetry_enabled": false,
            "terminal_font_size": {"malformed": true, "items": [1, 2]},
            "unknown_feature": {
                "metadata": "alpha",
                "nested_items": ["x", "y", {"deep_flag": true}]
            }
        });
        std::fs::create_dir_all(scope.settings_file().parent().unwrap()).unwrap();
        std::fs::write(
            scope.settings_file(),
            serde_json::to_string_pretty(&initial_json).unwrap(),
        )
        .unwrap();

        assert_eq!(read_terminal_font_size(), 12);
        assert_eq!(terminal_font_size(), 12);

        assert_eq!(set_terminal_font_size(15), true);
        assert_eq!(terminal_font_size(), 15);

        let final_content = std::fs::read_to_string(scope.settings_file()).unwrap();
        let final_value: serde_json::Value = serde_json::from_str(&final_content).unwrap();

        assert_eq!(final_value["theme_preference"], "light");
        assert_eq!(final_value["telemetry_enabled"], false);
        assert_eq!(final_value["terminal_font_size"], 15);
        assert_eq!(
            final_value["unknown_feature"],
            serde_json::json!({
                "metadata": "alpha",
                "nested_items": ["x", "y", {"deep_flag": true}]
            })
        );
    }
}
