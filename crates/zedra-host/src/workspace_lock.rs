// workspace_lock.rs — PID lock file to prevent duplicate zedra-host instances
// on the same workspace directory.
//
// Lock file: <platform config>/zedra/workspaces/<hash>/daemon.lock
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
//   - On Unix, sends SIGTERM; polls up to the grace period; escalates to SIGKILL.
//   - On Windows, terminates the process if the local shutdown API did not work.
//   - Removes the lock file.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::identity;

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

/// Scan all workspace config directories under Zedra's platform config root
/// and return every instance that has a lock file, along with whether its
/// process is still alive and the path to its config directory.
pub fn scan_all_instances() -> Vec<(PathBuf, LockInfo, bool)> {
    let workspaces_dir = match identity::zedra_config_dir() {
        Ok(dir) => dir.join("workspaces"),
        Err(_) => return Vec::new(),
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
/// 2. Sends the platform stop request and waits up to `grace_secs`.
/// 3. Escalates if the process is still running after the grace period.
/// 4. Removes the lock file.
///
/// Returns an error if there is no lock file or the process cannot be stopped.
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

    request_process_exit(&info)?;

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

    escalate_process_exit(info.pid, grace_secs)?;
    std::thread::sleep(std::time::Duration::from_millis(500));
    let _ = std::fs::remove_file(&lock_path);

    if is_process_alive(info.pid) {
        anyhow::bail!("Process {} could not be killed.", info.pid);
    }

    tracing::info!("Process {} killed.", info.pid);
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Path: `<platform config>/zedra/workspaces/<hash>/daemon.lock`
fn lock_file_path(workdir: &Path) -> Result<PathBuf> {
    let path = identity::workspace_config_dir(workdir)?.join("daemon.lock");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(path)
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

#[cfg(all(not(unix), not(windows)))]
pub fn is_process_alive(pid: u32) -> bool {
    let _ = pid;
    true
}

/// Send a graceful process exit request.
#[cfg(unix)]
fn request_process_exit(info: &LockInfo) -> Result<()> {
    tracing::info!(
        "Sending SIGTERM to PID {} (workdir: {}, started: {})",
        info.pid,
        info.workdir,
        info.running_for()
    );
    send_signal(info.pid, libc::SIGTERM)
}

#[cfg(windows)]
fn request_process_exit(info: &LockInfo) -> Result<()> {
    // `zedra stop` tries the authenticated local shutdown API before this
    // fallback. Windows has no Unix-style SIGTERM, so the fallback is terminate.
    tracing::warn!(
        "Terminating PID {} (workdir: {}, started: {})",
        info.pid,
        info.workdir,
        info.running_for()
    );
    terminate_process(info.pid)
}

#[cfg(all(not(unix), not(windows)))]
fn request_process_exit(_info: &LockInfo) -> Result<()> {
    anyhow::bail!("'zedra stop' is not supported on this platform.");
}

/// Escalate when graceful stop did not exit within the grace period.
#[cfg(unix)]
fn escalate_process_exit(pid: u32, grace_secs: u64) -> Result<()> {
    tracing::warn!(
        "Process {} did not exit after {}s; sending SIGKILL.",
        pid,
        grace_secs
    );
    send_signal(pid, libc::SIGKILL)
}

#[cfg(windows)]
fn escalate_process_exit(pid: u32, grace_secs: u64) -> Result<()> {
    tracing::warn!(
        "Process {} did not exit after {}s; terminating again.",
        pid,
        grace_secs
    );
    terminate_process(pid)
}

#[cfg(all(not(unix), not(windows)))]
fn escalate_process_exit(_pid: u32, _grace_secs: u64) -> Result<()> {
    anyhow::bail!("'zedra stop' is not supported on this platform.");
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

#[cfg(windows)]
pub fn is_process_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }

        let mut exit_code = 0;
        let ok = GetExitCodeProcess(handle, &mut exit_code) != 0;
        let _ = CloseHandle(handle);
        ok && exit_code == STILL_ACTIVE
    }
}

#[cfg(windows)]
fn terminate_process(pid: u32) -> Result<()> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, TerminateProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
    };

    unsafe {
        let handle = OpenProcess(
            PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION,
            0,
            pid,
        );
        if handle.is_null() {
            return Err(std::io::Error::last_os_error()).context(format!("OpenProcess({pid})"));
        }
        let ok = TerminateProcess(handle, 0) != 0;
        let _ = CloseHandle(handle);
        if ok {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error()).context(format!("TerminateProcess({pid})"))
        }
    }
}
