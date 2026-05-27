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
use zedra_rpc::proto::{
    LspKillReason, LspLanguage, LspServerInfo, LspServerState, LspStatusResult,
};

use crate::guard::{GUARD_DEFAULTS, GuardConfig};
use crate::persistence::{self, PersistedLspState};
use crate::supervisor::Supervisor;

pub struct LspManager {
    inner: Mutex<Inner>,
    storage_path: PathBuf,
    guard: GuardConfig,
    supervisor: Arc<Supervisor>,
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
        let guard = GUARD_DEFAULTS.with_env_overrides();
        let supervisor = Supervisor::new(guard);
        supervisor.spawn_watcher();
        Arc::new(Self {
            inner: Mutex::new(Inner { enabled }),
            storage_path,
            guard,
            supervisor,
        })
    }

    /// In-memory manager with no persistence and no watcher task. Used by
    /// tests and ephemeral daemons that have no workspace config dir.
    pub fn ephemeral() -> Arc<Self> {
        let guard = GUARD_DEFAULTS.with_env_overrides();
        Arc::new(Self {
            inner: Mutex::new(Inner {
                enabled: HashSet::new(),
            }),
            storage_path: PathBuf::new(),
            guard,
            supervisor: Supervisor::new(guard),
        })
    }

    pub fn guard_config(&self) -> GuardConfig {
        self.guard
    }

    /// Mark `language` as enabled for this workspace, persist the change, and
    /// spawn the language server. A spawn failure (binary missing, cap
    /// reached) is returned to the caller but does not roll back the persisted
    /// enablement — the server can be retried with the same flag set.
    pub async fn enable(&self, language: LspLanguage) -> anyhow::Result<()> {
        {
            let mut inner = self.inner.lock().await;
            if inner.enabled.insert(language) {
                self.persist_locked(&inner);
            }
        }
        self.supervisor.start(language).await
    }

    /// Mark `language` as disabled and shut down a running server.
    pub async fn disable(&self, language: LspLanguage) -> anyhow::Result<()> {
        {
            let mut inner = self.inner.lock().await;
            if inner.enabled.remove(&language) {
                self.persist_locked(&inner);
            }
        }
        self.supervisor.stop(language, LspKillReason::Manual).await;
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
        let enabled_langs: Vec<LspLanguage> = {
            let inner = self.inner.lock().await;
            inner.enabled.iter().copied().collect()
        };
        let mut servers: Vec<LspServerInfo> = Vec::with_capacity(enabled_langs.len());
        let mut aggregate: u64 = 0;
        for language in enabled_langs {
            let live = self.supervisor.get(language).await;
            let info = if let Some(server) = live {
                let rss = server.peak_rss_bytes();
                aggregate = aggregate.saturating_add(rss);
                LspServerInfo {
                    language,
                    state: server.state().await,
                    pid: server.pid(),
                    rss_bytes: rss,
                    uptime_secs: server.uptime_secs().await,
                    diagnostic_errors: 0,
                    diagnostic_warnings: 0,
                    last_request_ms: None,
                    last_kill_reason: server.last_kill_reason().await,
                    peak_rss_bytes: rss,
                }
            } else {
                LspServerInfo {
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
                }
            };
            servers.push(info);
        }
        servers.sort_by_key(|s| language_sort_key(s.language));

        LspStatusResult {
            enabled: !servers.is_empty(),
            servers,
            aggregate_rss_bytes: aggregate,
            aggregate_rss_cap_bytes: self.guard.aggregate_rss_cap_bytes,
            concurrent_cap: self.guard.concurrent_cap,
        }
    }

    /// Re-spawn servers for every persisted-enabled language. Called once at
    /// daemon start so the watcher has servers to supervise. Failures are
    /// logged, not propagated; a missing binary should not prevent the
    /// daemon from starting other LSPs or the rest of zedra-host.
    pub async fn restore_enabled(&self) {
        let langs: Vec<LspLanguage> = {
            let inner = self.inner.lock().await;
            inner.enabled.iter().copied().collect()
        };
        for language in langs {
            if let Err(e) = self.supervisor.start(language).await {
                tracing::warn!(
                    language = ?language,
                    "Failed to start LSP server at daemon startup: {}",
                    e,
                );
            }
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
    use crate::persistence;

    // `enable()` tries to spawn the language server, which is unavailable in
    // CI. These tests cover the persistence + bookkeeping seams without
    // exercising the supervisor.

    #[tokio::test]
    async fn persistence_roundtrip_independent_of_supervisor() {
        let dir = tempdir();
        let path = dir.join("lsp.json");
        persistence::save(
            &path,
            &PersistedLspState {
                version: 1,
                enabled_languages: vec![LspLanguage::Rust, LspLanguage::Go],
            },
        );
        let m = LspManager::load(path);
        assert!(m.is_enabled(LspLanguage::Rust).await);
        assert!(m.is_enabled(LspLanguage::Go).await);
        assert!(!m.is_enabled(LspLanguage::Python).await);
    }

    #[tokio::test]
    async fn status_snapshot_reports_idle_for_enabled_but_unspawned() {
        let m = LspManager::ephemeral();
        // Seed enablement without going through enable() so we skip the spawn.
        {
            let mut inner = m.inner.lock().await;
            inner.enabled.insert(LspLanguage::Rust);
            inner.enabled.insert(LspLanguage::TypeScript);
        }
        let snap = m.status_snapshot().await;
        assert!(snap.enabled);
        assert_eq!(snap.servers.len(), 2);
        assert!(
            snap.servers
                .iter()
                .all(|s| matches!(s.state, LspServerState::Idle))
        );
        assert_eq!(snap.concurrent_cap, m.guard.concurrent_cap);
    }

    #[tokio::test]
    async fn empty_manager_reports_disabled() {
        let m = LspManager::ephemeral();
        let snap = m.status_snapshot().await;
        assert!(!snap.enabled);
        assert!(snap.servers.is_empty());
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
