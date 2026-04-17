use gpui::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tracing::*;

use zedra_rpc::proto::HostEvent;
use zedra_session::{Session, SessionHandle, SessionState};

use crate::theme;
use crate::workspace_state::{WorkspaceState, WorkspaceStateEvent};

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
    loading: bool,
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
    /// Cached flattened entry list; rebuilt only when entries change.
    flat_entries: Vec<FlatEntry>,
    /// Whether `flat_entries` needs to be rebuilt.
    flat_dirty: bool,
    workdir: String,
    watched_paths: HashSet<String>,
    selected_file_path: Option<String>,
    last_refresh_at: HashMap<String, Instant>,
    request_epoch: u64,
    /// Keep track all tasks spawned by the file explorer. All dropped when the file explorer is dropped.
    tasks: Vec<Task<()>>,
    #[allow(dead_code)]
    workspace_state: Entity<WorkspaceState>,
    #[allow(dead_code)]
    session_state: Entity<SessionState>,
    session_handle: SessionHandle,
    _subscriptions: Vec<Subscription>,
}

const OBSERVER_REFRESH_THROTTLE: Duration = Duration::from_millis(1200);

impl FileExplorer {
    pub fn new(
        workspace_state: Entity<WorkspaceState>,
        session_state: Entity<SessionState>,
        session: Session,
        session_handle: SessionHandle,
        cx: &mut Context<Self>,
    ) -> Self {
        let workdir = workspace_state.read(cx).workdir.to_string();
        let mut host_event_rx = session.subscribe_host_events();
        let host_event_task = cx.spawn(async move |this, cx| {
            loop {
                match host_event_rx.recv().await {
                    Ok(HostEvent::FsChanged { path }) => {
                        let should_break = this
                            .update(cx, |this, cx| {
                                this.invalidate_dir(&path, cx);
                            })
                            .is_err();
                        if should_break {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!("file explorer host event listener lagged by {}", skipped);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        let mut subscriptions = Vec::new();
        subscriptions.push(cx.subscribe(
            &workspace_state,
            |this, _workspace, event: &WorkspaceStateEvent, cx| {
                if matches!(event, WorkspaceStateEvent::SyncComplete) {
                    this.handle_sync_complete(cx);
                }
            },
        ));

        Self {
            entries: Vec::new(),
            focus_handle: cx.focus_handle(),
            remote_loaded: false,
            root_total: 0,
            workdir,
            flat_entries: Vec::new(),
            flat_dirty: false,
            watched_paths: HashSet::new(),
            selected_file_path: None,
            last_refresh_at: HashMap::new(),
            request_epoch: 0,
            tasks: vec![host_event_task],
            workspace_state,
            session_state,
            session_handle,
            _subscriptions: subscriptions,
        }
    }

    fn handle_sync_complete(&mut self, cx: &mut Context<Self>) {
        self.workdir = self.workspace_state.read(cx).workdir.to_string();
        self.request_epoch = self.request_epoch.wrapping_add(1);
        self.rebind_watches(cx);
        let show_loading = !self.remote_loaded && self.entries.is_empty();
        self.request_root_listing(show_loading, cx);
    }

    /// Request root listing. When `show_loading` is false we preserve the
    /// existing tree and only apply if root structure actually changed.
    fn request_root_listing(&mut self, show_loading: bool, cx: &mut Context<Self>) {
        info!("request root listing in {:?}", self.workdir);
        let epoch = self.request_epoch;
        self.fs_watch_path(".".to_string(), cx);

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
        self.flat_dirty = true;
        cx.notify();

        let handle = self.session_handle.clone();
        cx.spawn(async move |this, cx| {
            let result = match handle.fs_list(".").await {
                Ok((entries, total, _has_more)) => Ok((Self::to_file_entries(entries), total)),
                Err(e) => {
                    error!("fs/list failed: {}", e);
                    Err(vec![FileEntry {
                        name: format!("Error: {}", e),
                        path: String::new(),
                        is_dir: false,
                        expanded: false,
                        children: Vec::new(),
                        loading: false,
                        children_total: 0,
                    }])
                }
            };

            let _ = this.update(cx, |this, cx| {
                if this.request_epoch != epoch {
                    return;
                }
                match result {
                    Ok((entries, total)) => {
                        let old_root_sig = Self::root_signature(&this.entries);
                        let new_root_sig = Self::root_signature(&entries);
                        if old_root_sig == new_root_sig {
                            return;
                        }
                        let cache = this.entry_cache();
                        this.entries = entries;
                        this.root_total = total;
                        this.flat_dirty = true;
                        let mut watched = Vec::new();
                        for entry in &mut this.entries {
                            Self::restore_entry_state(entry, &cache, &mut watched);
                        }
                        for path in watched {
                            this.fs_watch_path(path, cx);
                        }
                    }
                    Err(entries) => {
                        this.entries = entries;
                        this.root_total = 0;
                        this.flat_dirty = true;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn to_file_entries(entries: Vec<zedra_rpc::proto::FsEntry>) -> Vec<FileEntry> {
        entries
            .into_iter()
            .map(|e| {
                if e.is_dir {
                    FileEntry::dir(&e.name, &e.path, Vec::new())
                } else {
                    FileEntry::file(&e.name, &e.path)
                }
            })
            .collect()
    }

    /// Load children for a directory at the given index path from remote
    fn load_remote_children(&mut self, index_path: &[usize], cx: &mut Context<Self>) {
        let epoch = self.request_epoch;

        let dir_path = match self.entry_at_path(index_path) {
            Some(entry) => entry.path.clone(),
            None => return,
        };
        self.fs_watch_path(dir_path.clone(), cx);

        if let Some(entry) = self.entry_at_path_mut(index_path) {
            entry.loading = true;
            entry.expanded = true;
        }
        self.note_refreshed(&dir_path);
        self.flat_dirty = true;
        cx.notify();

        let handle = self.session_handle.clone();
        cx.spawn(async move |this, cx| {
            let result = match handle.fs_list(&dir_path).await {
                Ok((entries, total, _has_more)) => Some((Self::to_file_entries(entries), total)),
                Err(e) => {
                    error!("fs/list for {:?} failed: {}", dir_path, e);
                    None
                }
            };

            let _ = this.update(cx, |this, cx| {
                if this.request_epoch != epoch {
                    return;
                }
                let Some(path) = this.find_index_path_by_path(&dir_path) else {
                    return;
                };

                match result {
                    Some((children, total)) => {
                        let cache = this.entry_cache();
                        if let Some(entry) = this.entry_at_path_mut(&path) {
                            entry.children = children;
                            entry.children_total = total;
                            entry.loading = false;
                            this.flat_dirty = true;
                        }
                        if let Some(entry) = this.entry_at_path(&path) {
                            let mut restored = entry.clone();
                            let mut watched = Vec::new();
                            for child in &mut restored.children {
                                Self::restore_entry_state(child, &cache, &mut watched);
                            }
                            if let Some(entry_mut) = this.entry_at_path_mut(&path) {
                                entry_mut.children = restored.children;
                            }
                            for p in watched {
                                this.fs_watch_path(p, cx);
                            }
                        }
                    }
                    None => {
                        if let Some(entry) = this.entry_at_path_mut(&path) {
                            entry.loading = false;
                            this.flat_dirty = true;
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Load the next page of entries for a directory or the root.
    /// `load_more_path` is `[]` for root, or the index path of a dir entry.
    fn load_more_entries(&mut self, load_more_path: Vec<usize>, cx: &mut Context<Self>) {
        let epoch = self.request_epoch;

        let (dir_path, offset) = if load_more_path.is_empty() {
            (".".to_string(), self.entries.len() as u32)
        } else {
            match self.entry_at_path(&load_more_path) {
                Some(entry) => (entry.path.clone(), entry.children.len() as u32),
                None => return,
            }
        };

        let handle = self.session_handle.clone();
        cx.spawn(async move |this, cx| {
            let result = handle
                .fs_list_page(&dir_path, offset, zedra_rpc::proto::FS_LIST_DEFAULT_LIMIT)
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.request_epoch != epoch {
                    return;
                }
                match result {
                    Ok((entries, _total, _has_more)) => {
                        let more = Self::to_file_entries(entries);
                        if load_more_path.is_empty() {
                            this.entries.extend(more);
                        } else if let Some(entry) = this.entry_at_path_mut(&load_more_path) {
                            entry.children.extend(more);
                            entry.loading = false;
                        }
                        this.flat_dirty = true;
                        cx.notify();
                    }
                    Err(e) => {
                        error!("fs/list load_more for {:?} failed: {}", dir_path, e);
                    }
                }
            });
        })
        .detach();
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
            self.fs_watch_path(path, cx);
        }
        if let Some(tree) = collapsed_subtree {
            let mut paths = Vec::new();
            Self::collect_dir_paths(&tree, &mut paths);
            for path in paths {
                self.fs_unwatch_path(path, cx);
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

    fn fs_watch_path(&mut self, path: String, cx: &mut Context<Self>) {
        let watch_path = self.to_watch_path(&path);
        if !self.watched_paths.insert(watch_path.clone()) {
            return;
        }
        let handle = self.session_handle.clone();
        let task = cx.spawn(
            async move |_this, _cx| match handle.fs_watch(&watch_path).await {
                Ok(zedra_rpc::proto::FsWatchResult::Ok) => {}
                Ok(other) => debug!("fs_watch({watch_path}) rejected: {other:?}"),
                Err(e) => debug!("fs_watch({watch_path}) failed: {e}"),
            },
        );
        self.tasks.push(task);
    }

    fn fs_unwatch_path(&mut self, path: String, cx: &mut Context<Self>) {
        let watch_path = self.to_watch_path(&path);
        if !self.watched_paths.remove(&watch_path) {
            return;
        }

        let handle = self.session_handle.clone();
        cx.spawn(async move |_this, _cx| {
            match handle.fs_unwatch(&watch_path).await {
                Ok(zedra_rpc::proto::FsUnwatchResult::Ok) => {}
                Ok(other) => debug!("fs_unwatch({watch_path}) rejected: {other:?}"),
                Err(e) => debug!("fs_unwatch({watch_path}) failed: {e}"),
            };
        })
        .detach();
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
            self.request_root_listing(false, cx);
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

    fn rebind_watches(&mut self, cx: &mut Context<Self>) {
        let mut desired = HashSet::new();
        desired.insert(".".to_string());
        Self::collect_expanded_watch_paths(&self.entries, &mut desired);
        if let Some(file_path) = self.selected_file_path.clone() {
            let parent = Path::new(&file_path)
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .filter(|p| !p.is_empty())
                .unwrap_or_else(|| ".".to_string());
            desired.insert(parent);
        }

        self.watched_paths.clear();
        for path in desired {
            self.fs_watch_path(path, cx);
        }
    }

    fn collect_expanded_watch_paths(entries: &[FileEntry], out: &mut HashSet<String>) {
        for entry in entries {
            if entry.is_dir && entry.expanded && !entry.path.is_empty() {
                out.insert(entry.path.clone());
                Self::collect_expanded_watch_paths(&entry.children, out);
            }
        }
    }

    fn watch_parent_dir_for_file(&mut self, file_path: &str, cx: &mut Context<Self>) {
        let parent = Path::new(file_path)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| ".".to_string());
        self.fs_watch_path(parent, cx);
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
        if self.flat_dirty {
            self.flat_entries = self.flatten();
            self.flat_dirty = false;
        }

        let mut list = div().id("file-list").flex_1().flex().flex_col();

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
                        this.watch_parent_dir_for_file(&path, cx);
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
            .id("file-list-container")
            .flex_1()
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .child(list)
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
