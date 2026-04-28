// Git operations for Zedra
//
// Uses `git` CLI under the hood for zero-dependency simplicity.
// All operations are synchronous and work on the host side.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatusEntry {
    pub path: String,
    pub staged_status: Option<FileStatus>,
    pub unstaged_status: Option<FileStatus>,
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

    fn git_ok(&self, args: &[&str]) -> Result<()> {
        self.git(args)?;
        Ok(())
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
            if line.len() < 3 {
                continue;
            }
            if line.starts_with("?? ") {
                entries.push(StatusEntry {
                    path: parse_status_path(&line[3..]),
                    staged_status: None,
                    unstaged_status: Some(FileStatus::Untracked),
                });
                continue;
            }
            if line.len() < 4 {
                continue;
            }

            let xy = &line[..2];
            let mut chars = xy.chars();
            let staged_code = chars.next().unwrap_or(' ');
            let unstaged_code = chars.next().unwrap_or(' ');

            entries.push(StatusEntry {
                path: parse_status_path(&line[3..]),
                staged_status: parse_status_code(staged_code),
                unstaged_status: parse_status_code(unstaged_code),
            });
        }
        Ok(entries)
    }

    /// Diff output.
    pub fn diff(&self, path: Option<&str>, staged: bool) -> Result<String> {
        if !staged {
            if let Some(path) = path {
                if self.is_untracked_path(path)? {
                    return self.diff_untracked_file(path);
                }
            }
        }

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

    fn is_untracked_path(&self, path: &str) -> Result<bool> {
        let output = Command::new("git")
            .args(["ls-files", "--others", "--exclude-standard", "--", path])
            .current_dir(&self.workdir)
            .output()
            .with_context(|| format!("git ls-files failed for {}", path))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("git ls-files: {}", stderr.trim());
        }

        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .any(|candidate| candidate == path))
    }

    fn diff_untracked_file(&self, path: &str) -> Result<String> {
        let output = Command::new("git")
            .args(["diff", "--no-index", "--", "/dev/null", path])
            .current_dir(&self.workdir)
            .output()
            .with_context(|| format!("git diff --no-index failed for {}", path))?;

        if !output.status.success() && output.status.code() != Some(1) {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("git diff --no-index: {}", stderr.trim());
        }

        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// Commit log.
    pub fn log(&self, limit: usize) -> Result<Vec<LogEntry>> {
        let limit_str = format!("-{}", limit);
        let out = self.git(&["log", &limit_str, "--format=%H%n%s%n%an%n%at"])?;
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
    ///
    /// Branch names are validated against a safe character set before use,
    /// and `--` is inserted to prevent flag injection.
    pub fn checkout(&self, branch: &str) -> Result<()> {
        anyhow::ensure!(is_safe_ref(branch), "invalid branch name: {:?}", branch);
        self.git(&["checkout", "--", branch])?;
        Ok(())
    }

    /// Stage files and commit.
    ///
    /// `--` is inserted before all user-supplied paths to prevent flag injection.
    pub fn commit(&self, message: &str, paths: &[String]) -> Result<String> {
        if paths.is_empty() {
            anyhow::bail!("no paths to commit");
        }
        self.stage(paths)?;
        self.git(&["commit", "-m", message])?;
        let out = self.git(&["rev-parse", "HEAD"])?;
        Ok(out.trim().to_string())
    }

    /// Stage paths in the index.
    pub fn stage(&self, paths: &[String]) -> Result<()> {
        if paths.is_empty() {
            anyhow::bail!("no paths to stage");
        }
        let mut add_args: Vec<&str> = vec!["add", "--"];
        for path in paths {
            add_args.push(path.as_str());
        }
        self.git_ok(&add_args)
    }

    /// Remove paths from the index while keeping working tree contents.
    pub fn unstage(&self, paths: &[String]) -> Result<()> {
        if paths.is_empty() {
            anyhow::bail!("no paths to unstage");
        }

        let mut reset_args: Vec<&str> = vec!["reset", "HEAD", "--"];
        for path in paths {
            reset_args.push(path.as_str());
        }

        match self.git_ok(&reset_args) {
            Ok(()) => Ok(()),
            Err(reset_error) => {
                let head_exists = self.git_ok(&["rev-parse", "--verify", "HEAD"]).is_ok();
                if head_exists {
                    Err(reset_error)
                } else {
                    let mut rm_args: Vec<&str> = vec!["rm", "--cached", "--"];
                    for path in paths {
                        rm_args.push(path.as_str());
                    }
                    self.git_ok(&rm_args)
                }
            }
        }
    }
}

fn parse_status_path(path: &str) -> String {
    path.rsplit_once(" -> ")
        .map(|(_, new_path)| new_path)
        .unwrap_or(path)
        .to_string()
}

fn parse_status_code(code: char) -> Option<FileStatus> {
    match code {
        ' ' => None,
        'M' | 'T' => Some(FileStatus::Modified),
        'A' | 'C' => Some(FileStatus::Added),
        'D' => Some(FileStatus::Deleted),
        'R' => Some(FileStatus::Renamed),
        '?' => Some(FileStatus::Untracked),
        'U' => Some(FileStatus::Conflicted),
        _ => Some(FileStatus::Modified),
    }
}

/// Validate a git ref name (branch, tag) against a safe character set.
///
/// Allows alphanumerics, `/`, `_`, `.`, `-`. Rejects anything that could be
/// interpreted as a flag (leading `-`) or shell metacharacter.
fn is_safe_ref(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('-')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '.' | '-'))
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
        assert_eq!(status[0].staged_status, None);
        assert_eq!(status[0].unstaged_status, Some(FileStatus::Untracked));
        assert_eq!(status[0].path, "new.txt");
    }

    #[test]
    fn status_tracks_staged_and_unstaged_sides() {
        let (dir, repo) = init_repo();
        std::fs::write(dir.path().join("file.txt"), "one\n").unwrap();
        repo.commit("initial commit", &["file.txt".into()]).unwrap();

        std::fs::write(dir.path().join("file.txt"), "two\n").unwrap();
        Command::new("git")
            .args(["add", "file.txt"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::fs::write(dir.path().join("file.txt"), "three\n").unwrap();

        let status = repo.status().unwrap();
        assert_eq!(status.len(), 1);
        assert_eq!(status[0].path, "file.txt");
        assert_eq!(status[0].staged_status, Some(FileStatus::Modified));
        assert_eq!(status[0].unstaged_status, Some(FileStatus::Modified));
    }

    #[test]
    fn commit_and_log() {
        let (dir, repo) = init_repo();
        std::fs::write(dir.path().join("file.txt"), "content").unwrap();
        let hash = repo.commit("initial commit", &["file.txt".into()]).unwrap();
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
    fn diff_untracked_file() {
        let (dir, repo) = init_repo();
        std::fs::write(dir.path().join("new.txt"), "hello\nworld\n").unwrap();

        let diff = repo.diff(Some("new.txt"), false).unwrap();
        assert!(diff.contains("new file mode"));
        assert!(diff.contains("--- /dev/null"));
        assert!(diff.contains("new.txt"));
        assert!(diff.contains("+hello"));
        assert!(diff.contains("+world"));
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
            staged_status: Some(FileStatus::Modified),
            unstaged_status: None,
        };
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: StatusEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, entry);
    }
}
