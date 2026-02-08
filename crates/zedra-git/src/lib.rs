// zedra-git: Git operations for Zedra
//
// Uses `git` CLI under the hood for zero-dependency simplicity.
// All operations are synchronous and work on the host side.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatusEntry {
    pub path: String,
    pub status: FileStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FileStatus {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    Conflicted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LogEntry {
    pub id: String,
    pub message: String,
    pub author: String,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BranchInfo {
    pub name: String,
    pub is_head: bool,
}

// ---------------------------------------------------------------------------
// GitRepo
// ---------------------------------------------------------------------------

pub struct GitRepo {
    workdir: PathBuf,
}

impl GitRepo {
    /// Open a git repository at the given path.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let workdir = path.into();
        // Verify it's a git repo
        let output = Command::new("git")
            .args(["rev-parse", "--git-dir"])
            .current_dir(&workdir)
            .output()
            .context("git not found")?;
        if !output.status.success() {
            anyhow::bail!("not a git repository: {}", workdir.display());
        }
        Ok(Self { workdir })
    }

    pub fn workdir(&self) -> &Path {
        &self.workdir
    }

    fn git(&self, args: &[&str]) -> Result<String> {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.workdir)
            .output()
            .with_context(|| format!("git {} failed", args.join(" ")))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("git {}: {}", args.join(" "), stderr.trim());
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// Current branch name.
    pub fn branch(&self) -> Result<String> {
        let out = self.git(&["branch", "--show-current"])?;
        Ok(out.trim().to_string())
    }

    /// Working tree status.
    pub fn status(&self) -> Result<Vec<StatusEntry>> {
        let out = self.git(&["status", "--porcelain=v1"])?;
        let mut entries = Vec::new();
        for line in out.lines() {
            if line.len() < 4 {
                continue;
            }
            let xy = &line[..2];
            let path = line[3..].to_string();
            let status = match xy.trim() {
                "M" | "MM" | "AM" => FileStatus::Modified,
                "A" => FileStatus::Added,
                "D" => FileStatus::Deleted,
                "R" => FileStatus::Renamed,
                "??" => FileStatus::Untracked,
                "UU" | "AA" | "DD" => FileStatus::Conflicted,
                _ => FileStatus::Modified,
            };
            entries.push(StatusEntry { path, status });
        }
        Ok(entries)
    }

    /// Diff output.
    pub fn diff(&self, path: Option<&str>, staged: bool) -> Result<String> {
        let mut args = vec!["diff"];
        if staged {
            args.push("--cached");
        }
        if let Some(p) = path {
            args.push("--");
            args.push(p);
        }
        self.git(&args)
    }

    /// Commit log.
    pub fn log(&self, limit: usize) -> Result<Vec<LogEntry>> {
        let limit_str = format!("-{}", limit);
        let out = self.git(&[
            "log",
            &limit_str,
            "--format=%H%n%s%n%an%n%at",
        ])?;
        let lines: Vec<&str> = out.lines().collect();
        let mut entries = Vec::new();
        for chunk in lines.chunks(4) {
            if chunk.len() < 4 {
                break;
            }
            entries.push(LogEntry {
                id: chunk[0].to_string(),
                message: chunk[1].to_string(),
                author: chunk[2].to_string(),
                timestamp: chunk[3].parse().unwrap_or(0),
            });
        }
        Ok(entries)
    }

    /// List branches.
    pub fn branches(&self) -> Result<Vec<BranchInfo>> {
        let out = self.git(&["branch", "--format=%(HEAD) %(refname:short)"])?;
        let mut branches = Vec::new();
        for line in out.lines() {
            let is_head = line.starts_with('*');
            let name = line[2..].trim().to_string();
            if !name.is_empty() {
                branches.push(BranchInfo { name, is_head });
            }
        }
        Ok(branches)
    }

    /// Checkout a branch.
    pub fn checkout(&self, branch: &str) -> Result<()> {
        self.git(&["checkout", branch])?;
        Ok(())
    }

    /// Stage files and commit.
    pub fn commit(&self, message: &str, paths: &[String]) -> Result<String> {
        if paths.is_empty() {
            anyhow::bail!("no paths to commit");
        }
        let mut add_args: Vec<&str> = vec!["add"];
        for p in paths {
            add_args.push(p.as_str());
        }
        self.git(&add_args)?;
        self.git(&["commit", "-m", message])?;
        let out = self.git(&["rev-parse", "HEAD"])?;
        Ok(out.trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn init_repo() -> (tempfile::TempDir, GitRepo) {
        let dir = tempfile::tempdir().unwrap();
        Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        let repo = GitRepo::open(dir.path()).unwrap();
        (dir, repo)
    }

    #[test]
    fn open_non_repo_fails() {
        let dir = tempfile::tempdir().unwrap();
        assert!(GitRepo::open(dir.path()).is_err());
    }

    #[test]
    fn branch_on_empty_repo() {
        let (_dir, repo) = init_repo();
        // Empty repo might not have a branch yet
        let branch = repo.branch().unwrap();
        assert!(branch == "main" || branch == "master" || branch.is_empty());
    }

    #[test]
    fn status_untracked() {
        let (dir, repo) = init_repo();
        std::fs::write(dir.path().join("new.txt"), "hello").unwrap();
        let status = repo.status().unwrap();
        assert_eq!(status.len(), 1);
        assert_eq!(status[0].status, FileStatus::Untracked);
        assert_eq!(status[0].path, "new.txt");
    }

    #[test]
    fn commit_and_log() {
        let (dir, repo) = init_repo();
        std::fs::write(dir.path().join("file.txt"), "content").unwrap();
        let hash = repo
            .commit("initial commit", &["file.txt".into()])
            .unwrap();
        assert_eq!(hash.len(), 40);

        let log = repo.log(10).unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].message, "initial commit");
        assert_eq!(log[0].author, "Test");
    }

    #[test]
    fn diff_modified() {
        let (dir, repo) = init_repo();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "line1\n").unwrap();
        repo.commit("add a", &["a.txt".into()]).unwrap();
        std::fs::write(&file, "line1\nline2\n").unwrap();

        let diff = repo.diff(None, false).unwrap();
        assert!(diff.contains("+line2"));
    }

    #[test]
    fn branches_list() {
        let (dir, repo) = init_repo();
        std::fs::write(dir.path().join("f.txt"), "x").unwrap();
        repo.commit("init", &["f.txt".into()]).unwrap();

        let branches = repo.branches().unwrap();
        assert!(!branches.is_empty());
        assert!(branches.iter().any(|b| b.is_head));
    }

    #[test]
    fn checkout_branch() {
        let (dir, repo) = init_repo();
        std::fs::write(dir.path().join("f.txt"), "x").unwrap();
        repo.commit("init", &["f.txt".into()]).unwrap();

        Command::new("git")
            .args(["branch", "feature"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        repo.checkout("feature").unwrap();
        assert_eq!(repo.branch().unwrap(), "feature");
    }

    #[test]
    fn status_entry_serde() {
        let entry = StatusEntry {
            path: "src/lib.rs".into(),
            status: FileStatus::Modified,
        };
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: StatusEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, entry);
    }
}
