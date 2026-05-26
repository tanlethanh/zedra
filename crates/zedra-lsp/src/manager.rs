//! `LspManager` — per-workspace LSP enablement state.
//!
//! Current scope: bookkeeping only. Tracks which languages are enabled, where
//! the persisted file lives, and how to produce an `LspStatusResult` snapshot
//! that mirrors what the runtime supervisor will report. Spawning the actual
//! language-server processes is wired in a follow-up commit; until then every
//! enabled language reports `LspServerState::Idle`.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Mutex;
use zedra_rpc::proto::{LspLanguage, LspServerInfo, LspServerState, LspStatusResult};

use crate::guard::{GUARD_DEFAULTS, GuardConfig};
use crate::persistence::{self, PersistedLspState};

pub struct LspManager {
    inner: Mutex<Inner>,
    storage_path: PathBuf,
    guard: GuardConfig,
}

struct Inner {
    enabled: HashSet<LspLanguage>,
}

impl LspManager {
    /// Load persisted enablement from `storage_path` and return a manager. The
    /// supervisor is **not** started here — that arrives with the spawn
    /// commit.
    pub fn load(storage_path: PathBuf) -> Arc<Self> {
        let persisted = persistence::load(&storage_path);
        let enabled: HashSet<LspLanguage> = persisted.enabled_languages.into_iter().collect();
        Arc::new(Self {
            inner: Mutex::new(Inner { enabled }),
            storage_path,
            guard: GUARD_DEFAULTS.with_env_overrides(),
        })
    }

    /// In-memory manager with no persistence. Used by tests and ephemeral
    /// daemons that have no workspace config dir.
    pub fn ephemeral() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner {
                enabled: HashSet::new(),
            }),
            storage_path: PathBuf::new(),
            guard: GUARD_DEFAULTS.with_env_overrides(),
        })
    }

    pub fn guard_config(&self) -> GuardConfig {
        self.guard
    }

    /// Mark `language` as enabled for this workspace and persist the change.
    /// Returns `Ok(())` on success. The supervisor will treat the language as
    /// spawnable on the next subscription.
    pub async fn enable(&self, language: LspLanguage) -> anyhow::Result<()> {
        let mut inner = self.inner.lock().await;
        if inner.enabled.insert(language) {
            self.persist_locked(&inner);
        }
        Ok(())
    }

    /// Mark `language` as disabled. Future commits will tear down a running
    /// server here.
    pub async fn disable(&self, language: LspLanguage) -> anyhow::Result<()> {
        let mut inner = self.inner.lock().await;
        if inner.enabled.remove(&language) {
            self.persist_locked(&inner);
        }
        Ok(())
    }

    pub async fn is_enabled(&self, language: LspLanguage) -> bool {
        self.inner.lock().await.enabled.contains(&language)
    }

    pub async fn any_enabled(&self) -> bool {
        !self.inner.lock().await.enabled.is_empty()
    }

    /// Build the status snapshot returned by `LspStatus` RPC and consumed by
    /// the `zedra status` CLI block. Single source of truth so the two views
    /// can never drift.
    pub async fn status_snapshot(&self) -> LspStatusResult {
        let inner = self.inner.lock().await;
        let mut servers: Vec<LspServerInfo> = inner
            .enabled
            .iter()
            .copied()
            .map(|language| LspServerInfo {
                language,
                state: LspServerState::Idle,
                pid: None,
                rss_bytes: 0,
                uptime_secs: 0,
                diagnostic_errors: 0,
                diagnostic_warnings: 0,
                last_request_ms: None,
                last_kill_reason: None,
                peak_rss_bytes: 0,
            })
            .collect();
        // Stable ordering for CLI / UI render.
        servers.sort_by_key(|s| language_sort_key(s.language));

        LspStatusResult {
            enabled: !inner.enabled.is_empty(),
            servers,
            aggregate_rss_bytes: 0,
            aggregate_rss_cap_bytes: self.guard.aggregate_rss_cap_bytes,
            concurrent_cap: self.guard.concurrent_cap,
        }
    }

    fn persist_locked(&self, inner: &Inner) {
        if self.storage_path.as_os_str().is_empty() {
            return;
        }
        let state = PersistedLspState {
            version: 1,
            enabled_languages: {
                let mut v: Vec<LspLanguage> = inner.enabled.iter().copied().collect();
                v.sort_by_key(|l| language_sort_key(*l));
                v
            },
        };
        persistence::save(&self.storage_path, &state);
    }
}

fn language_sort_key(language: LspLanguage) -> u8 {
    match language {
        LspLanguage::Rust => 0,
        LspLanguage::Go => 1,
        LspLanguage::TypeScript => 2,
        LspLanguage::JavaScript => 3,
        LspLanguage::Python => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn enable_disable_roundtrip_persists() {
        let dir = tempdir();
        let path = dir.join("lsp.json");
        let m = LspManager::load(path.clone());
        m.enable(LspLanguage::Rust).await.unwrap();
        m.enable(LspLanguage::Go).await.unwrap();
        drop(m);

        let m = LspManager::load(path);
        assert!(m.is_enabled(LspLanguage::Rust).await);
        assert!(m.is_enabled(LspLanguage::Go).await);
        assert!(!m.is_enabled(LspLanguage::Python).await);

        m.disable(LspLanguage::Rust).await.unwrap();
        assert!(!m.is_enabled(LspLanguage::Rust).await);
    }

    #[tokio::test]
    async fn ephemeral_manager_does_not_persist() {
        let m = LspManager::ephemeral();
        m.enable(LspLanguage::Rust).await.unwrap();
        assert!(m.is_enabled(LspLanguage::Rust).await);
    }

    #[tokio::test]
    async fn status_snapshot_lists_enabled_languages_as_idle() {
        let m = LspManager::ephemeral();
        m.enable(LspLanguage::Rust).await.unwrap();
        m.enable(LspLanguage::TypeScript).await.unwrap();
        let snap = m.status_snapshot().await;
        assert!(snap.enabled);
        assert_eq!(snap.servers.len(), 2);
        assert!(
            snap.servers
                .iter()
                .all(|s| matches!(s.state, LspServerState::Idle))
        );
    }

    fn tempdir() -> PathBuf {
        let mut p = std::env::temp_dir();
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        p.push(format!("zedra-lsp-test-{nonce}"));
        std::fs::create_dir_all(&p).unwrap();
        p
    }
}
