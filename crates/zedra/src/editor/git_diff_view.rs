//! Unified-diff data structures and parser, shared by `combined_diff_view`.

// ── Diff data types ─────────────────────────────────────────────────────────

/// The kind of change a diff line represents.
#[derive(Clone, Debug, PartialEq)]
pub enum DiffLineKind {
    Header,
    Added,
    Removed,
    Unchanged,
}

/// A single line in a diff hunk.
#[derive(Clone, Debug)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub old_line_num: Option<usize>,
    pub new_line_num: Option<usize>,
    pub content: String,
}

/// A contiguous hunk of changes.
#[derive(Clone, Debug)]
pub struct DiffHunk {
    pub old_start: usize,
    pub old_count: usize,
    pub new_start: usize,
    pub new_count: usize,
    pub lines: Vec<DiffLine>,
}

/// A single file's diff (old path → new path with hunks).
#[derive(Clone, Debug)]
pub struct FileDiff {
    pub old_path: String,
    pub new_path: String,
    pub hunks: Vec<DiffHunk>,
}

impl FileDiff {
    pub fn change_counts(&self) -> (usize, usize) {
        let mut added = 0;
        let mut removed = 0;

        for hunk in &self.hunks {
            for line in &hunk.lines {
                match line.kind {
                    DiffLineKind::Added => added += 1,
                    DiffLineKind::Removed => removed += 1,
                    DiffLineKind::Header | DiffLineKind::Unchanged => {}
                }
            }
        }

        (added, removed)
    }

    pub fn display_path(&self) -> String {
        let old_empty = self.old_path.is_empty() || self.old_path == "/dev/null";
        let new_empty = self.new_path.is_empty() || self.new_path == "/dev/null";

        if old_empty {
            return self.new_path.clone();
        }

        if new_empty || self.old_path == self.new_path {
            self.old_path.clone()
        } else {
            format!("{} -> {}", self.old_path, self.new_path)
        }
    }
}

// ── Unified-diff parser ─────────────────────────────────────────────────────

/// Parse a unified diff string into a list of per-file diffs.
pub fn parse_unified_diff(text: &str) -> Vec<FileDiff> {
    let mut diffs: Vec<FileDiff> = Vec::new();
    let mut current_diff: Option<FileDiff> = None;
    let mut current_hunk: Option<DiffHunk> = None;
    let mut old_line: usize = 0;
    let mut new_line: usize = 0;

    for raw_line in text.lines() {
        if raw_line.starts_with("diff --git ") {
            // Per-file preamble in a multi-file diff (`diff --git a/x b/x`,
            // `index ..`, mode changes, ...) — close out the previous file's
            // trailing hunk so these lines don't get swept in as unchanged
            // content, then fall through and ignore them until `--- ` starts
            // the next file.
            if let (Some(diff), Some(hunk)) = (&mut current_diff, current_hunk.take()) {
                diff.hunks.push(hunk);
            }
        } else if let Some(path) = raw_line.strip_prefix("--- ") {
            if let (Some(diff), Some(hunk)) = (&mut current_diff, current_hunk.take()) {
                diff.hunks.push(hunk);
            }
            if let Some(diff) = current_diff.take() {
                diffs.push(diff);
            }
            let path = path.strip_prefix("a/").unwrap_or(path);
            current_diff = Some(FileDiff {
                old_path: path.to_string(),
                new_path: String::new(),
                hunks: Vec::new(),
            });
        } else if let Some(path) = raw_line.strip_prefix("+++ ") {
            let path = path.strip_prefix("b/").unwrap_or(path);
            if let Some(diff) = &mut current_diff {
                diff.new_path = path.to_string();
            }
        } else if raw_line.starts_with("@@ ") {
            if let (Some(diff), Some(hunk)) = (&mut current_diff, current_hunk.take()) {
                diff.hunks.push(hunk);
            }
            let (os, oc, ns, nc) = parse_hunk_header(raw_line);
            old_line = os;
            new_line = ns;
            current_hunk = Some(DiffHunk {
                old_start: os,
                old_count: oc,
                new_start: ns,
                new_count: nc,
                lines: Vec::new(),
            });
        } else if let Some(hunk) = &mut current_hunk {
            if let Some(content) = raw_line.strip_prefix('+') {
                hunk.lines.push(DiffLine {
                    kind: DiffLineKind::Added,
                    old_line_num: None,
                    new_line_num: Some(new_line),
                    content: content.to_string(),
                });
                new_line += 1;
            } else if let Some(content) = raw_line.strip_prefix('-') {
                hunk.lines.push(DiffLine {
                    kind: DiffLineKind::Removed,
                    old_line_num: Some(old_line),
                    new_line_num: None,
                    content: content.to_string(),
                });
                old_line += 1;
            } else {
                let content = raw_line.strip_prefix(' ').unwrap_or(raw_line);
                hunk.lines.push(DiffLine {
                    kind: DiffLineKind::Unchanged,
                    old_line_num: Some(old_line),
                    new_line_num: Some(new_line),
                    content: content.to_string(),
                });
                old_line += 1;
                new_line += 1;
            }
        }
    }

    if let (Some(diff), Some(hunk)) = (&mut current_diff, current_hunk.take()) {
        diff.hunks.push(hunk);
    }
    if let Some(diff) = current_diff.take() {
        diffs.push(diff);
    }

    diffs
}

fn parse_hunk_header(line: &str) -> (usize, usize, usize, usize) {
    let trimmed = line
        .strip_prefix("@@ ")
        .unwrap_or(line)
        .split(" @@")
        .next()
        .unwrap_or("");

    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    let (old_start, old_count) = parse_range(parts.first().unwrap_or(&""));
    let (new_start, new_count) = parse_range(parts.get(1).unwrap_or(&""));
    (old_start, old_count, new_start, new_count)
}

fn parse_range(s: &str) -> (usize, usize) {
    let s = s.trim_start_matches(['-', '+']);
    if let Some((start, count)) = s.split_once(',') {
        (start.parse().unwrap_or(1), count.parse().unwrap_or(0))
    } else {
        (s.parse().unwrap_or(1), 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multi_file_diff_does_not_leak_preamble_into_previous_file_hunk() {
        // Plain `git diff` (no path filter) prefixes each file with
        // `diff --git a/x b/x` + `index ..` *between* files, i.e. right
        // after the previous file's last hunk line — regression test for
        // those leaking in as trailing "unchanged" content.
        let text = "\
diff --git a/src/a.rs b/src/a.rs
index 1111111..2222222 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -1,1 +1,1 @@
-old_a
+new_a
diff --git a/src/b.rs b/src/b.rs
index 3333333..4444444 100644
--- a/src/b.rs
+++ b/src/b.rs
@@ -1,1 +1,1 @@
-old_b
+new_b
";
        let diffs = parse_unified_diff(text);
        assert_eq!(diffs.len(), 2);

        assert_eq!(diffs[0].new_path, "src/a.rs");
        let a_lines: Vec<&str> = diffs[0].hunks[0]
            .lines
            .iter()
            .map(|l| l.content.as_str())
            .collect();
        assert_eq!(a_lines, ["old_a", "new_a"]);

        assert_eq!(diffs[1].new_path, "src/b.rs");
        let b_lines: Vec<&str> = diffs[1].hunks[0]
            .lines
            .iter()
            .map(|l| l.content.as_str())
            .collect();
        assert_eq!(b_lines, ["old_b", "new_b"]);
    }
}
