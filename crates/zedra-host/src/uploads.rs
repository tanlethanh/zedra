// Storage and lifecycle for client-uploaded image files.
//
// Uploads land in Zedra's cache directory under a host-chosen filename
// (`<unix_secs>-<uuid_v4>.<ext>`), and the returned path is absolute. Host-level
// (not project-scoped) storage keeps transient paste input out of every git repo.
// `resolve_path` still gates the directory, since a pre-planted symlink there could
// otherwise escape the jail.

#[cfg(not(windows))]
use crate::rpc_daemon::current_home_dir;
use crate::rpc_daemon::resolve_path;
use anyhow::{Context, Result};
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[cfg(test)]
const CACHE_DIR: &str = "zedra";
pub const UPLOADS_DIR: &str = "zedra/uploads";
/// How long an uploaded file is kept before the cleanup sweep removes it.
pub const UPLOAD_GRACE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const CLEANUP_LOCK: &str = "zedra/uploads.cleanup.lock";
const ALLOWED_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "gif"];

/// The cache root, used as the jail for Zedra's upload cache.
fn uploads_base() -> Result<PathBuf> {
    #[cfg(not(windows))]
    {
        return current_home_dir()
            .map(PathBuf::from)
            .map(|home| home.join(".cache"))
            .context("could not determine home directory for upload cache");
    }

    #[cfg(windows)]
    {
        directories::BaseDirs::new()
            .map(|dirs| dirs.cache_dir().to_path_buf())
            .context("could not determine cache directory for uploads")
    }
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

/// Writes `data` into the Zedra upload cache, creating the directory if needed.
/// Returns the absolute path as a string.
pub fn store_upload(data: &[u8], extension: &str) -> Result<String> {
    let base = uploads_base()?;
    std::fs::create_dir_all(&base)
        .with_context(|| format!("failed to create cache root {}", base.display()))?;
    store_upload_in(&base, data, extension)
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

/// Runs one cleanup sweep on Tokio's blocking pool. Another Zedra daemon holding
/// the cleanup lock makes this startup skip immediately.
pub fn spawn_startup_cleanup() {
    tokio::task::spawn_blocking(|| match cleanup_uploads_on_start() {
        Ok(Some(0)) => {}
        Ok(Some(removed)) => tracing::info!(removed, "uploads: swept stale files on startup"),
        Ok(None) => tracing::debug!("uploads: skip startup sweep; cleanup lock is held"),
        Err(error) => tracing::warn!(%error, "uploads: startup sweep failed"),
    });
}

fn cleanup_uploads_on_start() -> Result<Option<usize>> {
    let base = uploads_base()?;
    match std::fs::metadata(&base) {
        Ok(_) => cleanup_uploads_on_start_in(&base, UPLOAD_GRACE),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Some(0)),
        Err(error) => Err(error).context("failed to inspect cache directory"),
    }
}

fn cleanup_uploads_on_start_in(base: &Path, grace: Duration) -> Result<Option<usize>> {
    let dir = resolve_path(base, UPLOADS_DIR)?;
    match std::fs::metadata(&dir) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Some(0)),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to inspect {}", dir.display()))
        }
    }

    let lock_path = resolve_path(base, CLEANUP_LOCK)?;
    let lock_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .with_context(|| format!("failed to open cleanup lock {}", lock_path.display()))?;
    if let Err(error) = fs2::FileExt::try_lock_exclusive(&lock_file) {
        if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() {
            return Ok(None);
        }
        return Err(error).context("failed to lock upload cleanup");
    }

    cleanup_uploads_in(base, grace).map(Some)
}

/// Deletes uploaded files older than `grace`. Returns the number of files removed.
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

    #[cfg(not(windows))]
    #[test]
    fn uploads_base_uses_dot_cache_on_unix() {
        let home = PathBuf::from(current_home_dir().unwrap());
        assert_eq!(uploads_base().unwrap(), home.join(".cache"));
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
    fn store_upload_rejects_symlinked_zedra_dir_escaping_cache() {
        let cache = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();

        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), cache.path().join(CACHE_DIR)).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(outside.path(), cache.path().join(CACHE_DIR)).unwrap();

        let result = store_upload_in(cache.path(), b"data", "jpg");
        assert!(result.is_err(), "expected jail escape to be rejected");
        assert!(
            std::fs::read_dir(outside.path()).unwrap().next().is_none(),
            "no file should have been written outside the cache"
        );
    }

    #[test]
    fn store_upload_rejects_symlinked_uploads_dir_escaping_cache() {
        let cache = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(cache.path().join(CACHE_DIR)).unwrap();

        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), cache.path().join(UPLOADS_DIR)).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(outside.path(), cache.path().join(UPLOADS_DIR)).unwrap();

        let result = store_upload_in(cache.path(), b"data", "jpg");
        assert!(result.is_err(), "expected jail escape to be rejected");
        assert!(
            std::fs::read_dir(outside.path()).unwrap().next().is_none(),
            "no file should have been written outside the cache"
        );
    }

    #[test]
    fn cleanup_uploads_rejects_symlinked_uploads_dir_escaping_cache() {
        let cache = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(cache.path().join(CACHE_DIR)).unwrap();
        std::fs::write(outside.path().join("secret.jpg"), b"data").unwrap();

        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), cache.path().join(UPLOADS_DIR)).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(outside.path(), cache.path().join(UPLOADS_DIR)).unwrap();

        let result = cleanup_uploads_in(cache.path(), Duration::from_secs(0));
        assert!(result.is_err(), "expected jail escape to be rejected");
        assert!(
            outside.path().join("secret.jpg").exists(),
            "file outside the cache must survive cleanup"
        );
    }

    #[test]
    fn startup_cleanup_removes_only_stale_files() {
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

        let removed = cleanup_uploads_on_start_in(dir.path(), Duration::from_secs(7 * 86400))
            .unwrap()
            .unwrap();
        assert_eq!(removed, 1);
        assert!(!stale.exists());
        assert!(fresh.exists());
    }

    #[test]
    fn startup_cleanup_missing_dir_is_ok() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            cleanup_uploads_on_start_in(dir.path(), UPLOAD_GRACE).unwrap(),
            Some(0)
        );
        assert!(!dir.path().join(CACHE_DIR).exists());
    }

    #[test]
    fn startup_cleanup_skips_when_lock_is_held() {
        let base = tempfile::tempdir().unwrap();
        let uploads_dir = base.path().join(UPLOADS_DIR);
        std::fs::create_dir_all(&uploads_dir).unwrap();
        let stale = uploads_dir.join("1-stale.jpg");
        std::fs::write(&stale, b"old").unwrap();
        let old_time =
            FileTime::from_system_time(SystemTime::now() - Duration::from_secs(8 * 86400));
        set_file_mtime(&stale, old_time).unwrap();

        let lock_path = base.path().join(CLEANUP_LOCK);
        let lock_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)
            .unwrap();
        fs2::FileExt::try_lock_exclusive(&lock_file).unwrap();

        assert_eq!(
            cleanup_uploads_on_start_in(base.path(), Duration::from_secs(0)).unwrap(),
            None
        );
        assert!(stale.exists());
    }
}
