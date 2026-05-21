//! Version checking and self-update for zedra-host.
//!
//! Resolves the latest release from GitHub and optionally downloads + replaces
//! the running binary.

use crate::utils;
use anyhow::{bail, Context, Result};
use std::path::Path;

const REPO: &str = "tanlethanh/zedra";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Returns the latest release tag if it is newer than the current version,
/// or `None` if already up-to-date.
pub async fn check_latest_version() -> Result<Option<String>> {
    let tag = resolve_latest_tag().await?;
    let latest = tag.strip_prefix('v').unwrap_or(&tag);
    if is_newer(latest, CURRENT_VERSION) {
        Ok(Some(tag))
    } else {
        Ok(None)
    }
}

/// Download the specified release and replace the current binary.
pub async fn self_update(tag: &str) -> Result<String> {
    let tag = tag.to_string();

    let platform = detect_platform()?;

    let base_url = format!("https://github.com/{REPO}/releases/download/{tag}");
    let archive_name = format!("zedra-{platform}.tar.gz");
    let archive_url = format!("{base_url}/{archive_name}");
    let checksum_url = format!("{base_url}/{archive_name}.sha256");

    let tmp_dir = tempfile::tempdir().context("failed to create temp dir")?;

    utils::eprintln_step(format!("Downloading {archive_url}"));
    let client = reqwest::Client::new();
    let archive_bytes = client
        .get(&archive_url)
        .send()
        .await?
        .error_for_status()
        .with_context(|| format!("download failed — does release '{tag}' exist?"))?
        .bytes()
        .await?;

    let archive_path = tmp_dir.path().join(&archive_name);
    tokio::fs::write(&archive_path, &archive_bytes).await?;

    // Verify SHA256 checksum (best-effort: skip if .sha256 file unavailable)
    let checksum_verified = if let Ok(resp) = client.get(&checksum_url).send().await {
        if let Ok(body) = resp.text().await {
            let expected = body.split_whitespace().next().unwrap_or("");
            if !expected.is_empty() {
                let actual = sha256_hex(&archive_bytes);
                if actual != expected {
                    bail!("checksum mismatch!\n  expected: {expected}\n  actual:   {actual}");
                }
                true
            } else {
                false
            }
        } else {
            false
        }
    } else {
        false
    };
    if checksum_verified {
        utils::eprintln_success("Checksum verified.");
    } else {
        utils::eprintln_warn("Checksum verification skipped (unavailable).");
    }

    utils::eprintln_step("Extracting");
    let archive_file = std::fs::File::open(&archive_path)?;
    let decoder = flate2::read::GzDecoder::new(archive_file);
    let mut archive = tar::Archive::new(decoder);
    archive.unpack(tmp_dir.path())?;

    let extracted = tmp_dir.path().join(binary_name());
    if !extracted.exists() {
        bail!("archive did not contain '{}' binary", binary_name());
    }

    let current_exe =
        std::env::current_exe().context("cannot determine current executable path")?;
    let current_exe = current_exe
        .canonicalize()
        .unwrap_or_else(|_| current_exe.clone());

    replace_current_binary(&extracted, &current_exe, tmp_dir)?;

    Ok(tag)
}

#[cfg(windows)]
const WINDOWS_UPDATE_SCRIPT: &str = r#"param(
    [int]$ParentPid,
    [string]$Source,
    [string]$Destination,
    [string]$StagingDir
)

$ErrorActionPreference = "Stop"
Wait-Process -Id $ParentPid -ErrorAction SilentlyContinue

$backup = "$Destination.old.$([Guid]::NewGuid().ToString("N"))"
try {
    Move-Item -LiteralPath $Destination -Destination $backup -Force
    Copy-Item -LiteralPath $Source -Destination $Destination -Force

    # Running daemons can keep the renamed image locked, so old backups are
    # intentionally best-effort cleanup. Ref: https://docs.rs/self-replace/latest/self_replace/#implementation
    Remove-Item -LiteralPath $backup -Force -ErrorAction SilentlyContinue
} catch {
    if ((Test-Path -LiteralPath $backup) -and -not (Test-Path -LiteralPath $Destination)) {
        Move-Item -LiteralPath $backup -Destination $Destination -Force
    }
    throw
} finally {
    Remove-Item -LiteralPath $StagingDir -Recurse -Force -ErrorAction SilentlyContinue
}
"#;

fn binary_name() -> &'static str {
    if cfg!(windows) {
        "zedra.exe"
    } else {
        "zedra"
    }
}

#[cfg(not(windows))]
fn replace_current_binary(
    extracted: &Path,
    current_exe: &Path,
    _tmp_dir: tempfile::TempDir,
) -> Result<()> {
    // Rename the old binary out of the way, move new one in, then delete old.
    // This avoids "text file busy" on some systems.
    let backup = current_exe.with_extension("old");
    if backup.exists() {
        let _ = std::fs::remove_file(&backup);
    }
    std::fs::rename(current_exe, &backup).with_context(|| {
        format!(
            "failed to rename current binary at {}",
            current_exe.display()
        )
    })?;
    match std::fs::copy(extracted, current_exe) {
        Ok(_) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ =
                    std::fs::set_permissions(current_exe, std::fs::Permissions::from_mode(0o755));
            }
            let _ = std::fs::remove_file(&backup);
        }
        Err(e) => {
            // Rollback
            let _ = std::fs::rename(&backup, current_exe);
            bail!("failed to install new binary: {e}");
        }
    }

    Ok(())
}

#[cfg(windows)]
fn replace_current_binary(
    extracted: &Path,
    current_exe: &Path,
    tmp_dir: tempfile::TempDir,
) -> Result<()> {
    assert_parent_writable(current_exe)?;

    let script_path = tmp_dir.path().join("finish-update.ps1");
    std::fs::write(&script_path, WINDOWS_UPDATE_SCRIPT)
        .context("failed to write Windows update helper")?;

    let staging_dir = tmp_dir.keep();
    let spawn_result =
        spawn_windows_update_helper(&script_path, extracted, current_exe, &staging_dir);
    if let Err(err) = spawn_result {
        let _ = std::fs::remove_dir_all(&staging_dir);
        return Err(err);
    }

    utils::eprintln_note("Windows will finish replacing zedra.exe after this command exits.");
    Ok(())
}

#[cfg(windows)]
fn assert_parent_writable(current_exe: &Path) -> Result<()> {
    let parent = current_exe
        .parent()
        .context("cannot determine current executable directory")?;
    let probe = parent.join(format!(".zedra-update-write-test-{}", std::process::id()));
    std::fs::write(&probe, b"zedra")
        .with_context(|| format!("install directory is not writable: {}", parent.display()))?;
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

#[cfg(windows)]
fn spawn_windows_update_helper(
    script_path: &Path,
    extracted: &Path,
    current_exe: &Path,
    staging_dir: &Path,
) -> Result<()> {
    let parent_pid = std::process::id().to_string();
    let mut last_error = None;
    for shell in ["powershell.exe", "pwsh.exe"] {
        let result = std::process::Command::new(shell)
            .arg("-NoProfile")
            .arg("-ExecutionPolicy")
            .arg("Bypass")
            .arg("-File")
            .arg(script_path)
            .arg(parent_pid.as_str())
            .arg(extracted)
            .arg(current_exe)
            .arg(staging_dir)
            .spawn();

        match result {
            Ok(_) => return Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                last_error = Some(err);
            }
            Err(err) => {
                return Err(err).with_context(|| format!("failed to start {shell}"));
            }
        }
    }

    match last_error {
        Some(err) => Err(err).context("failed to start PowerShell for Windows self-update"),
        None => bail!("failed to start PowerShell for Windows self-update"),
    }
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

async fn resolve_latest_tag() -> Result<String> {
    let url = format!("https://github.com/{REPO}/releases/latest");
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let resp = client.head(&url).send().await?;

    // GitHub responds with 302 → Location: .../releases/tag/vX.Y.Z
    let location = resp
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let tag = location.rsplit('/').next().unwrap_or("");
    if tag.is_empty() || !tag.starts_with('v') {
        bail!("failed to resolve latest release tag from GitHub");
    }
    Ok(tag.to_string())
}

fn detect_platform() -> Result<String> {
    let arch = std::env::consts::ARCH;
    let os = std::env::consts::OS;

    let arch = match arch {
        "aarch64" => "aarch64",
        "x86_64" => "x86_64",
        other => bail!("unsupported architecture: {other}"),
    };
    let os = match os {
        "macos" => "apple-darwin",
        "linux" => "unknown-linux-gnu",
        "windows" => "pc-windows-msvc",
        other => bail!("unsupported OS: {other}"),
    };

    Ok(format!("{arch}-{os}"))
}

/// Simple semver comparison: returns true if `a` > `b`.
fn is_newer(a: &str, b: &str) -> bool {
    let parse = |s: &str| -> (u64, u64, u64) {
        let mut parts = s.splitn(3, '.');
        let major = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        let minor = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        let patch = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        (major, minor, patch)
    };
    parse(a) > parse(b)
}

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write;
    let hash = Sha256::digest(data);
    let mut out = String::with_capacity(64);
    for b in hash {
        let _ = write!(out, "{b:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_newer() {
        assert!(is_newer("0.2.0", "0.1.1"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(is_newer("0.1.2", "0.1.1"));
        assert!(!is_newer("0.1.1", "0.1.1"));
        assert!(!is_newer("0.1.0", "0.1.1"));
    }

    #[test]
    fn test_detect_platform() {
        // Should not fail on the current platform
        let p = detect_platform().unwrap();
        assert!(
            p.contains("apple-darwin")
                || p.contains("unknown-linux-gnu")
                || p.contains("pc-windows-msvc")
        );
    }

    #[test]
    fn test_binary_name_matches_platform() {
        if cfg!(windows) {
            assert_eq!(binary_name(), "zedra.exe");
        } else {
            assert_eq!(binary_name(), "zedra");
        }
    }
}
