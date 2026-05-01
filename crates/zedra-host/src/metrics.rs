use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::identity;

const METRICS_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonStartMode {
    Foreground,
    Detached,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkspaceMetrics {
    pub version: u32,
    pub daemon_starts: u64,
    pub foreground_starts: u64,
    pub detached_starts: u64,
    pub successful_connections: u64,
    pub new_pairings: u64,
    pub qr_codes_created: u64,
    pub sessions_created: u64,
    pub terminals_created: u64,
    pub max_sessions_seen: u64,
    pub max_terminals_seen: u64,
    pub total_active_secs: u64,
    pub active_connection_count: u64,
    pub active_started_at_unix_secs: Option<u64>,
    pub first_started_at_unix_secs: Option<u64>,
    pub last_started_at_unix_secs: Option<u64>,
    pub last_connected_at_unix_secs: Option<u64>,
    pub last_pairing_at_unix_secs: Option<u64>,
    pub last_qr_created_at_unix_secs: Option<u64>,
    pub last_terminal_created_at_unix_secs: Option<u64>,
    pub last_seen_at_unix_secs: Option<u64>,
}

impl Default for WorkspaceMetrics {
    fn default() -> Self {
        Self {
            version: METRICS_VERSION,
            daemon_starts: 0,
            foreground_starts: 0,
            detached_starts: 0,
            successful_connections: 0,
            new_pairings: 0,
            qr_codes_created: 0,
            sessions_created: 0,
            terminals_created: 0,
            max_sessions_seen: 0,
            max_terminals_seen: 0,
            total_active_secs: 0,
            active_connection_count: 0,
            active_started_at_unix_secs: None,
            first_started_at_unix_secs: None,
            last_started_at_unix_secs: None,
            last_connected_at_unix_secs: None,
            last_pairing_at_unix_secs: None,
            last_qr_created_at_unix_secs: None,
            last_terminal_created_at_unix_secs: None,
            last_seen_at_unix_secs: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetricsSnapshot {
    pub metrics: WorkspaceMetrics,
    pub generated_at_unix_secs: u64,
    pub active_secs: u64,
}

impl WorkspaceMetrics {
    fn active_secs_at(&self, now: u64) -> u64 {
        let live_secs = if self.active_connection_count > 0 {
            self.active_started_at_unix_secs
                .map(|started| now.saturating_sub(started))
                .unwrap_or_default()
        } else {
            0
        };
        self.total_active_secs.saturating_add(live_secs)
    }

    fn close_stale_active_window(&mut self, now: u64) {
        if self.active_connection_count == 0 {
            self.active_started_at_unix_secs = None;
            return;
        }

        let end = self.last_seen_at_unix_secs.unwrap_or(now).min(now);
        if let Some(started) = self.active_started_at_unix_secs.take() {
            self.total_active_secs = self
                .total_active_secs
                .saturating_add(end.saturating_sub(started));
        }
        self.active_connection_count = 0;
    }

    fn snapshot_at(&self, now: u64) -> MetricsSnapshot {
        MetricsSnapshot {
            metrics: self.clone(),
            generated_at_unix_secs: now,
            active_secs: self.active_secs_at(now),
        }
    }
}

pub fn metrics_path(workdir: &Path) -> Result<PathBuf> {
    Ok(identity::workspace_config_dir(workdir)?.join("metrics.json"))
}

pub fn load(workdir: &Path) -> Result<WorkspaceMetrics> {
    read_metrics_file(&metrics_path(workdir)?)
}

pub fn snapshot(workdir: &Path) -> Result<MetricsSnapshot> {
    let now = unix_now_secs();
    let metrics = load(workdir)?;
    Ok(metrics.snapshot_at(now))
}

pub fn record_daemon_start(workdir: &Path, mode: DaemonStartMode) -> Result<()> {
    update(workdir, |metrics, now| {
        metrics.close_stale_active_window(now);
        metrics.daemon_starts = metrics.daemon_starts.saturating_add(1);
        match mode {
            DaemonStartMode::Foreground => {
                metrics.foreground_starts = metrics.foreground_starts.saturating_add(1);
            }
            DaemonStartMode::Detached => {
                metrics.detached_starts = metrics.detached_starts.saturating_add(1);
            }
        }
        if metrics.first_started_at_unix_secs.is_none() {
            metrics.first_started_at_unix_secs = Some(now);
        }
        metrics.last_started_at_unix_secs = Some(now);
    })
}

pub fn record_daemon_heartbeat(
    workdir: &Path,
    session_count: usize,
    terminal_count: usize,
) -> Result<()> {
    update(workdir, |metrics, _now| {
        metrics.max_sessions_seen = metrics.max_sessions_seen.max(session_count as u64);
        metrics.max_terminals_seen = metrics.max_terminals_seen.max(terminal_count as u64);
    })
}

pub fn record_session_created(workdir: &Path, session_count: usize) -> Result<()> {
    update(workdir, |metrics, _now| {
        metrics.sessions_created = metrics.sessions_created.saturating_add(1);
        metrics.max_sessions_seen = metrics.max_sessions_seen.max(session_count as u64);
    })
}

pub fn record_connection_opened(workdir: &Path) -> Result<()> {
    update(workdir, |metrics, now| {
        if metrics.active_connection_count == 0 {
            metrics.active_started_at_unix_secs = Some(now);
        }
        metrics.active_connection_count = metrics.active_connection_count.saturating_add(1);
        metrics.successful_connections = metrics.successful_connections.saturating_add(1);
        metrics.last_connected_at_unix_secs = Some(now);
    })
}

pub fn record_connection_closed(workdir: &Path) -> Result<()> {
    update(workdir, |metrics, now| {
        if metrics.active_connection_count == 0 {
            metrics.active_started_at_unix_secs = None;
            return;
        }

        metrics.active_connection_count = metrics.active_connection_count.saturating_sub(1);
        if metrics.active_connection_count == 0 {
            if let Some(started) = metrics.active_started_at_unix_secs.take() {
                metrics.total_active_secs = metrics
                    .total_active_secs
                    .saturating_add(now.saturating_sub(started));
            }
        }
    })
}

pub fn record_pairing_completed(workdir: &Path) -> Result<()> {
    update(workdir, |metrics, now| {
        metrics.new_pairings = metrics.new_pairings.saturating_add(1);
        metrics.last_pairing_at_unix_secs = Some(now);
    })
}

pub fn record_qr_created(workdir: &Path) -> Result<()> {
    update(workdir, |metrics, now| {
        metrics.qr_codes_created = metrics.qr_codes_created.saturating_add(1);
        metrics.last_qr_created_at_unix_secs = Some(now);
    })
}

pub fn record_terminal_created(workdir: &Path, terminal_count: usize) -> Result<()> {
    update(workdir, |metrics, now| {
        metrics.terminals_created = metrics.terminals_created.saturating_add(1);
        metrics.max_terminals_seen = metrics.max_terminals_seen.max(terminal_count as u64);
        metrics.last_terminal_created_at_unix_secs = Some(now);
    })
}

fn update(workdir: &Path, apply: impl FnOnce(&mut WorkspaceMetrics, u64)) -> Result<()> {
    let _guard = metrics_lock().lock().unwrap_or_else(|err| err.into_inner());
    let path = metrics_path(workdir)?;
    let now = unix_now_secs();
    let mut metrics = read_metrics_file(&path)?;
    metrics.version = METRICS_VERSION;
    apply(&mut metrics, now);
    metrics.last_seen_at_unix_secs = Some(now);
    write_metrics_file(&path, &metrics)
}

fn read_metrics_file(path: &Path) -> Result<WorkspaceMetrics> {
    let data = match std::fs::read_to_string(path) {
        Ok(data) => data,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(WorkspaceMetrics::default());
        }
        Err(err) => {
            return Err(err)
                .with_context(|| format!("failed to read metrics at {}", path.display()));
        }
    };

    serde_json::from_str(&data)
        .with_context(|| format!("failed to parse metrics at {}", path.display()))
}

fn write_metrics_file(path: &Path, metrics: &WorkspaceMetrics) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("metrics path has no parent"))?;
    std::fs::create_dir_all(parent)?;

    let json = serde_json::to_string_pretty(metrics)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("metrics.json");
    let tmp_path = parent.join(format!(
        ".{}.{}.{}.tmp",
        file_name,
        std::process::id(),
        rand::random::<u64>()
    ));

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let mut file = options
        .open(&tmp_path)
        .with_context(|| format!("failed to create metrics temp file {}", tmp_path.display()))?;
    file.write_all(json.as_bytes())?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    drop(file);

    std::fs::rename(&tmp_path, path).with_context(|| {
        format!(
            "failed to replace metrics file {} with {}",
            path.display(),
            tmp_path.display()
        )
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }

    Ok(())
}

fn metrics_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_seconds_accumulate_after_last_connection_closes() {
        let mut metrics = WorkspaceMetrics::default();

        metrics.active_connection_count = 1;
        metrics.active_started_at_unix_secs = Some(100);
        assert_eq!(metrics.active_secs_at(150), 50);

        metrics.close_stale_active_window(160);
        assert_eq!(metrics.active_connection_count, 0);
        assert_eq!(metrics.total_active_secs, 60);
        assert_eq!(metrics.active_secs_at(200), 60);
    }

    #[test]
    fn stale_active_window_uses_last_seen_instead_of_restart_time() {
        let mut metrics = WorkspaceMetrics {
            active_connection_count: 1,
            active_started_at_unix_secs: Some(100),
            last_seen_at_unix_secs: Some(140),
            ..WorkspaceMetrics::default()
        };

        metrics.close_stale_active_window(200);

        assert_eq!(metrics.total_active_secs, 40);
        assert_eq!(metrics.active_connection_count, 0);
        assert_eq!(metrics.active_started_at_unix_secs, None);
    }

    #[test]
    fn missing_metrics_file_loads_empty_metrics() {
        let dir = tempfile::tempdir().unwrap();
        let metrics = read_metrics_file(&dir.path().join("missing.json")).unwrap();

        assert_eq!(metrics.version, METRICS_VERSION);
        assert_eq!(metrics.daemon_starts, 0);
        assert_eq!(metrics.successful_connections, 0);
    }

    #[cfg(unix)]
    #[test]
    fn metrics_file_is_private_on_unix() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("metrics.json");
        let metrics = WorkspaceMetrics {
            daemon_starts: 1,
            ..WorkspaceMetrics::default()
        };

        write_metrics_file(&path, &metrics).unwrap();

        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(read_metrics_file(&path).unwrap().daemon_starts, 1);
    }
}
