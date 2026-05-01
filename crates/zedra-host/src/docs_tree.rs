use anyhow::Result;
use std::path::{Component, Path, PathBuf};
use std::time::Instant;
use uuid::Uuid;
use zedra_rpc::proto::{
    FsDocNode, FsDocsTreeError, FsDocsTreeResult, FS_DOCS_TREE_DEFAULT_LIMIT,
    FS_DOCS_TREE_MAX_LIMIT, FS_DOCS_TREE_MAX_OFFSET, FS_DOCS_TREE_MAX_VISITED_ENTRIES,
};

pub const DOCS_TREE_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(10 * 60);

const FALLBACK_COMPONENT_IGNORES: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "node_modules",
    "bower_components",
    "jspm_packages",
    ".pnpm-store",
    ".next",
    ".nuxt",
    ".svelte-kit",
    ".astro",
    ".turbo",
    ".parcel-cache",
    ".vite",
    ".vercel",
    ".netlify",
    ".output",
    "dist",
    "build",
    "out",
    "coverage",
    ".coverage",
    ".cache",
    ".tmp",
    "tmp",
    "temp",
    "logs",
    ".logs",
    "target",
    "cmake-build-debug",
    "cmake-build-release",
    "CMakeFiles",
    "cmakefiles",
    ".cxx",
    ".venv",
    "venv",
    "env",
    ".tox",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".nox",
    "site-packages",
    ".gradle",
    ".idea",
    "DerivedData",
    "deriveddata",
    ".build",
    "Pods",
    "pods",
    ".bundle",
];

const FALLBACK_SEQUENCE_IGNORES: &[&[&str]] = &[
    &["public", "build"],
    &["vendor", "bundle"],
    &["pkg", "mod"],
    &["pkg", "sumdb"],
];

const FALLBACK_ARTIFACT_EXTENSIONS: &[&str] = &[
    "xcframework",
    "framework",
    "app",
    "apk",
    "aab",
    "ipa",
    "dsym",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocsTreeFile {
    pub name: String,
    pub path: String,
    pub size: u64,
}

#[derive(Clone, Debug)]
pub struct DocsTreeSnapshot {
    pub id: String,
    pub root_name: String,
    pub root_path: String,
    pub docs: Vec<DocsTreeFile>,
    pub truncated: bool,
    pub created_at: Instant,
}

#[derive(Clone, Debug)]
pub struct DocsTreeCacheEntry {
    pub root_key: String,
    pub snapshot: DocsTreeSnapshot,
}

pub fn docs_tree_cache_key(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

pub fn docs_tree_limit(limit: u32) -> u32 {
    if limit == 0 {
        FS_DOCS_TREE_DEFAULT_LIMIT
    } else {
        limit.min(FS_DOCS_TREE_MAX_LIMIT)
    }
}

pub fn validate_docs_tree_offset(offset: u32) -> std::result::Result<(), FsDocsTreeError> {
    if offset > FS_DOCS_TREE_MAX_OFFSET {
        return Err(FsDocsTreeError::InvalidRequest(format!(
            "offset exceeds maximum {FS_DOCS_TREE_MAX_OFFSET}"
        )));
    }
    Ok(())
}

pub fn build_snapshot(root: PathBuf) -> Result<DocsTreeSnapshot> {
    let root_name = root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| ".".to_string());
    let root_path = root.to_string_lossy().into_owned();
    let mut docs = Vec::new();
    let mut visited = 0u32;
    let mut truncated = false;
    // Bound scan work and cached docs; file contents are never read.
    let docs_cap = FS_DOCS_TREE_MAX_OFFSET as usize + FS_DOCS_TREE_MAX_LIMIT as usize + 1;

    let mut builder = ignore::WalkBuilder::new(&root);
    builder
        .hidden(false)
        .follow_links(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .parents(true)
        .ignore(true);

    let fallback_root = root.clone();
    // Combine gitignore with built-in fallback ignores so generated trees stay bounded.
    builder.filter_entry(move |entry| !is_fallback_ignored(&fallback_root, entry));

    for entry in builder.build() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                tracing::debug!("docs tree: skipping unreadable entry: {error}");
                continue;
            }
        };

        if entry.path() == root {
            continue;
        }

        visited += 1;
        if visited > FS_DOCS_TREE_MAX_VISITED_ENTRIES {
            truncated = true;
            break;
        }

        let Some(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() || file_type.is_symlink() || !is_markdown_path(entry.path()) {
            continue;
        }

        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(error) => {
                tracing::debug!(
                    "docs tree: metadata failed for {}: {error}",
                    entry.path().display()
                );
                continue;
            }
        };

        docs.push(DocsTreeFile {
            name: entry.file_name().to_string_lossy().into_owned(),
            path: entry.path().to_string_lossy().into_owned(),
            size: metadata.len(),
        });

        if docs.len() >= docs_cap {
            truncated = true;
            break;
        }
    }

    docs.sort_by(|left, right| left.path.cmp(&right.path));

    Ok(DocsTreeSnapshot {
        id: Uuid::new_v4().to_string(),
        root_name,
        root_path,
        docs,
        truncated,
        created_at: Instant::now(),
    })
}

pub fn snapshot_page_result(
    snapshot: &DocsTreeSnapshot,
    offset: u32,
    limit: u32,
) -> FsDocsTreeResult {
    let limit = docs_tree_limit(limit) as usize;
    let offset = offset as usize;
    let end = offset.saturating_add(limit).min(snapshot.docs.len());
    let docs = if offset < snapshot.docs.len() {
        &snapshot.docs[offset..end]
    } else {
        &[]
    };
    // Page reconstruction keeps the cached scan flat while returning recursive RPC data.
    let root = build_page_tree(snapshot, docs);
    let next_offset = end.min(u32::MAX as usize) as u32;
    let has_more = end < snapshot.docs.len() && next_offset <= FS_DOCS_TREE_MAX_OFFSET;

    FsDocsTreeResult {
        root: Some(root),
        snapshot_id: Some(snapshot.id.clone()),
        next_offset,
        has_more,
        truncated: snapshot.truncated,
        error: None,
    }
}

fn build_page_tree(snapshot: &DocsTreeSnapshot, docs: &[DocsTreeFile]) -> FsDocNode {
    let root_path = Path::new(&snapshot.root_path);
    let mut root = FsDocNode {
        name: snapshot.root_name.clone(),
        path: snapshot.root_path.clone(),
        is_dir: true,
        size: 0,
        children: Vec::new(),
    };

    for doc in docs {
        insert_doc_node(&mut root, root_path, doc);
    }
    sort_doc_node_children(&mut root);
    root
}

fn insert_doc_node(root: &mut FsDocNode, root_path: &Path, doc: &DocsTreeFile) {
    let doc_path = Path::new(&doc.path);
    let rel_path = doc_path.strip_prefix(root_path).unwrap_or(doc_path);
    let components = path_components(rel_path);
    if components.is_empty() {
        return;
    }
    insert_components(root, root_path.to_path_buf(), &components, doc);
}

fn insert_components(
    parent: &mut FsDocNode,
    parent_path: PathBuf,
    components: &[String],
    doc: &DocsTreeFile,
) {
    if components.len() == 1 {
        parent.children.push(FsDocNode {
            name: doc.name.clone(),
            path: doc.path.clone(),
            is_dir: false,
            size: doc.size,
            children: Vec::new(),
        });
        return;
    }

    let dir_name = &components[0];
    let dir_path = parent_path.join(dir_name);
    let dir_path_string = dir_path.to_string_lossy().into_owned();
    let index = match parent
        .children
        .iter()
        .position(|child| child.is_dir && child.path == dir_path_string)
    {
        Some(index) => index,
        None => {
            parent.children.push(FsDocNode {
                name: dir_name.clone(),
                path: dir_path_string,
                is_dir: true,
                size: 0,
                children: Vec::new(),
            });
            parent.children.len() - 1
        }
    };
    insert_components(&mut parent.children[index], dir_path, &components[1..], doc);
}

fn sort_doc_node_children(node: &mut FsDocNode) {
    for child in &mut node.children {
        sort_doc_node_children(child);
    }
    node.children.sort_by(|left, right| {
        right
            .is_dir
            .cmp(&left.is_dir)
            .then(left.name.cmp(&right.name))
    });
}

fn is_markdown_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            let extension = extension.to_ascii_lowercase();
            extension == "md" || extension == "markdown"
        })
        .unwrap_or(false)
}

fn is_fallback_ignored(root: &Path, entry: &ignore::DirEntry) -> bool {
    let components = normalized_relative_components(root, entry.path());
    if components.is_empty() {
        return false;
    }

    if entry
        .file_type()
        .is_some_and(|file_type| file_type.is_dir())
        && components
            .iter()
            .any(|component| component.starts_with('.'))
    {
        return true;
    }

    if components
        .iter()
        .any(|component| FALLBACK_COMPONENT_IGNORES.contains(&component.as_str()))
    {
        return true;
    }

    if FALLBACK_SEQUENCE_IGNORES
        .iter()
        .any(|sequence| contains_sequence(&components, sequence))
    {
        return true;
    }

    components.iter().any(|component| {
        Path::new(component)
            .extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| {
                FALLBACK_ARTIFACT_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
            })
            .unwrap_or(false)
    })
}

fn normalized_relative_components(root: &Path, path: &Path) -> Vec<String> {
    let rel = path.strip_prefix(root).unwrap_or(path);
    path_components(rel)
}

fn path_components(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect()
}

fn contains_sequence(components: &[String], sequence: &[&str]) -> bool {
    components.windows(sequence.len()).any(|window| {
        window
            .iter()
            .map(String::as_str)
            .eq(sequence.iter().copied())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(path: &Path, content: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    #[test]
    fn fs_docs_tree_builds_recursive_page_tree() {
        let temp = tempfile::tempdir().unwrap();
        write(&temp.path().join("README.md"), "# root");
        write(&temp.path().join("vendor/zed/docs/guide.md"), "# guide");

        let snapshot = build_snapshot(temp.path().canonicalize().unwrap()).unwrap();
        let result = snapshot_page_result(&snapshot, 0, 50);
        let root = result.root.unwrap();

        assert_eq!(root.children.len(), 2);
        assert!(root.children.iter().any(|child| child.name == "README.md"));
        let vendor = root
            .children
            .iter()
            .find(|child| child.name == "vendor")
            .unwrap();
        let zed = vendor
            .children
            .iter()
            .find(|child| child.name == "zed")
            .unwrap();
        let docs = zed
            .children
            .iter()
            .find(|child| child.name == "docs")
            .unwrap();
        assert_eq!(docs.children[0].name, "guide.md");
    }

    #[test]
    fn fs_docs_tree_uses_gitignore_and_fallback_ignores() {
        let temp = tempfile::tempdir().unwrap();
        write(&temp.path().join(".gitignore"), "ignored/\n");
        write(&temp.path().join("docs/keep.md"), "# keep");
        write(&temp.path().join("ignored/drop.md"), "# drop");
        write(&temp.path().join("node_modules/pkg/drop.md"), "# drop");
        write(&temp.path().join(".git/objects/drop.md"), "# drop");
        write(&temp.path().join(".github/drop.md"), "# drop");
        write(&temp.path().join("docs/.generated/drop.md"), "# drop");

        let snapshot = build_snapshot(temp.path().canonicalize().unwrap()).unwrap();
        let paths = snapshot
            .docs
            .iter()
            .map(|doc| doc.path.as_str())
            .collect::<Vec<_>>();

        assert_eq!(paths.len(), 1);
        assert!(paths[0].ends_with("docs/keep.md"));
    }

    #[test]
    fn fs_docs_tree_detects_markdown_extensions_case_insensitively() {
        let temp = tempfile::tempdir().unwrap();
        write(&temp.path().join("README.MD"), "# root");
        write(&temp.path().join("guide.MARKDOWN"), "# guide");
        write(&temp.path().join("notes.mdx"), "# nope");

        let snapshot = build_snapshot(temp.path().canonicalize().unwrap()).unwrap();

        assert_eq!(snapshot.docs.len(), 2);
        assert!(snapshot.docs.iter().any(|doc| doc.name == "README.MD"));
        assert!(snapshot.docs.iter().any(|doc| doc.name == "guide.MARKDOWN"));
    }

    #[test]
    fn fs_docs_tree_pages_from_cached_flat_list() {
        let snapshot = DocsTreeSnapshot {
            id: "snapshot".to_string(),
            root_name: "repo".to_string(),
            root_path: "/repo".to_string(),
            docs: (0..3)
                .map(|index| DocsTreeFile {
                    name: format!("doc-{index}.md"),
                    path: format!("/repo/docs/doc-{index}.md"),
                    size: index,
                })
                .collect(),
            truncated: false,
            created_at: Instant::now(),
        };

        let first = snapshot_page_result(&snapshot, 0, 2);
        let second = snapshot_page_result(&snapshot, first.next_offset, 2);

        assert!(first.has_more);
        assert_eq!(first.next_offset, 2);
        assert!(!second.has_more);
        let second_root = second.root.unwrap();
        let docs_dir = &second_root.children[0];
        assert_eq!(docs_dir.children[0].name, "doc-2.md");
    }

    #[test]
    fn fs_docs_tree_rejects_large_offsets() {
        assert!(validate_docs_tree_offset(FS_DOCS_TREE_MAX_OFFSET).is_ok());
        assert_eq!(
            validate_docs_tree_offset(FS_DOCS_TREE_MAX_OFFSET + 1).unwrap_err(),
            FsDocsTreeError::InvalidRequest(format!(
                "offset exceeds maximum {FS_DOCS_TREE_MAX_OFFSET}"
            ))
        );
    }

    #[cfg(unix)]
    #[test]
    fn fs_docs_tree_skips_symlink_directories() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        write(&outside.path().join("outside.md"), "# outside");
        symlink(outside.path(), temp.path().join("linked")).unwrap();
        write(&temp.path().join("inside.md"), "# inside");

        let snapshot = build_snapshot(temp.path().canonicalize().unwrap()).unwrap();

        assert_eq!(snapshot.docs.len(), 1);
        assert_eq!(snapshot.docs[0].name, "inside.md");
    }
}
