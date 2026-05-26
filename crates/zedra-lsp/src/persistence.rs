//! On-disk persistence of the per-workspace LSP enablement list.
//!
//! Stored at `<workspace_config_dir>/lsp.json` alongside `sessions.json`. Kept
//! in its own file so the LSP subsystem can be added or removed without
//! touching `SessionRegistry` persistence (decoupling NFR).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use zedra_rpc::proto::LspLanguage;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedLspState {
    pub version: u32,
    pub enabled_languages: Vec<LspLanguage>,
}

impl Default for PersistedLspState {
    fn default() -> Self {
        Self {
            version: 1,
            enabled_languages: Vec::new(),
        }
    }
}

/// Load the persisted state from `path`, or return defaults on missing /
/// unreadable file. Parse errors are logged, not propagated — a corrupt
/// `lsp.json` should not block daemon start.
pub fn load(path: &PathBuf) -> PersistedLspState {
    let data = match std::fs::read_to_string(path) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return PersistedLspState::default(),
        Err(e) => {
            tracing::warn!("Failed to read lsp.json at {}: {}", path.display(), e);
            return PersistedLspState::default();
        }
    };
    match serde_json::from_str(&data) {
        Ok(state) => state,
        Err(e) => {
            tracing::warn!("Failed to parse lsp.json at {}: {}", path.display(), e);
            PersistedLspState::default()
        }
    }
}

/// Atomically persist `state` to `path`. Errors are logged, not propagated —
/// a save failure should never abort an RPC call.
pub fn save(path: &PathBuf, state: &PersistedLspState) {
    let Some(parent) = path.parent() else {
        tracing::warn!("lsp.json path has no parent: {}", path.display());
        return;
    };
    if let Err(e) = std::fs::create_dir_all(parent) {
        tracing::warn!(
            "Failed to create lsp.json parent {}: {}",
            parent.display(),
            e
        );
        return;
    }
    let json = match serde_json::to_string_pretty(state) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("Failed to serialize lsp.json: {}", e);
            return;
        }
    };
    let tmp = path.with_extension("json.tmp");
    if let Err(e) = std::fs::write(&tmp, json.as_bytes()) {
        tracing::warn!("Failed to write {}: {}", tmp.display(), e);
        return;
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        tracing::warn!(
            "Failed to rename {} -> {}: {}",
            tmp.display(),
            path.display(),
            e
        );
    }
}
