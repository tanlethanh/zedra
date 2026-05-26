//! Resource guard configuration.
//!
//! Defines the caps the runtime supervisor will enforce. Numbers are the
//! defaults; operators override them via env vars or `~/.config/zedra/lsp.toml`
//! (config file parsing arrives with the supervisor commit). Holding the
//! policy here, separate from the supervisor loop, keeps the contract visible
//! and unit-testable without spinning up child processes.
//!
//! These defaults are conservative on purpose: LSP servers (especially
//! `rust-analyzer` on large projects) can balloon memory quickly. The supervisor
//! will surface every termination via `LspServerStateChange { reason }` and a
//! telemetry event so kills are observable, not silent.

use std::time::Duration;

#[derive(Debug, Clone, Copy)]
pub struct GuardConfig {
    /// Per-server RSS cap. Exceeding this triggers SIGTERM, then SIGKILL after
    /// the grace period.
    pub per_server_rss_cap_bytes: u64,
    /// Sum of RSS across all running servers. When exceeded, the supervisor
    /// kills the highest-RSS server first.
    pub aggregate_rss_cap_bytes: u64,
    /// SIGTERM → SIGKILL grace window.
    pub kill_grace: Duration,
    /// Poll interval for the RSS / CPU watcher.
    pub poll_interval: Duration,
    /// Sustained-CPU window. If the server averages above
    /// `cpu_kill_percent` for this long, the supervisor escalates.
    pub cpu_window: Duration,
    pub cpu_kill_percent: u32,
    /// Idle shutdown window. With no active subscription, a server is shut
    /// down after this long.
    pub idle_shutdown: Duration,
    /// Minimum interval between spawn attempts for the same language. Stops
    /// crash-loop thrash.
    pub spawn_rate_limit: Duration,
    /// Hard cap on concurrent running servers across all languages and
    /// workspaces. LRU-evict on overflow.
    pub concurrent_cap: u32,
}

impl GuardConfig {
    /// Effective RSS cap, taking `ZEDRA_LSP_MEM_CAP_MB` into account.
    pub fn with_env_overrides(mut self) -> Self {
        if let Ok(raw) = std::env::var("ZEDRA_LSP_MEM_CAP_MB") {
            if let Ok(mb) = raw.trim().parse::<u64>() {
                self.per_server_rss_cap_bytes = mb.saturating_mul(1024 * 1024);
            }
        }
        if let Ok(raw) = std::env::var("ZEDRA_LSP_AGG_MEM_CAP_MB") {
            if let Ok(mb) = raw.trim().parse::<u64>() {
                self.aggregate_rss_cap_bytes = mb.saturating_mul(1024 * 1024);
            }
        }
        if let Ok(raw) = std::env::var("ZEDRA_LSP_CONCURRENT_CAP") {
            if let Ok(n) = raw.trim().parse::<u32>() {
                self.concurrent_cap = n;
            }
        }
        self
    }
}

pub const GUARD_DEFAULTS: GuardConfig = GuardConfig {
    per_server_rss_cap_bytes: 1536 * 1024 * 1024,
    aggregate_rss_cap_bytes: 4096 * 1024 * 1024,
    kill_grace: Duration::from_secs(3),
    poll_interval: Duration::from_secs(5),
    cpu_window: Duration::from_secs(60),
    cpu_kill_percent: 90,
    idle_shutdown: Duration::from_secs(300),
    spawn_rate_limit: Duration::from_secs(30),
    concurrent_cap: 4,
};
