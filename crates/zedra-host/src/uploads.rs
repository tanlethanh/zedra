// Storage and lifecycle for client-uploaded image files.
//
// Uploads land in `~/.zedra/uploads/` on the host, under a host-chosen filename
// (`<unix_secs>-<uuid_v4>.<ext>`), and the returned path is absolute. Home-level
// (not project-scoped) keeps transient paste input out of every git repo and gives
// one predictable place to find it, mirroring how coding agents cache pasted images
// (`~/.claude/`, `~/.codex/`). `resolve_path` still gates the directory, since a
// pre-planted symlink there could otherwise escape the jail.

use crate::rpc_daemon::{current_home_dir, resolve_path};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

pub const UPLOADS_DIR: &str = ".zedra/uploads";
/// How long an uploaded file is kept before the cleanup sweep removes it.
pub const UPLOAD_GRACE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const CLEANUP_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const ALLOWED_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "gif"];

/// The host user's home directory, the base for `~/.zedra/uploads/`.
fn uploads_base() -> Result<PathBuf> {
    current_home_dir()
        .map(PathBuf::from)
        .context("could not determine home directory for uploads")
}

/// Lowercases and validates a client-supplied extension against an allowlist.
/// Never trusts the client filename otherwise — only the extension is used,
/// and only after this check passes.
pub fn sanitize_extension(ext: &str) -> Result<&'static str> {
    let lower = ext.trim().to_ascii_lowercase();
    ALLOWED_EXTENSIONS
        .iter()
        .find(|allowed| **allowed == lower)
        .copied()
        .with_context(|| format!("unsupported image extension: {ext:?}"))
}

/// Writes `data` into `~/.zedra/uploads/<unix_secs>-<uuid>.<ext>`, creating the
/// directory if needed. Returns the absolute path as a string.
pub fn store_upload(data: &[u8], extension: &str) -> Result<String> {
    store_upload_in(&uploads_base()?, data, extension)
}

fn store_upload_in(base: &Path, data: &[u8], extension: &str) -> Result<String> {
    let ext = sanitize_extension(extension)?;
    let dir = resolve_path(base, UPLOADS_DIR)?;
    std::fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;

    let unix_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let filename = format!("{unix_secs}-{}.{ext}", Uuid::new_v4());
    let path = dir.join(&filename);
    std::fs::write(&path, data).with_context(|| format!("failed to write {}", path.display()))?;

    Ok(path.to_string_lossy().into_owned())
}

/// Deletes uploaded files older than `grace`. Returns the number of files removed.
/// Missing directory is not an error.
pub fn cleanup_uploads(grace: Duration) -> Result<usize> {
    cleanup_uploads_in(&uploads_base()?, grace)
}

fn cleanup_uploads_in(base: &Path, grace: Duration) -> Result<usize> {
    let dir = resolve_path(base, UPLOADS_DIR)?;
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e).with_context(|| format!("failed to read {}", dir.display())),
    };

    let now = SystemTime::now();
    let mut removed = 0;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        let Ok(modified) = meta.modified() else {
            continue;
        };
        let Ok(age) = now.duration_since(modified) else {
            continue;
        };
        if age > grace && std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

/// Sweeps stale uploads once immediately, then on a fixed interval for the
/// lifetime of the daemon. Failures are logged and never abort the loop.
pub async fn run_cleanup_loop() {
    match cleanup_uploads(UPLOAD_GRACE) {
        Ok(0) => {}
        Ok(n) => tracing::info!("uploads: swept {n} stale file(s) on startup"),
        Err(e) => tracing::warn!("uploads: startup sweep failed: {e:#}"),
    }

    let mut interval = tokio::time::interval(CLEANUP_INTERVAL);
    interval.tick().await; // skip immediate re-run — startup sweep covers it
    loop {
        interval.tick().await;
        match cleanup_uploads(UPLOAD_GRACE) {
            Ok(0) => {}
            Ok(n) => tracing::info!("uploads: swept {n} stale file(s)"),
            Err(e) => tracing::warn!("uploads: periodic sweep failed: {e:#}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use filetime::{set_file_mtime, FileTime};

    #[test]
    fn sanitize_extension_allows_known_types() {
        assert_eq!(sanitize_extension("JPG").unwrap(), "jpg");
        assert_eq!(sanitize_extension(" png ").unwrap(), "png");
    }

    #[test]
    fn sanitize_extension_rejects_unknown_types() {
        assert!(sanitize_extension("exe").is_err());
        assert!(sanitize_extension("").is_err());
        assert!(sanitize_extension("../x").is_err());
    }

    #[test]
    fn store_upload_writes_file_and_returns_absolute_path() {
        let base = tempfile::tempdir().unwrap();
        let path = store_upload_in(base.path(), b"fake-bytes", "jpg").unwrap();

        assert!(Path::new(&path).is_absolute());
        assert!(path.ends_with(".jpg"));
        let uploads = base.path().join(UPLOADS_DIR).canonicalize().unwrap();
        assert!(Path::new(&path).starts_with(&uploads));

        let filename = Path::new(&path).file_name().unwrap().to_str().unwrap();
        let (secs, rest) = filename.split_once('-').expect("timestamp-uuid separator");
        assert!(secs.chars().all(|c| c.is_ascii_digit()));
        let uuid_part = rest.strip_suffix(".jpg").unwrap();
        assert!(
            Uuid::parse_str(uuid_part).is_ok(),
            "not a uuid: {uuid_part}"
        );

        assert_eq!(std::fs::read(&path).unwrap(), b"fake-bytes");
    }

    #[test]
    fn store_upload_rejects_unsupported_extension() {
        let base = tempfile::tempdir().unwrap();
        assert!(store_upload_in(base.path(), b"data", "exe").is_err());
    }

    #[test]
    fn store_upload_rejects_symlinked_zedra_dir_escaping_workspace() {
        let workdir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();

        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), workdir.path().join(".zedra")).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(outside.path(), workdir.path().join(".zedra")).unwrap();

        let result = store_upload_in(workdir.path(), b"data", "jpg");
        assert!(result.is_err(), "expected jail escape to be rejected");
        assert!(
            std::fs::read_dir(outside.path()).unwrap().next().is_none(),
            "no file should have been written outside the workspace"
        );
    }

    #[test]
    fn store_upload_rejects_symlinked_uploads_dir_escaping_workspace() {
        let workdir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(workdir.path().join(".zedra")).unwrap();

        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), workdir.path().join(".zedra/uploads")).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(outside.path(), workdir.path().join(".zedra/uploads"))
            .unwrap();

        let result = store_upload_in(workdir.path(), b"data", "jpg");
        assert!(result.is_err(), "expected jail escape to be rejected");
        assert!(
            std::fs::read_dir(outside.path()).unwrap().next().is_none(),
            "no file should have been written outside the workspace"
        );
    }

    #[test]
    fn cleanup_uploads_rejects_symlinked_uploads_dir_escaping_workspace() {
        let workdir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(workdir.path().join(".zedra")).unwrap();
        std::fs::write(outside.path().join("secret.jpg"), b"data").unwrap();

        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), workdir.path().join(".zedra/uploads")).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(outside.path(), workdir.path().join(".zedra/uploads"))
            .unwrap();

        let result = cleanup_uploads_in(workdir.path(), Duration::from_secs(0));
        assert!(result.is_err(), "expected jail escape to be rejected");
        assert!(
            outside.path().join("secret.jpg").exists(),
            "file outside the workspace must survive cleanup"
        );
    }

    #[test]
    fn cleanup_uploads_removes_only_stale_files() {
        let dir = tempfile::tempdir().unwrap();
        let uploads_dir = dir.path().join(UPLOADS_DIR);
        std::fs::create_dir_all(&uploads_dir).unwrap();

        let stale = uploads_dir.join("1-stale.jpg");
        std::fs::write(&stale, b"old").unwrap();
        let old_time =
            FileTime::from_system_time(SystemTime::now() - Duration::from_secs(8 * 86400));
        set_file_mtime(&stale, old_time).unwrap();

        let fresh = uploads_dir.join("2-fresh.jpg");
        std::fs::write(&fresh, b"new").unwrap();

        let removed = cleanup_uploads_in(dir.path(), Duration::from_secs(7 * 86400)).unwrap();
        assert_eq!(removed, 1);
        assert!(!stale.exists());
        assert!(fresh.exists());
    }

    #[test]
    fn cleanup_uploads_missing_dir_is_ok() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(cleanup_uploads_in(dir.path(), UPLOAD_GRACE).unwrap(), 0);
    }
}
