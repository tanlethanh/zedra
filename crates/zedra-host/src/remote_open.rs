//! Remote "Open project": let a paired client browse the host user's home
//! directory and start a detached daemon for another workdir.
//!
//! Every other filesystem RPC is jailed to this daemon's workdir. These two are
//! deliberately outside it, so they carry their own jail: the host user's home.

use anyhow::{ensure, Result};
use std::path::{Path, PathBuf};

use zedra_rpc::proto::{
    HostDirEntry, HostDirListResult, HostWorkspaceOpenResult, HOST_DIR_LIST_MAX_ENTRIES,
};

use crate::daemon_launch::{
    request_pairing_qr, start_detached, wait_for_detached_pairing_qr, DetachedStartOptions,
};
use crate::{session_registry, start_config, workspace_lock};

fn home_dir() -> Result<PathBuf> {
    let home = crate::rpc_daemon::current_home_dir()
        .ok_or_else(|| anyhow::anyhow!("host home directory is unknown"))?;
    PathBuf::from(home)
        .canonicalize()
        .map_err(|e| anyhow::anyhow!("host home directory is unreadable: {e}"))
}

/// Canonicalize `user_path` (empty means home) and keep it inside the home jail.
fn resolve_in_home(home: &Path, user_path: &str) -> Result<PathBuf> {
    if user_path.is_empty() {
        return Ok(home.to_path_buf());
    }
    let resolved = PathBuf::from(user_path)
        .canonicalize()
        .map_err(|e| anyhow::anyhow!("{user_path}: {e}"))?;
    ensure!(
        resolved.starts_with(home),
        "path {} is outside the home directory",
        resolved.display()
    );
    Ok(resolved)
}

fn display_path(home: &Path, path: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

pub fn dir_list_error(message: String) -> HostDirListResult {
    HostDirListResult {
        path: String::new(),
        display_path: String::new(),
        parent: None,
        entries: vec![],
        truncated: false,
        error: Some(message),
    }
}

fn is_daemon_running(workdir: &Path) -> bool {
    // `peek_lock_info`, not `read_lock_info`: the latter creates the workspace
    // config dir, which would litter one per browsed folder.
    workspace_lock::peek_lock_info(workdir)
        .is_some_and(|info| workspace_lock::is_process_alive(info.pid))
}

/// List the sub-directories of `user_path` for the client's project picker.
pub fn list_dirs(user_path: &str) -> HostDirListResult {
    let (home, path) = match home_dir().and_then(|home| {
        let path = resolve_in_home(&home, user_path)?;
        Ok((home, path))
    }) {
        Ok(pair) => pair,
        Err(e) => return dir_list_error(e.to_string()),
    };

    let read_dir = match std::fs::read_dir(&path) {
        Ok(read_dir) => read_dir,
        Err(e) => return dir_list_error(format!("{}: {e}", display_path(&home, &path))),
    };

    let mut dirs: Vec<PathBuf> = read_dir
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_dir()))
        .map(|entry| entry.path())
        .filter(|path| {
            !path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with('.'))
        })
        .collect();
    dirs.sort_by_key(|path| path.file_name().unwrap_or_default().to_ascii_lowercase());

    let truncated = dirs.len() > HOST_DIR_LIST_MAX_ENTRIES;
    let entries = dirs
        .into_iter()
        .take(HOST_DIR_LIST_MAX_ENTRIES)
        .map(|path| HostDirEntry {
            name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            is_running: is_daemon_running(&path),
            path: path.display().to_string(),
        })
        .collect();

    HostDirListResult {
        display_path: display_path(&home, &path),
        parent: (path != home).then(|| path.parent().unwrap_or(&home).display().to_string()),
        path: path.display().to_string(),
        entries,
        truncated,
        error: None,
    }
}

/// Start (or reuse) a daemon for `user_path` and mint a pairing ticket for it.
pub async fn open_workspace(user_path: &str) -> HostWorkspaceOpenResult {
    match open_workspace_inner(user_path).await {
        Ok(result) => result,
        Err(e) => HostWorkspaceOpenResult {
            pairing_url: String::new(),
            workdir: user_path.to_string(),
            already_running: false,
            error: Some(e.to_string()),
        },
    }
}

async fn open_workspace_inner(user_path: &str) -> Result<HostWorkspaceOpenResult> {
    ensure!(!user_path.is_empty(), "empty workdir");
    let home = home_dir()?;
    let workdir = resolve_in_home(&home, user_path)?;
    ensure!(workdir.is_dir(), "{} is not a directory", workdir.display());

    let already_running = is_daemon_running(&workdir);
    if !already_running {
        spawn_daemon(&workdir).await?;
    }

    let info = request_pairing_qr(&workdir, session_registry::PairingSlotMode::OneTime).await?;
    tracing::info!(
        "remote-open: paired {} (already_running={already_running})",
        workdir.display()
    );
    Ok(HostWorkspaceOpenResult {
        pairing_url: info.pairing_url,
        workdir: workdir.display().to_string(),
        already_running,
        error: None,
    })
}

async fn spawn_daemon(workdir: &Path) -> Result<()> {
    let exe = std::env::current_exe().map_err(|e| anyhow::anyhow!("zedra binary path: {e}"))?;
    // Relaunch flags come from the target workspace's own launch.yaml when it has
    // one, so reopening a workspace keeps the relay options it was started with.
    let launch = workspace_lock::lock_config_dir(workdir)
        .map(|dir| start_config::load(&dir))
        .unwrap_or_default();
    let workdir = workdir.to_path_buf();

    tracing::info!("remote-open: starting daemon for {}", workdir.display());
    let started = tokio::task::spawn_blocking(move || {
        start_detached(DetachedStartOptions {
            exe,
            workdir,
            verbose: launch.verbose,
            relay_url: launch.relay_url,
            no_telemetry: launch.no_telemetry,
            debug_telemetry: launch.debug_telemetry,
            relay_only: launch.relay_only,
            static_qr: launch.static_qr,
            usage_refresh_secs: launch.usage_refresh_secs,
        })
    })
    .await??;

    wait_for_detached_pairing_qr(
        &started.workdir,
        started.pid,
        session_registry::PairingSlotMode::OneTime,
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_rejects_paths_outside_home() {
        let home = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let home = home.path().canonicalize().unwrap();

        let inside = home.join("projects");
        std::fs::create_dir(&inside).unwrap();
        assert_eq!(
            resolve_in_home(&home, &inside.display().to_string()).unwrap(),
            inside.canonicalize().unwrap()
        );
        assert_eq!(resolve_in_home(&home, "").unwrap(), home);
        assert!(resolve_in_home(&home, &outside.path().display().to_string()).is_err());
    }

    #[test]
    fn display_path_uses_tilde() {
        let home = PathBuf::from("/Users/dev");
        assert_eq!(display_path(&home, &home), "~");
        assert_eq!(display_path(&home, &home.join("code/app")), "~/code/app");
        assert_eq!(display_path(&home, Path::new("/opt/app")), "/opt/app");
    }
}
