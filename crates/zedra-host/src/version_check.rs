//! Version checking and self-update for zedra-host.
//!
//! Resolves the latest release from GitHub and optionally downloads + replaces
//! the running binary.

use crate::utils;
use anyhow::{bail, Context, Result};
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

    eprintln!("  Downloading {archive_url}...");
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
        eprintln!("  Checksum verified.");
    } else {
        utils::eprintln_warn("  Warning: checksum verification skipped (unavailable).");
    }

    eprintln!("  Extracting...");
    let archive_file = std::fs::File::open(&archive_path)?;
    let decoder = flate2::read::GzDecoder::new(archive_file);
    let mut archive = tar::Archive::new(decoder);
    archive.unpack(tmp_dir.path())?;

    let extracted = tmp_dir.path().join("zedra");
    if !extracted.exists() {
        bail!("archive did not contain 'zedra' binary");
    }

    // Replace current binary via atomic rename
    let current_exe =
        std::env::current_exe().context("cannot determine current executable path")?;
    let current_exe = current_exe
        .canonicalize()
        .unwrap_or_else(|_| current_exe.clone());

    // On macOS/Linux: rename the old binary out of the way, move new one in,
    // then delete old. This avoids "text file busy" on some systems.
    let backup = current_exe.with_extension("old");
    if backup.exists() {
        let _ = std::fs::remove_file(&backup);
    }
    std::fs::rename(&current_exe, &backup).with_context(|| {
        format!(
            "failed to rename current binary at {}",
            current_exe.display()
        )
    })?;
    match std::fs::copy(&extracted, &current_exe) {
        Ok(_) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ =
                    std::fs::set_permissions(&current_exe, std::fs::Permissions::from_mode(0o755));
            }
            let _ = std::fs::remove_file(&backup);
        }
        Err(e) => {
            // Rollback
            let _ = std::fs::rename(&backup, &current_exe);
            bail!("failed to install new binary: {e}");
        }
    }

    Ok(tag)
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
        assert!(p.contains("apple-darwin") || p.contains("unknown-linux-gnu"));
    }
}
