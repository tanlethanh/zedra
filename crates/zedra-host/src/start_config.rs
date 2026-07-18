// start_config.rs — persisted daemon launch flags, written as `launch.yaml`
// next to `daemon.lock` so `zedra update` can relaunch daemons with the
// options they were originally started with. Distinct from the user-editable
// `config.yaml` (see `global_config.rs`): this file is machine-written state.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const FILE_NAME: &str = "launch.yaml";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StartConfig {
    pub verbose: bool,
    pub relay_url: Vec<String>,
    pub no_telemetry: bool,
    pub debug_telemetry: bool,
    pub relay_only: bool,
    pub static_qr: bool,
    pub usage_refresh_secs: u64,
}

impl Default for StartConfig {
    fn default() -> Self {
        Self {
            verbose: false,
            relay_url: Vec::new(),
            no_telemetry: false,
            debug_telemetry: false,
            relay_only: false,
            static_qr: false,
            usage_refresh_secs: 300,
        }
    }
}

/// Write the launch config into the workspace config directory.
pub fn save(config_dir: &Path, config: &StartConfig) -> Result<()> {
    std::fs::create_dir_all(config_dir)?;
    let yaml = serde_yaml::to_string(config)?;
    std::fs::write(config_dir.join(FILE_NAME), yaml)?;
    Ok(())
}

/// Missing or unreadable configs fall back to defaults so daemons started
/// before this file existed stay restartable.
pub fn load(config_dir: &Path) -> StartConfig {
    match std::fs::read_to_string(config_dir.join(FILE_NAME)) {
        Ok(contents) => serde_yaml::from_str(&contents).unwrap_or_else(|e| {
            tracing::warn!("start_config: failed to parse {FILE_NAME}, using defaults: {e}");
            StartConfig::default()
        }),
        Err(_) => StartConfig::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let config = StartConfig {
            verbose: true,
            relay_url: vec!["https://sg1.relay.zedra.dev".to_string()],
            no_telemetry: true,
            debug_telemetry: false,
            relay_only: true,
            static_qr: true,
            usage_refresh_secs: 60,
        };
        save(dir.path(), &config).unwrap();
        assert_eq!(load(dir.path()), config);
    }

    #[test]
    fn load_defaults_when_missing_or_malformed() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()), StartConfig::default());

        std::fs::write(dir.path().join(FILE_NAME), "relay_url: {not a list}").unwrap();
        assert_eq!(load(dir.path()), StartConfig::default());
    }
}
