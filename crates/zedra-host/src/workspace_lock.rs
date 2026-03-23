// workspace_lock.rs — PID lock file to prevent duplicate zedra-host instances
// on the same workspace directory.
//
// Lock file: ~/.config/zedra/workspaces/<hash>/daemon.lock
// Contains:  JSON with PID, workdir path, hostname, and start timestamp.
//
// Acquire semantics:
//   1. Try to create the lock file exclusively (O_CREAT | O_EXCL).
//   2. If already exists, read the metadata and check whether that process is alive.
//      - Alive  → bail with a descriptive error showing who owns the lock.
//      - Dead   → stale lock; overwrite and proceed.
//   3. Write our metadata, return a guard that deletes the file on drop.
//
// Stop command (`kill_and_unlock`):
//   - Reads the lock file for the given workdir.
//   - Sends SIGTERM; polls up to 5 s; escalates to SIGKILL if needed.
//   - Removes the lock file.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Lock metadata
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub struct LockInfo {
    /// PID of the owning zedra-host process.
    pub pid: u32,
    /// Canonical working directory being served.
    pub workdir: String,
    /// Hostname of the machine running the daemon.
    pub hostname: String,
    /// Unix timestamp (seconds) when the daemon started.
    pub started_secs: u64,
}

impl LockInfo {
    fn new(workdir: &Path) -> Self {
        Self {
            pid: std::process::id(),
            workdir: workdir.display().to_string(),
            hostname: hostname::get()
                .ok()
                .and_then(|h| h.into_string().ok())
                .unwrap_or_else(|| "unknown".to_string()),
            started_secs: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        }
    }

    /// Human-readable "X minutes ago" / "X seconds ago" string.
    pub fn running_for(&self) -> String {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(self.started_secs);
        let secs = now.saturating_sub(self.started_secs);
        if secs < 60 {
            format!("{} second{} ago", secs, if secs == 1 { "" } else { "s" })
        } else if secs < 3600 {
            let m = secs / 60;
            format!("{} minute{} ago", m, if m == 1 { "" } else { "s" })
        } else {
            let h = secs / 3600;
            format!("{} hour{} ago", h, if h == 1 { "" } else { "s" })
        }
    }
}

// ---------------------------------------------------------------------------
// Lock guard
// ---------------------------------------------------------------------------

/// Held for the lifetime of the daemon process. Deletes the lock file on drop.
pub struct WorkspaceLock {
    path: PathBuf,
}

impl Drop for WorkspaceLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Acquire a workspace-scoped lock for `workdir`.
///
/// Returns a `WorkspaceLock` guard that must be kept alive for the duration of
/// the daemon. Fails with a human-readable error if another live instance
/// already holds the lock for this workspace.
pub fn acquire(workdir: &Path) -> Result<WorkspaceLock> {
    let lock_path = lock_file_path(workdir)?;
    let info = LockInfo::new(workdir);

    // Attempt atomic creation first.
    if try_write_exclusive(&lock_path, &info).is_ok() {
        return Ok(WorkspaceLock { path: lock_path });
    }

    // File already exists — check if owner is still alive.
    if let Some(existing) = read_lock_file(&lock_path) {
        if is_process_alive(existing.pid) {
            anyhow::bail!(
                "zedra-host is already running for this workspace.\n\
                 \n\
                 \x20 PID:      {}\n\
                 \x20 Workdir:  {}\n\
                 \x20 Host:     {}\n\
                 \x20 Started:  {}\n\
                 \x20 Lock:     {}\n\
                 \n\
                 Run 'zedra stop --workdir {}' to stop it.",
                existing.pid,
                existing.workdir,
                existing.hostname,
                existing.running_for(),
                lock_path.display(),
                existing.workdir,
            );
        }
        // Stale lock — previous instance died without cleanup.
        tracing::warn!(
            "Removing stale lock file (PID {} is no longer running, was serving {})",
            existing.pid,
            existing.workdir,
        );
    }

    // Overwrite stale / unreadable lock with our metadata.
    overwrite_lock(&lock_path, &info)
        .with_context(|| format!("Failed to write lock file {}", lock_path.display()))?;

    Ok(WorkspaceLock { path: lock_path })
}

/// Scan all workspace config directories under `~/.config/zedra/workspaces/`
/// and return every instance that has a lock file, along with whether its
/// process is still alive and the path to its config directory.
pub fn scan_all_instances() -> Vec<(PathBuf, LockInfo, bool)> {
    let workspaces_dir = match (|| -> Option<PathBuf> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .or_else(|| directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf()))?;
        Some(home.join(".config").join("zedra").join("workspaces"))
    })() {
        Some(d) => d,
        None => return Vec::new(),
    };

    let entries = match std::fs::read_dir(&workspaces_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut result = Vec::new();
    for entry in entries.flatten() {
        let config_dir = entry.path();
        let lock_path = config_dir.join("daemon.lock");
        if let Some(info) = read_lock_file(&lock_path) {
            let alive = is_process_alive(info.pid);
            result.push((config_dir, info, alive));
        }
    }

    // Sort by workdir for stable output
    result.sort_by(|a, b| a.1.workdir.cmp(&b.1.workdir));
    result
}

/// Read the lock file metadata for a workspace without acquiring the lock.
/// Returns `None` if no lock file exists or it cannot be parsed.
pub fn read_lock_info(workdir: &Path) -> Result<Option<LockInfo>> {
    let lock_path = lock_file_path(workdir)?;
    if !lock_path.exists() {
        return Ok(None);
    }
    Ok(read_lock_file(&lock_path))
}

/// Stop the daemon owning `workdir`'s lock.
///
/// 1. Reads the lock file to get the PID and confirm it is alive.
/// 2. Sends SIGTERM and waits up to `grace_secs` for a clean exit.
/// 3. Sends SIGKILL if the process is still running after the grace period.
/// 4. Removes the lock file.
///
/// Returns an error if there is no lock file or the process cannot be stopped.
#[cfg(unix)]
pub fn kill_and_unlock(workdir: &Path, grace_secs: u64) -> Result<()> {
    let lock_path = lock_file_path(workdir)?;

    let info = read_lock_file(&lock_path).ok_or_else(|| {
        anyhow::anyhow!(
            "No running zedra-host found for this workspace.\nLock file: {}",
            lock_path.display()
        )
    })?;

    if !is_process_alive(info.pid) {
        tracing::info!(
            "Process {} is already gone; removing stale lock file.",
            info.pid
        );
        let _ = std::fs::remove_file(&lock_path);
        return Ok(());
    }

    tracing::info!(
        "Sending SIGTERM to PID {} (workdir: {}, started: {})",
        info.pid,
        info.workdir,
        info.running_for()
    );
    send_signal(info.pid, libc::SIGTERM)?;

    // Poll for clean exit.
    let poll_interval = std::time::Duration::from_millis(200);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(grace_secs);
    while std::time::Instant::now() < deadline {
        std::thread::sleep(poll_interval);
        if !is_process_alive(info.pid) {
            tracing::info!("Process {} exited cleanly.", info.pid);
            let _ = std::fs::remove_file(&lock_path);
            return Ok(());
        }
    }

    // Grace period expired — escalate to SIGKILL.
    tracing::warn!(
        "Process {} did not exit after {}s; sending SIGKILL.",
        info.pid,
        grace_secs
    );
    send_signal(info.pid, libc::SIGKILL)?;
    std::thread::sleep(std::time::Duration::from_millis(500));
    let _ = std::fs::remove_file(&lock_path);

    if is_process_alive(info.pid) {
        anyhow::bail!("Process {} could not be killed.", info.pid);
    }

    tracing::info!("Process {} killed.", info.pid);
    Ok(())
}

#[cfg(not(unix))]
pub fn kill_and_unlock(_workdir: &Path, _grace_secs: u64) -> Result<()> {
    anyhow::bail!("'zedra stop' is not supported on this platform.");
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Path: `~/.config/zedra/workspaces/<hash>/daemon.lock`
fn lock_file_path(workdir: &Path) -> Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf()))
        .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
    let config = home.join(".config").join("zedra");
    let hash = path_hash(workdir);
    let path = config.join("workspaces").join(&hash).join("daemon.lock");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(path)
}

/// Same 16-hex-char hash used by `identity.rs` for the workspace directory.
fn path_hash(workdir: &Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    workdir.to_string_lossy().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Try to create the lock file atomically; fail if it already exists.
fn try_write_exclusive(path: &Path, info: &LockInfo) -> std::io::Result<()> {
    let f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true) // O_CREAT | O_EXCL — fails if exists
        .open(path)?;
    serde_json::to_writer_pretty(&f, info)?;
    Ok(())
}

/// Overwrite an existing lock file with new metadata.
fn overwrite_lock(path: &Path, info: &LockInfo) -> std::io::Result<()> {
    let f = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .create(true)
        .open(path)?;
    serde_json::to_writer_pretty(&f, info)?;
    Ok(())
}

/// Parse the lock file; returns `None` if missing or malformed.
fn read_lock_file(path: &Path) -> Option<LockInfo> {
    let contents = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&contents).ok()
}

/// Returns `true` if the process with `pid` is currently running.
#[cfg(unix)]
pub fn is_process_alive(pid: u32) -> bool {
    let ret = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if ret == 0 {
        return true;
    }
    let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
    errno == libc::EPERM
}

#[cfg(not(unix))]
pub fn is_process_alive(pid: u32) -> bool {
    let _ = pid;
    true
}

/// Send a signal to a process.
#[cfg(unix)]
fn send_signal(pid: u32, signal: libc::c_int) -> Result<()> {
    let ret = unsafe { libc::kill(pid as libc::pid_t, signal) };
    if ret == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error()).context(format!("kill({}, {})", pid, signal))
    }
}
