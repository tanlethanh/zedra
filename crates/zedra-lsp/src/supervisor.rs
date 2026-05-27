//! Background watcher that enforces the resource guard contract.
//!
//! Polls each running `LspServer` at `guard.poll_interval`, updates RSS, kills
//! servers that exceed caps, and tears down on aggregate-cap overflow by
//! evicting the highest-RSS server first.

use std::collections::HashMap;
use std::sync::Arc;

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};
use tokio::sync::Mutex;
use zedra_rpc::proto::{LspKillReason, LspLanguage};

use crate::guard::GuardConfig;
use crate::server::LspServer;

pub struct Supervisor {
    servers: Mutex<HashMap<LspLanguage, Arc<LspServer>>>,
    guard: GuardConfig,
}

impl Supervisor {
    pub fn new(guard: GuardConfig) -> Arc<Self> {
        Arc::new(Self {
            servers: Mutex::new(HashMap::new()),
            guard,
        })
    }

    /// Spawn the watcher task. Returns immediately. The task lives until the
    /// returned `Arc` and all clones are dropped.
    pub fn spawn_watcher(self: &Arc<Self>) {
        let me = Arc::clone(self);
        let interval = me.guard.poll_interval;
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                me.tick().await;
            }
        });
    }

    /// Look up a running server. `None` if no server is running for this
    /// language (either never spawned or already terminated).
    pub async fn get(&self, language: LspLanguage) -> Option<Arc<LspServer>> {
        self.servers.lock().await.get(&language).cloned()
    }

    pub async fn all(&self) -> Vec<Arc<LspServer>> {
        self.servers.lock().await.values().cloned().collect()
    }

    /// Spawn a server for `language` if not already running. Enforces the
    /// concurrent-cap by refusing to spawn when at capacity.
    pub async fn start(&self, language: LspLanguage) -> anyhow::Result<()> {
        let mut servers = self.servers.lock().await;
        if servers.contains_key(&language) {
            return Ok(());
        }
        if servers.len() as u32 >= self.guard.concurrent_cap {
            anyhow::bail!(
                "concurrent LSP cap reached ({}); disable a language first",
                self.guard.concurrent_cap
            );
        }
        let server = LspServer::new(language);
        server.spawn().await?;
        servers.insert(language, server);
        Ok(())
    }

    /// Shut down the server for `language` if running. No-op otherwise.
    pub async fn stop(&self, language: LspLanguage, reason: LspKillReason) {
        let server = self.servers.lock().await.remove(&language);
        if let Some(server) = server {
            server.shutdown(reason).await;
        }
    }

    /// One watcher iteration: refresh RSS, detect crashes, enforce per-server
    /// and aggregate caps.
    async fn tick(&self) {
        let servers: Vec<Arc<LspServer>> = self.servers.lock().await.values().cloned().collect();
        if servers.is_empty() {
            return;
        }

        let mut crashed: Vec<LspLanguage> = Vec::new();
        for server in &servers {
            if server.poll_crash().await {
                crashed.push(server.language());
            }
        }
        if !crashed.is_empty() {
            let mut guard = self.servers.lock().await;
            for lang in crashed {
                guard.remove(&lang);
            }
        }

        let live: Vec<Arc<LspServer>> = self.servers.lock().await.values().cloned().collect();
        if live.is_empty() {
            return;
        }

        let mut system = System::new();
        let pids: Vec<Pid> = live
            .iter()
            .filter_map(|s| s.pid().map(|p| Pid::from(p as usize)))
            .collect();
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&pids),
            ProcessRefreshKind::new().with_memory(),
        );
        let _ = RefreshKind::new();

        let mut aggregate = 0u64;
        let mut oversized: Vec<Arc<LspServer>> = Vec::new();
        let mut per_server_rss: Vec<(Arc<LspServer>, u64)> = Vec::with_capacity(live.len());

        for server in live {
            let Some(pid) = server.pid() else {
                continue;
            };
            let rss = system
                .process(Pid::from(pid as usize))
                .map(|p| p.memory())
                .unwrap_or(0);
            server.record_rss(rss);
            aggregate = aggregate.saturating_add(rss);
            if rss > self.guard.per_server_rss_cap_bytes {
                oversized.push(Arc::clone(&server));
            }
            per_server_rss.push((server, rss));
        }

        for server in oversized {
            tracing::warn!(
                language = ?server.language(),
                "LSP per-server RSS cap exceeded, terminating",
            );
            self.stop(server.language(), LspKillReason::Oom).await;
        }

        if self.guard.aggregate_rss_cap_bytes > 0 && aggregate > self.guard.aggregate_rss_cap_bytes
        {
            // Evict the highest-RSS live server. Repeat once if still over;
            // tick will catch the rest next interval.
            per_server_rss.sort_by(|a, b| b.1.cmp(&a.1));
            if let Some((worst, _)) = per_server_rss.first() {
                tracing::warn!(
                    language = ?worst.language(),
                    aggregate_bytes = aggregate,
                    cap_bytes = self.guard.aggregate_rss_cap_bytes,
                    "LSP aggregate RSS cap exceeded, evicting worst server",
                );
                self.stop(worst.language(), LspKillReason::AggregateOom)
                    .await;
            }
        }
    }
}
