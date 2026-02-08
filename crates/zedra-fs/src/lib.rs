// zedra-fs: Filesystem operations for Zedra
//
// Provides a trait-based filesystem abstraction with a local implementation.
// The trait allows swapping in remote (RPC-backed) implementations on mobile.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileStat {
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
    /// Seconds since UNIX epoch, if available.
    pub modified: Option<u64>,
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

pub trait Filesystem: Send + Sync {
    fn list(&self, path: &Path) -> Result<Vec<DirEntry>>;
    fn read(&self, path: &Path) -> Result<String>;
    fn write(&self, path: &Path, content: &str) -> Result<()>;
    fn stat(&self, path: &Path) -> Result<FileStat>;
    fn mkdir(&self, path: &Path) -> Result<()>;
    fn remove(&self, path: &Path) -> Result<()>;
}

// ---------------------------------------------------------------------------
// Local implementation
// ---------------------------------------------------------------------------

pub struct LocalFs;

impl Filesystem for LocalFs {
    fn list(&self, path: &Path) -> Result<Vec<DirEntry>> {
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let meta = entry.metadata()?;
            entries.push(DirEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                path: entry.path(),
                is_dir: meta.is_dir(),
                size: meta.len(),
            });
        }
        entries.sort_by(|a, b| {
            b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name))
        });
        Ok(entries)
    }

    fn read(&self, path: &Path) -> Result<String> {
        Ok(std::fs::read_to_string(path)?)
    }

    fn write(&self, path: &Path, content: &str) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Ok(std::fs::write(path, content)?)
    }

    fn stat(&self, path: &Path) -> Result<FileStat> {
        let meta = std::fs::metadata(path)?;
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs());
        Ok(FileStat {
            path: path.to_path_buf(),
            is_dir: meta.is_dir(),
            size: meta.len(),
            modified,
        })
    }

    fn mkdir(&self, path: &Path) -> Result<()> {
        Ok(std::fs::create_dir_all(path)?)
    }

    fn remove(&self, path: &Path) -> Result<()> {
        let meta = std::fs::metadata(path)?;
        if meta.is_dir() {
            std::fs::remove_dir_all(path)?;
        } else {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, LocalFs) {
        (tempfile::tempdir().unwrap(), LocalFs)
    }

    #[test]
    fn write_and_read() {
        let (dir, fs) = setup();
        let file = dir.path().join("hello.txt");
        fs.write(&file, "hello world").unwrap();
        assert_eq!(fs.read(&file).unwrap(), "hello world");
    }

    #[test]
    fn list_directory() {
        let (dir, fs) = setup();
        fs.write(&dir.path().join("a.txt"), "a").unwrap();
        fs.write(&dir.path().join("b.txt"), "b").unwrap();
        fs.mkdir(&dir.path().join("subdir")).unwrap();

        let entries = fs.list(dir.path()).unwrap();
        assert_eq!(entries.len(), 3);
        // Directories first
        assert!(entries[0].is_dir);
        assert_eq!(entries[0].name, "subdir");
    }

    #[test]
    fn stat_file() {
        let (dir, fs) = setup();
        let file = dir.path().join("test.txt");
        fs.write(&file, "content").unwrap();

        let stat = fs.stat(&file).unwrap();
        assert!(!stat.is_dir);
        assert_eq!(stat.size, 7);
        assert!(stat.modified.is_some());
    }

    #[test]
    fn stat_dir() {
        let (dir, fs) = setup();
        let sub = dir.path().join("sub");
        fs.mkdir(&sub).unwrap();

        let stat = fs.stat(&sub).unwrap();
        assert!(stat.is_dir);
    }

    #[test]
    fn mkdir_nested() {
        let (dir, fs) = setup();
        let nested = dir.path().join("a").join("b").join("c");
        fs.mkdir(&nested).unwrap();
        assert!(nested.exists());
    }

    #[test]
    fn remove_file() {
        let (dir, fs) = setup();
        let file = dir.path().join("rm_me.txt");
        fs.write(&file, "bye").unwrap();
        fs.remove(&file).unwrap();
        assert!(!file.exists());
    }

    #[test]
    fn remove_dir() {
        let (dir, fs) = setup();
        let sub = dir.path().join("rmdir");
        fs.mkdir(&sub).unwrap();
        fs.write(&sub.join("inner.txt"), "data").unwrap();
        fs.remove(&sub).unwrap();
        assert!(!sub.exists());
    }

    #[test]
    fn write_creates_parent_dirs() {
        let (dir, fs) = setup();
        let file = dir.path().join("deep").join("nested").join("file.txt");
        fs.write(&file, "deep").unwrap();
        assert_eq!(fs.read(&file).unwrap(), "deep");
    }

    #[test]
    fn read_nonexistent_fails() {
        let fs = LocalFs;
        assert!(fs.read(Path::new("/nonexistent/path/file")).is_err());
    }
}
