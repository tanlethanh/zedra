use gpui::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::pending::{SharedPendingSlot, shared_pending_slot, spawn_notify_poll};
use crate::theme;

#[derive(Clone, Debug)]
pub struct FileSelected {
    pub path: String,
}

#[derive(Clone)]
pub struct FileEntry {
    name: String,
    path: String,
    is_dir: bool,
    expanded: bool,
    children: Vec<FileEntry>,
    /// Whether children are currently being loaded from remote
    loading: bool,
    /// Total children on the server (may exceed `children.len()` when paginated)
    children_total: u32,
}

impl FileEntry {
    fn dir(name: &str, path: &str, children: Vec<FileEntry>) -> Self {
        Self {
            name: name.to_string(),
            path: path.to_string(),
            is_dir: true,
            expanded: false,
            children,
            loading: false,
            children_total: 0,
        }
    }

    fn file(name: &str, path: &str) -> Self {
        Self {
            name: name.to_string(),
            path: path.to_string(),
            is_dir: false,
            expanded: false,
            children: Vec::new(),
            loading: false,
            children_total: 0,
        }
    }
}

/// Flat representation of a file entry for rendering.
struct FlatEntry {
    name: String,
    is_dir: bool,
    depth: usize,
    expanded: bool,
    loading: bool,
    /// Index path into the tree for toggling (e.g. [0, 2] = root.children[0].children[2])
    index_path: Vec<usize>,
    /// If true, this row is a "Load N more…" action rather than a real entry.
    is_load_more: bool,
    /// For load-more rows: index path of the parent dir (`[]` = root level).
    load_more_for: Vec<usize>,
}

pub struct FileExplorer {
    entries: Vec<FileEntry>,
    focus_handle: FocusHandle,
    /// Whether entries were loaded from the remote host
    remote_loaded: bool,
    /// Total root entries on the server (may exceed `entries.len()` when paginated)
    root_total: u32,
    /// Pending root entries from async fs/list (carries entries + total)
    pending_entries: SharedPendingSlot<(Vec<FileEntry>, u32)>,
    /// Pending children refresh queue from async fs/list.
    /// We use a queue (not a single slot) because nested observers can trigger
    /// parent/child reloads concurrently; dropping one completion leaves a dir
    /// stuck in `loading=true` and renders trailing "Loading..." rows forever.
    /// `None` payload means load failed and only loading state should be cleared.
    pending_children: Arc<Mutex<Vec<(String, Option<(Vec<FileEntry>, u32)>)>>>,
    /// Pending appended entries from "load more" (index_path + entries; [] = root)
    pending_more: SharedPendingSlot<(Vec<usize>, Vec<FileEntry>)>,
    /// Cached flattened entry list; rebuilt only when entries change.
    flat_entries: Vec<FlatEntry>,
    /// Whether `flat_entries` needs to be rebuilt.
    flat_dirty: bool,
    session_handle: Option<zedra_session::SessionHandle>,
    workdir: String,
    watched_paths: HashSet<String>,
    selected_file_path: Option<String>,
    /// Last observer-triggered refresh instant by directory path.
    last_refresh_at: HashMap<String, Instant>,
    /// Background task polling pending slots.
    _poll_task: Task<()>,
}

const OBSERVER_REFRESH_THROTTLE: Duration = Duration::from_millis(1200);

impl FileExplorer {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let pending_entries: SharedPendingSlot<(Vec<FileEntry>, u32)> = shared_pending_slot();
        let pending_children: Arc<Mutex<Vec<(String, Option<(Vec<FileEntry>, u32)>)>>> =
            Arc::new(Mutex::new(Vec::new()));
        let pending_more: SharedPendingSlot<(Vec<usize>, Vec<FileEntry>)> = shared_pending_slot();

        let poll_entries = pending_entries.clone();
        let poll_children = pending_children.clone();
        let poll_more = pending_more.clone();
        let poll_task = spawn_notify_poll(cx, Duration::from_millis(32), move || {
            poll_entries.has_pending()
                || poll_more.has_pending()
                || poll_children.lock().map(|g| !g.is_empty()).unwrap_or(false)
        });

        let mut explorer = Self {
            entries: Vec::new(),
            focus_handle: cx.focus_handle(),
            remote_loaded: false,
            root_total: 0,
            pending_entries,
            pending_children,
            pending_more,
            flat_entries: Vec::new(),
            flat_dirty: false,
            session_handle: None,
            workdir: String::new(),
            watched_paths: HashSet::new(),
            selected_file_path: None,
            last_refresh_at: HashMap::new(),
            _poll_task: poll_task,
        };

        // If there's an active session, load root entries from remote
        explorer.try_load_remote_root(cx);

        explorer
    }

    /// Set the session handle so the explorer can make RPC calls.
    pub fn set_session_handle(
        &mut self,
        handle: zedra_session::SessionHandle,
        workdir: String,
        cx: &mut Context<Self>,
    ) {
        self.session_handle = Some(handle);
        self.workdir = workdir;
        self.watched_paths.clear();
        self.selected_file_path = None;
        self.last_refresh_at.clear();
        self.try_load_remote_root(cx);
    }

    /// Reset to empty state (e.g. after disconnect)
    pub fn reset_to_demo(&mut self, cx: &mut Context<Self>) {
        self.entries = Vec::new();
        self.root_total = 0;
        self.remote_loaded = false;
        self.watched_paths.clear();
        self.selected_file_path = None;
        self.last_refresh_at.clear();
        self.flat_dirty = true;
        cx.notify();
    }

    /// Reload the root directory from the currently active session.
    /// Used when switching workspaces so the explorer reflects the new session.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.remote_loaded = false;
        self.root_total = 0;
        self.watched_paths.clear();
        self.last_refresh_at.clear();
        // Clear any in-flight pending results from the old session.
        // Keeping stale child updates can re-apply loading/data into a new tree.
        let _ = self.pending_entries.take();
        self.pending_children.lock().unwrap().clear();
        let _ = self.pending_more.take();
        self.try_load_remote_root(cx);
        cx.notify();
    }

    /// Attempt to load the root directory listing from the active remote session.
    fn try_load_remote_root(&mut self, _cx: &mut Context<Self>) {
        self.request_root_listing(true);
    }

    /// Request root listing. When `show_loading` is false we preserve the
    /// existing tree and only apply if root structure actually changed.
    fn request_root_listing(&mut self, show_loading: bool) {
        let handle = match self.session_handle.as_ref() {
            Some(h) if h.has_client() => h.clone(),
            _ => return,
        };
        self.fs_watch_path(".".to_string());

        if show_loading {
            self.entries = vec![FileEntry {
                name: "Loading...".to_string(),
                path: String::new(),
                is_dir: false,
                expanded: false,
                children: Vec::new(),
                loading: true,
                children_total: 0,
            }];
        }
        self.remote_loaded = true;
        self.note_refreshed(".");

        let pending = self.pending_entries.clone();
        zedra_session::session_runtime().spawn(async move {
            match handle.fs_list(".").await {
                Ok((entries, total, _has_more)) => {
                    let file_entries: Vec<FileEntry> = entries
                        .into_iter()
                        .map(|e| {
                            if e.is_dir {
                                FileEntry::dir(&e.name, &e.path, Vec::new())
                            } else {
                                FileEntry::file(&e.name, &e.path)
                            }
                        })
                        .collect();

                    pending.set((file_entries, total));
                }
                Err(e) => {
                    tracing::error!("fs/list failed: {}", e);
                    pending.set((
                        vec![FileEntry {
                            name: format!("Error: {}", e),
                            path: String::new(),
                            is_dir: false,
                            expanded: false,
                            children: Vec::new(),
                            loading: false,
                            children_total: 0,
                        }],
                        0,
                    ));
                }
            }
        });
    }

    /// Check for pending entries from async fs/list and apply them
    fn apply_pending_entries(&mut self) {
        if let Some((entries, total)) = self.pending_entries.take() {
            let old_root_sig = Self::root_signature(&self.entries);
            let new_root_sig = Self::root_signature(&entries);
            if old_root_sig == new_root_sig {
                return;
            }
            let cache = self.entry_cache();
            self.entries = entries;
            self.root_total = total;
            self.flat_dirty = true;
            let mut watched = Vec::new();
            for entry in &mut self.entries {
                Self::restore_entry_state(entry, &cache, &mut watched);
            }
            for path in watched {
                self.fs_watch_path(path);
            }
        }
    }

    /// Load children for a directory at the given index path from remote
    fn load_remote_children(&mut self, index_path: &[usize], cx: &mut Context<Self>) {
        let handle = match self.session_handle.as_ref() {
            Some(h) if h.has_client() => h.clone(),
            _ => return,
        };

        // Get the full path of the directory to list
        let dir_path = match self.entry_at_path(index_path) {
            Some(entry) => entry.path.clone(),
            None => return,
        };
        self.fs_watch_path(dir_path.clone());

        // Mark as loading
        if let Some(entry) = self.entry_at_path_mut(index_path) {
            entry.loading = true;
            entry.expanded = true;
        }
        self.note_refreshed(&dir_path);
        cx.notify();

        // Capture a stable directory path for completion routing. Index paths are
        // positional and can become stale after unrelated tree updates.
        let pending = self.pending_children.clone();
        let dir_path_for_pending = dir_path.clone();

        zedra_session::session_runtime().spawn(async move {
            match handle.fs_list(&dir_path).await {
                Ok((entries, total, _has_more)) => {
                    let file_entries: Vec<FileEntry> = entries
                        .into_iter()
                        .map(|e| {
                            if e.is_dir {
                                FileEntry::dir(&e.name, &e.path, Vec::new())
                            } else {
                                FileEntry::file(&e.name, &e.path)
                            }
                        })
                        .collect();

                    // Queue every completion to avoid overwrite races when both
                    // parent and nested directories refresh in the same window.
                    pending
                        .lock()
                        .unwrap()
                        .push((dir_path_for_pending, Some((file_entries, total))));
                }
                Err(e) => {
                    tracing::error!("fs/list for {:?} failed: {}", dir_path, e);
                    // Ensure error paths still clear spinner state for this dir.
                    pending.lock().unwrap().push((dir_path, None));
                }
            }
        });
    }

    /// Check for pending children from async fs/list and apply them
    fn apply_pending_children(&mut self) {
        // Drain all queued updates for this frame so no directory completion is lost.
        let updates = {
            let mut queue = self.pending_children.lock().unwrap();
            if queue.is_empty() {
                return;
            }
            std::mem::take(&mut *queue)
        };

        for (dir_path, children_result) in updates {
            // Resolve by stable path at apply time to avoid stale index-path bugs.
            let Some(path) = self.find_index_path_by_path(&dir_path) else {
                continue;
            };

            match children_result {
                Some((children, total)) => {
                    let cache = self.entry_cache();
                    if let Some(entry) = self.entry_at_path_mut(&path) {
                        entry.children = children;
                        entry.children_total = total;
                        entry.loading = false;
                        self.flat_dirty = true;
                    }
                    if let Some(entry) = self.entry_at_path(&path) {
                        let mut restored = entry.clone();
                        let mut watched = Vec::new();
                        for child in &mut restored.children {
                            Self::restore_entry_state(child, &cache, &mut watched);
                        }
                        if let Some(entry_mut) = self.entry_at_path_mut(&path) {
                            entry_mut.children = restored.children;
                        }
                        for p in watched {
                            self.fs_watch_path(p);
                        }
                    }
                }
                None => {
                    // Failed refresh: clear loading so the parent does not stay
                    // in "...Loading..." state after nested file churn.
                    if let Some(entry) = self.entry_at_path_mut(&path) {
                        entry.loading = false;
                        self.flat_dirty = true;
                    }
                }
            }
        }
    }

    /// Load the next page of entries for a directory or the root.
    /// `load_more_path` is `[]` for root, or the index path of a dir entry.
    fn load_more_entries(&mut self, load_more_path: Vec<usize>, cx: &mut Context<Self>) {
        let handle = match self.session_handle.as_ref() {
            Some(h) if h.has_client() => h.clone(),
            _ => return,
        };

        let (dir_path, offset) = if load_more_path.is_empty() {
            (".".to_string(), self.entries.len() as u32)
        } else {
            match self.entry_at_path(&load_more_path) {
                Some(entry) => (entry.path.clone(), entry.children.len() as u32),
                None => return,
            }
        };

        let pending = self.pending_more.clone();
        zedra_session::session_runtime().spawn(async move {
            match handle
                .fs_list_page(&dir_path, offset, zedra_rpc::proto::FS_LIST_DEFAULT_LIMIT)
                .await
            {
                Ok((entries, _total, _has_more)) => {
                    let file_entries: Vec<FileEntry> = entries
                        .into_iter()
                        .map(|e| {
                            if e.is_dir {
                                FileEntry::dir(&e.name, &e.path, Vec::new())
                            } else {
                                FileEntry::file(&e.name, &e.path)
                            }
                        })
                        .collect();
                    pending.set((load_more_path, file_entries));
                }
                Err(e) => {
                    tracing::error!("fs/list load_more for {:?} failed: {}", dir_path, e);
                }
            }
        });
        cx.notify();
    }

    /// Check for pending "load more" entries and append them
    fn apply_pending_more(&mut self) {
        if let Some((path, more)) = self.pending_more.take() {
            if path.is_empty() {
                self.entries.extend(more);
            } else if let Some(entry) = self.entry_at_path_mut(&path) {
                entry.children.extend(more);
                entry.loading = false;
            }
            self.flat_dirty = true;
        }
    }

    fn flatten(&self) -> Vec<FlatEntry> {
        let mut flat = Vec::new();
        for (i, entry) in self.entries.iter().enumerate() {
            flatten_entry(entry, 0, &mut vec![i], &mut flat);
        }
        // Root-level load-more row
        if self.entries.len() < self.root_total as usize {
            let remaining = self.root_total as usize - self.entries.len();
            flat.push(FlatEntry {
                name: format!(
                    "Load {} more…",
                    remaining.min(zedra_rpc::proto::FS_LIST_DEFAULT_LIMIT as usize)
                ),
                is_dir: false,
                depth: 0,
                expanded: false,
                loading: false,
                index_path: Vec::new(),
                is_load_more: true,
                load_more_for: Vec::new(),
            });
        }
        flat
    }

    fn toggle_dir(&mut self, index_path: &[usize], cx: &mut Context<Self>) {
        // Check if we need to lazy-load children (using shared borrow first)
        let should_load = self
            .entry_at_path(index_path)
            .is_some_and(|e| e.is_dir && !e.expanded && e.children.is_empty())
            && self.remote_loaded;

        if should_load {
            self.load_remote_children(index_path, cx);
            return;
        }

        let mut collapsed_subtree: Option<FileEntry> = None;
        let mut expanded_path: Option<String> = None;
        if let Some(entry) = self.entry_at_path_mut(index_path) {
            if entry.is_dir {
                let was_expanded = entry.expanded;
                entry.expanded = !entry.expanded;
                if was_expanded {
                    collapsed_subtree = Some(entry.clone());
                } else {
                    expanded_path = Some(entry.path.clone());
                }
                self.flat_dirty = true;
                cx.notify();
            }
        }
        if let Some(path) = expanded_path {
            self.fs_watch_path(path);
        }
        if let Some(tree) = collapsed_subtree {
            let mut paths = Vec::new();
            Self::collect_dir_paths(&tree, &mut paths);
            for path in paths {
                self.fs_unwatch_path(path);
            }
        }
    }

    fn collect_dir_paths(entry: &FileEntry, out: &mut Vec<String>) {
        if !entry.is_dir || entry.path.is_empty() {
            return;
        }
        out.push(entry.path.clone());
        for child in &entry.children {
            Self::collect_dir_paths(child, out);
        }
    }

    fn entry_cache(&self) -> HashMap<String, FileEntry> {
        fn visit(entries: &[FileEntry], out: &mut HashMap<String, FileEntry>) {
            for entry in entries {
                if !entry.path.is_empty() {
                    out.insert(entry.path.clone(), entry.clone());
                }
                visit(&entry.children, out);
            }
        }
        let mut out = HashMap::new();
        visit(&self.entries, &mut out);
        out
    }

    fn restore_entry_state(
        entry: &mut FileEntry,
        cache: &HashMap<String, FileEntry>,
        watched_paths: &mut Vec<String>,
    ) {
        if !entry.is_dir || entry.path.is_empty() {
            return;
        }
        if let Some(prev) = cache.get(&entry.path) {
            entry.expanded = prev.expanded;
            if prev.expanded {
                entry.children = prev.children.clone();
                entry.children_total = prev.children_total;
                entry.loading = false;
                watched_paths.push(entry.path.clone());
            }
        }
        for child in &mut entry.children {
            Self::restore_entry_state(child, cache, watched_paths);
        }
    }

    fn note_refreshed(&mut self, path: &str) {
        self.last_refresh_at
            .insert(path.to_string(), Instant::now());
    }

    fn refresh_throttled(&self, path: &str) -> bool {
        self.last_refresh_at
            .get(path)
            .is_some_and(|t| t.elapsed() < OBSERVER_REFRESH_THROTTLE)
    }

    fn fs_watch_path(&mut self, path: String) {
        let watch_path = self.to_watch_path(&path);
        if !self.watched_paths.insert(watch_path.clone()) {
            return;
        }
        let handle = match self.session_handle.as_ref() {
            Some(h) if h.has_client() => h.clone(),
            _ => return,
        };
        zedra_session::session_runtime().spawn(async move {
            match handle.fs_watch(&watch_path).await {
                Ok(zedra_rpc::proto::FsWatchResult::Ok) => {}
                Ok(other) => tracing::debug!("fs_watch({watch_path}) rejected: {other:?}"),
                Err(e) => tracing::debug!("fs_watch({watch_path}) failed: {e}"),
            }
        });
    }

    fn fs_unwatch_path(&mut self, path: String) {
        let watch_path = self.to_watch_path(&path);
        if !self.watched_paths.remove(&watch_path) {
            return;
        }
        let handle = match self.session_handle.as_ref() {
            Some(h) if h.has_client() => h.clone(),
            _ => return,
        };
        zedra_session::session_runtime().spawn(async move {
            match handle.fs_unwatch(&watch_path).await {
                Ok(zedra_rpc::proto::FsUnwatchResult::Ok) => {}
                Ok(other) => tracing::debug!("fs_unwatch({watch_path}) rejected: {other:?}"),
                Err(e) => tracing::debug!("fs_unwatch({watch_path}) failed: {e}"),
            }
        });
    }

    fn to_watch_path(&self, path: &str) -> String {
        if path == "." {
            return ".".to_string();
        }
        let p = Path::new(path);
        if !p.is_absolute() {
            let rel = path.trim_start_matches("./").trim_start_matches('/');
            return if rel.is_empty() {
                ".".to_string()
            } else {
                rel.to_string()
            };
        }
        if self.workdir.is_empty() {
            return ".".to_string();
        }
        let wd = Path::new(&self.workdir);
        match p.strip_prefix(wd) {
            Ok(rest) => {
                let rel = rest.to_string_lossy().trim_start_matches('/').to_string();
                if rel.is_empty() { ".".to_string() } else { rel }
            }
            Err(_) => ".".to_string(),
        }
    }

    fn event_path_to_entry_path(&self, path: &str) -> String {
        if path == "." {
            return ".".to_string();
        }
        let p = Path::new(path);
        if p.is_absolute() {
            return path.to_string();
        }
        if self.workdir.is_empty() {
            return path.to_string();
        }
        PathBuf::from(&self.workdir)
            .join(path)
            .to_string_lossy()
            .to_string()
    }

    fn find_index_path_by_path(&self, path: &str) -> Option<Vec<usize>> {
        fn visit(
            entries: &[FileEntry],
            target: &str,
            prefix: &mut Vec<usize>,
        ) -> Option<Vec<usize>> {
            for (i, entry) in entries.iter().enumerate() {
                prefix.push(i);
                if entry.path == target {
                    return Some(prefix.clone());
                }
                if let Some(found) = visit(&entry.children, target, prefix) {
                    return Some(found);
                }
                prefix.pop();
            }
            None
        }
        let mut prefix = Vec::new();
        visit(&self.entries, path, &mut prefix)
    }

    fn invalidate_dir(&mut self, path: &str, cx: &mut Context<Self>) {
        if self.refresh_throttled(path) {
            return;
        }
        if path == "." {
            self.request_root_listing(false);
            return;
        }
        let lookup_path = self.event_path_to_entry_path(path);
        let Some(index_path) = self.find_index_path_by_path(&lookup_path) else {
            return;
        };
        // Reload only expanded directories and only when not already loading,
        // so observer bursts do not continuously restart in-flight fetches.
        let should_reload = self
            .entry_at_path(&index_path)
            .is_some_and(|entry| entry.is_dir && entry.expanded && !entry.loading);
        if should_reload {
            self.load_remote_children(&index_path, cx);
        }
    }

    fn entry_at_path(&self, path: &[usize]) -> Option<&FileEntry> {
        if path.is_empty() {
            return None;
        }
        let mut current = self.entries.get(path[0])?;
        for &idx in &path[1..] {
            current = current.children.get(idx)?;
        }
        Some(current)
    }

    fn entry_at_path_mut(&mut self, path: &[usize]) -> Option<&mut FileEntry> {
        if path.is_empty() {
            return None;
        }
        let mut current = self.entries.get_mut(path[0])?;
        for &idx in &path[1..] {
            current = current.children.get_mut(idx)?;
        }
        Some(current)
    }

    fn full_path_for(&self, index_path: &[usize]) -> String {
        // Use the stored path if available
        if let Some(entry) = self.entry_at_path(index_path) {
            if !entry.path.is_empty() {
                return entry.path.clone();
            }
        }
        // Fallback: build path from names
        if index_path.is_empty() {
            return String::new();
        }
        let mut parts = Vec::new();
        let mut entries = &self.entries;
        for &idx in index_path {
            if let Some(entry) = entries.get(idx) {
                parts.push(entry.name.clone());
                entries = &entry.children;
            }
        }
        parts.join("/")
    }

    fn root_signature(entries: &[FileEntry]) -> Vec<(String, bool)> {
        let mut out: Vec<(String, bool)> = entries
            .iter()
            .filter(|e| !e.path.is_empty())
            .map(|e| (e.path.clone(), e.is_dir))
            .collect();
        out.sort();
        out
    }
}

impl EventEmitter<FileSelected> for FileExplorer {}

impl Focusable for FileExplorer {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for FileExplorer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // If a session appeared after construction, load remote root
        if !self.remote_loaded
            && self
                .session_handle
                .as_ref()
                .map_or(false, |h| h.has_client())
        {
            self.try_load_remote_root(cx);
        }

        let changed_paths = self
            .session_handle
            .as_ref()
            .map(|h| h.take_fs_changed())
            .unwrap_or_default();
        for path in changed_paths {
            self.invalidate_dir(&path, cx);
        }

        // Apply any pending async results
        self.apply_pending_entries();
        self.apply_pending_children();
        self.apply_pending_more();

        if self.flat_dirty {
            self.flat_entries = self.flatten();
            self.flat_dirty = false;
        }

        let mut list = div().id("file-list").flex().flex_col();

        // We need to iterate by index to avoid borrow issues with cx.listener closures.
        // Clone the flat entries that the listener closures will capture.
        let flat_len = self.flat_entries.len();
        for flat_idx in 0..flat_len {
            let entry = &self.flat_entries[flat_idx];
            let indent = entry.depth as f32 * 16.0;
            let text_color = if entry.is_dir {
                rgb(0xffffff) // white for dirs
            } else {
                rgb(0xcacaca) // light gray for files
            };
            let index_path = entry.index_path.clone();
            let is_dir = entry.is_dir;
            let name = entry.name.clone();
            let loading = entry.loading;
            let expanded = entry.expanded;
            let is_load_more = entry.is_load_more;
            let load_more_for = entry.load_more_for.clone();
            let index_path_for_path = index_path.clone();
            let row_path = if is_dir {
                None
            } else {
                Some(self.full_path_for(&index_path_for_path))
            };
            let is_selected = row_path
                .as_deref()
                .zip(self.selected_file_path.as_deref())
                .is_some_and(|(a, b)| a == b);

            // Load-more sentinel row
            if is_load_more {
                list = list.child(
                    div()
                        .id(flat_idx)
                        .flex()
                        .flex_row()
                        .items_center()
                        .py(px(4.0))
                        .pl(px(12.0 + indent))
                        .pr(px(8.0))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _event, _window, cx| {
                            this.load_more_entries(load_more_for.clone(), cx);
                        }))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_color(rgb(theme::TEXT_MUTED))
                                .text_size(px(theme::FONT_BODY))
                                .child(name),
                        ),
                );
                continue;
            }

            let icon_element: AnyElement = if loading {
                div()
                    .text_color(rgb(theme::TEXT_MUTED))
                    .text_size(px(theme::FONT_BODY))
                    .child("...")
                    .into_any_element()
            } else if is_dir {
                let (icon_path, icon_size) = if expanded {
                    ("icons/folder-open.svg", px(theme::ICON_FILE_DIR))
                } else {
                    ("icons/folder.svg", px(theme::ICON_FILE))
                };
                svg()
                    .path(icon_path)
                    .size(icon_size)
                    .text_color(rgb(theme::TEXT_MUTED))
                    .into_any_element()
            } else {
                svg()
                    .path("icons/file.svg")
                    .size(px(theme::ICON_FILE))
                    .text_color(rgb(theme::TEXT_MUTED))
                    .into_any_element()
            };

            let mut row = div()
                .id(flat_idx)
                .flex()
                .flex_row()
                .items_center()
                .gap(px(5.0))
                .py(px(4.0))
                .pl(px(12.0 + indent))
                .pr(px(8.0))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _event, _window, cx| {
                    if is_dir {
                        this.toggle_dir(&index_path, cx);
                    } else {
                        let path = this.full_path_for(&index_path_for_path);
                        this.selected_file_path = Some(path.clone());
                        cx.emit(FileSelected { path });
                    }
                }))
                .child(div().flex_shrink_0().child(icon_element))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(text_color)
                        .text_size(px(theme::FONT_BODY))
                        .child(name),
                );
            if is_selected {
                row = row.bg(hsla(0.0, 0.0, 1.0, 0.10));
            }
            list = list.child(row);
        }

        div()
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .size_full()
            // File list (transparent bg, inherits from drawer)
            .child(
                div()
                    .id("file-list-container")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(list),
            )
    }
}

fn flatten_entry(entry: &FileEntry, depth: usize, path: &mut Vec<usize>, out: &mut Vec<FlatEntry>) {
    out.push(FlatEntry {
        name: entry.name.clone(),
        is_dir: entry.is_dir,
        depth,
        expanded: entry.expanded,
        loading: entry.loading,
        index_path: path.clone(),
        is_load_more: false,
        load_more_for: Vec::new(),
    });

    if entry.is_dir && entry.expanded {
        for (i, child) in entry.children.iter().enumerate() {
            path.push(i);
            flatten_entry(child, depth + 1, path, out);
            path.pop();
        }
        // Keep existing children visible while refresh is in-flight.
        if entry.loading {
            out.push(FlatEntry {
                name: "Loading...".to_string(),
                is_dir: false,
                depth: depth + 1,
                expanded: false,
                loading: true,
                index_path: Vec::new(),
                is_load_more: false,
                load_more_for: Vec::new(),
            });
        }
        // Load-more row if more children exist on the server
        if entry.children.len() < entry.children_total as usize {
            let remaining = entry.children_total as usize - entry.children.len();
            out.push(FlatEntry {
                name: format!(
                    "Load {} more…",
                    remaining.min(zedra_rpc::proto::FS_LIST_DEFAULT_LIMIT as usize)
                ),
                is_dir: false,
                depth: depth + 1,
                expanded: false,
                loading: false,
                index_path: Vec::new(),
                is_load_more: true,
                load_more_for: path.clone(),
            });
        }
    }
}
