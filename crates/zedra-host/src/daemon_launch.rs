//! Spawning and pairing detached daemons for other workspaces.
//!
//! Used by `zedra start --detach`, the post-update restart, and the app's
//! remote "Open project" flow, which reaches this through `HostWorkspaceOpen`.

use anyhow::{Context, Result};

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::{identity, qr, session_registry, workspace_lock};

pub struct DetachedStartOptions {
    /// Binary to spawn; resolved by the caller because `current_exe()` can dangle after a self-update rename.
    pub exe: PathBuf,
    pub workdir: PathBuf,
    pub verbose: bool,
    pub relay_url: Vec<String>,
    pub no_telemetry: bool,
    pub debug_telemetry: bool,
    pub relay_only: bool,
    pub static_qr: bool,
    pub usage_refresh_secs: u64,
}

pub struct DetachedStartResult {
    pub pid: u32,
    pub workdir: PathBuf,
}

pub fn detached_start_child_args(options: &DetachedStartOptions) -> Vec<String> {
    let mut args = Vec::new();
    if options.verbose {
        args.push("--verbose".to_string());
    }
    args.extend([
        "start".to_string(),
        "--workdir".to_string(),
        options.workdir.display().to_string(),
    ]);
    for relay_url in &options.relay_url {
        args.extend(["--relay-url".to_string(), relay_url.clone()]);
    }
    if options.no_telemetry {
        args.push("--no-telemetry".to_string());
    }
    if options.debug_telemetry {
        args.push("--debug-telemetry".to_string());
    }
    if options.relay_only {
        args.push("--relay-only".to_string());
    }
    if options.static_qr {
        args.push("--static-qr".to_string());
    }
    if options.usage_refresh_secs != 300 {
        args.extend([
            "--usage-refresh-secs".to_string(),
            options.usage_refresh_secs.to_string(),
        ]);
    }
    args
}

#[cfg(unix)]
pub fn start_detached(options: DetachedStartOptions) -> Result<DetachedStartResult> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;

    if let Some(existing) = workspace_lock::read_lock_info(&options.workdir)? {
        if workspace_lock::is_process_alive(existing.pid) {
            anyhow::bail!(
                "Zedra daemon is already running for this workspace.\n\
                 \n\
                 \x20 PID:      {}\n\
                 \x20 Workdir:  {}\n\
                 \x20 Host:     {}\n\
                 \x20 Started:  {}\n\
                 \n\
                 Run `zedra stop` from this workspace to stop it.\n\
                 From another directory, add `--workdir <path>`.",
                existing.pid,
                existing.workdir,
                existing.hostname,
                existing.running_for(),
            );
        }
    }

    let config_dir = identity::workspace_config_dir(&options.workdir)?;
    std::fs::create_dir_all(&config_dir)?;
    let log_path = config_dir.join("daemon.log");
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&log_path)?;
    std::fs::set_permissions(&log_path, std::fs::Permissions::from_mode(0o600))?;
    writeln!(
        log,
        "\n--- zedra detached start parent_pid={} workdir={} ---",
        std::process::id(),
        options.workdir.display()
    )?;

    let mut command = std::process::Command::new(&options.exe);
    command
        .args(detached_start_child_args(&options))
        .current_dir(&options.workdir)
        .env("ZEDRA_DETACHED", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log));

    unsafe {
        command.pre_exec(|| {
            // Detached hosts must leave the SSH session's process group and
            // controlling terminal, otherwise logout can still deliver SIGHUP.
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let mut child = command.spawn()?;
    let child_pid = child.id();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        if let Some(status) = child.try_wait()? {
            anyhow::bail!(
                "detached zedra-host exited early with status {}. See log: {}",
                status,
                log_path.display()
            );
        }
        if let Some(info) = workspace_lock::read_lock_info(&options.workdir)? {
            if info.pid == child_pid {
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    Ok(DetachedStartResult {
        pid: child_pid,
        workdir: options.workdir,
    })
}

#[cfg(windows)]
pub fn start_detached(options: DetachedStartOptions) -> Result<DetachedStartResult> {
    use std::os::windows::process::CommandExt;
    use std::process::Stdio;

    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const DETACHED_PROCESS: u32 = 0x0000_0008;

    if let Some(existing) = workspace_lock::read_lock_info(&options.workdir)? {
        if workspace_lock::is_process_alive(existing.pid) {
            anyhow::bail!(
                "Zedra daemon is already running for this workspace.\n\
                 \n\
                 \x20 PID:      {}\n\
                 \x20 Workdir:  {}\n\
                 \x20 Host:     {}\n\
                 \x20 Started:  {}\n\
                 \n\
                 Run `zedra stop` from this workspace to stop it.\n\
                 From another directory, add `--workdir <path>`.",
                existing.pid,
                existing.workdir,
                existing.hostname,
                existing.running_for(),
            );
        }
    }

    let config_dir = identity::workspace_config_dir(&options.workdir)?;
    std::fs::create_dir_all(&config_dir)?;
    let log_path = config_dir.join("daemon.log");
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    writeln!(
        log,
        "\n--- zedra detached start parent_pid={} workdir={} ---",
        std::process::id(),
        options.workdir.display()
    )?;
    let launch_shell = crate::pty::detect_parent_shell();
    if let Some(shell) = &launch_shell {
        writeln!(log, "detected_launch_shell={shell}")?;
    }

    let mut command = std::process::Command::new(&options.exe);
    command
        .args(detached_start_child_args(&options))
        .current_dir(&options.workdir)
        .env("ZEDRA_DETACHED", "1")
        .creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log));
    if let Some(shell) = launch_shell {
        command.env("ZEDRA_LAUNCH_SHELL", shell);
    }

    let mut child = command.spawn()?;
    let child_pid = child.id();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        if let Some(status) = child.try_wait()? {
            anyhow::bail!(
                "detached zedra-host exited early with status {}. See log: {}",
                status,
                log_path.display()
            );
        }
        if let Some(info) = workspace_lock::read_lock_info(&options.workdir)? {
            if info.pid == child_pid {
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    Ok(DetachedStartResult {
        pid: child_pid,
        workdir: options.workdir,
    })
}

#[cfg(all(not(unix), not(windows)))]
pub fn start_detached(_options: DetachedStartOptions) -> Result<DetachedStartResult> {
    anyhow::bail!("`zedra start --detach` is not supported on this platform.");
}

pub async fn request_pairing_qr(
    workdir: &Path,
    mode: session_registry::PairingSlotMode,
) -> Result<qr::StartupInfo> {
    let config_dir = identity::workspace_config_dir(workdir)?;
    let addr = std::fs::read_to_string(config_dir.join("api-addr")).unwrap_or_default();
    let token = std::fs::read_to_string(config_dir.join("api-token")).unwrap_or_default();
    if addr.trim().is_empty() {
        anyhow::bail!("No running daemon found for: {}", workdir.display());
    }

    let path = match mode {
        session_registry::PairingSlotMode::OneTime => "/api/qr",
        session_registry::PairingSlotMode::Static => "/api/qr/static",
    };
    let url = format!("http://{}{}", addr.trim(), path);
    let resp = reqwest::Client::new()
        .post(&url)
        .bearer_auth(token.trim())
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        if status == reqwest::StatusCode::NOT_FOUND {
            if mode == session_registry::PairingSlotMode::Static {
                anyhow::bail!(
                    "Running daemon does not support `zedra qr --static`; restart it with the updated zedra binary."
                );
            } else {
                anyhow::bail!(
                    "Running daemon does not support `zedra qr`; restart it with the updated zedra binary."
                );
            }
        }
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Failed to request pairing QR: HTTP {} {}", status, body);
    }

    resp.json::<qr::StartupInfo>().await.map_err(Into::into)
}

pub async fn wait_for_detached_pairing_qr(
    workdir: &Path,
    pid: u32,
    mode: session_registry::PairingSlotMode,
) -> Result<qr::StartupInfo> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);

    loop {
        if !workspace_lock::is_process_alive(pid) {
            anyhow::bail!("detached daemon exited before its pairing QR was ready");
        }

        let err = match request_pairing_qr(workdir, mode).await {
            Ok(info) => return Ok(info),
            Err(err) => err,
        };

        if tokio::time::Instant::now() >= deadline {
            return Err(err).context("pairing QR was not ready before the startup timeout");
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
}
