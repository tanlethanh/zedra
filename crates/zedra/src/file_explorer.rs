use gpui::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tracing::*;

use zedra_rpc::proto::HostEvent;
use zedra_session::{Session, SessionHandle, SessionState};

use crate::theme;
use crate::workspace_action;
use crate::workspace_state::WorkspaceState;

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
#[derive(Clone)]
struct FlatEntry {
    name: String,
    path: String,
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
    scroll_handle: UniformListScrollHandle,
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

        Self {
            entries: Vec::new(),
            focus_handle: cx.focus_handle(),
            scroll_handle: UniformListScrollHandle::new(),
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
            _subscriptions: Vec::new(),
        }
    }

    pub fn refresh_after_sync(&mut self, cx: &mut Context<Self>) -> Task<()> {
        self.workdir = self.workspace_state.read(cx).workdir.to_string();
        self.request_epoch = self.request_epoch.wrapping_add(1);
        self.rebind_watches(cx);
        let show_loading = !self.remote_loaded && self.entries.is_empty();
        self.request_root_listing(show_loading, cx)
    }

    /// Request root listing. When `show_loading` is false we preserve the
    /// existing tree and only apply if root structure actually changed.
    fn request_root_listing(&mut self, show_loading: bool, cx: &mut Context<Self>) -> Task<()> {
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
        flatten_entries(&self.entries, self.root_total)
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

        let mut collapsed_paths = Vec::new();
        let mut expanded_path: Option<String> = None;
        if let Some(entry) = self.entry_at_path_mut(index_path) {
            if entry.is_dir {
                let was_expanded = entry.expanded;
                if was_expanded {
                    Self::collect_dir_paths(entry, &mut collapsed_paths);
                }
                entry.expanded = !entry.expanded;
                if !was_expanded {
                    expanded_path = Some(entry.path.clone());
                }
                self.flat_dirty = true;
                cx.notify();
            }
        }
        if let Some(path) = expanded_path {
            self.fs_watch_path(path, cx);
        }
        self.fs_unwatch_paths(collapsed_paths, cx);
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
        let watch_path = normalize_watch_path(&path, &self.workdir);
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

    fn fs_unwatch_paths(&mut self, paths: Vec<String>, cx: &mut Context<Self>) {
        let watch_paths =
            drain_watched_paths_for_unwatch(paths, &self.workdir, &mut self.watched_paths);
        if watch_paths.is_empty() {
            return;
        }

        let handle = self.session_handle.clone();
        cx.spawn(async move |_this, _cx| {
            for watch_path in watch_paths {
                match handle.fs_unwatch(&watch_path).await {
                    Ok(zedra_rpc::proto::FsUnwatchResult::Ok) => {}
                    Ok(other) => debug!("fs_unwatch({watch_path}) rejected: {other:?}"),
                    Err(e) => debug!("fs_unwatch({watch_path}) failed: {e}"),
                };
            }
        })
        .detach();
    }

    fn event_path_to_entry_path(&self, path: &str) -> String {
        event_path_to_entry_path(path, &self.workdir)
    }

    fn find_index_path_by_path(&self, path: &str) -> Option<Vec<usize>> {
        find_index_path_by_path(&self.entries, path)
    }

    fn invalidate_dir(&mut self, path: &str, cx: &mut Context<Self>) {
        if self.refresh_throttled(path) {
            return;
        }
        if path == "." {
            self.request_root_listing(false, cx).detach();
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
}

fn flatten_entries(entries: &[FileEntry], root_total: u32) -> Vec<FlatEntry> {
    let mut flat = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        flatten_entry(entry, 0, &mut vec![i], &mut flat);
    }
    // Root-level load-more row
    if entries.len() < root_total as usize {
        let remaining = root_total as usize - entries.len();
        flat.push(FlatEntry {
            name: format!(
                "Load {} more…",
                remaining.min(zedra_rpc::proto::FS_LIST_DEFAULT_LIMIT as usize)
            ),
            path: String::new(),
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

fn normalize_watch_path(path: &str, workdir: &str) -> String {
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
    if workdir.is_empty() {
        return ".".to_string();
    }
    let wd = Path::new(workdir);
    match p.strip_prefix(wd) {
        Ok(rest) => {
            let rel = rest.to_string_lossy().trim_start_matches('/').to_string();
            if rel.is_empty() { ".".to_string() } else { rel }
        }
        Err(_) => ".".to_string(),
    }
}

fn event_path_to_entry_path(path: &str, workdir: &str) -> String {
    if path == "." {
        return ".".to_string();
    }
    let p = Path::new(path);
    if p.is_absolute() {
        return path.to_string();
    }
    if workdir.is_empty() {
        return path.to_string();
    }
    PathBuf::from(workdir)
        .join(path)
        .to_string_lossy()
        .to_string()
}

fn drain_watched_paths_for_unwatch(
    paths: Vec<String>,
    workdir: &str,
    watched_paths: &mut HashSet<String>,
) -> Vec<String> {
    let mut watch_paths = Vec::new();
    for path in paths {
        let watch_path = normalize_watch_path(&path, workdir);
        if watched_paths.remove(&watch_path) {
            watch_paths.push(watch_path);
        }
    }
    watch_paths
}

fn find_index_path_by_path(entries: &[FileEntry], path: &str) -> Option<Vec<usize>> {
    fn visit(entries: &[FileEntry], target: &str, prefix: &mut Vec<usize>) -> Option<Vec<usize>> {
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
    visit(entries, path, &mut prefix)
}

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

        let flat_len = self.flat_entries.len();
        div()
            .track_focus(&self.focus_handle)
            .id("file-list-container")
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .overflow_hidden()
            .relative()
            .child(
                uniform_list(
                    "file-list",
                    flat_len,
                    cx.processor(|this, range: std::ops::Range<usize>, window, cx| {
                        range
                            .filter_map(|flat_idx| {
                                this.flat_entries.get(flat_idx).cloned().map(|entry| {
                                    this.render_flat_entry(flat_idx, entry, window, cx)
                                })
                            })
                            .collect::<Vec<_>>()
                    }),
                )
                .track_scroll(&self.scroll_handle)
                .size_full()
                .flex_grow(),
            )
    }
}

impl FileExplorer {
    fn render_flat_entry(
        &mut self,
        flat_idx: usize,
        entry: FlatEntry,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entry_depth = entry.depth;
        let indent = entry_depth as f32 * 16.0;
        let text_color = if entry.is_dir {
            rgb(0xffffff)
        } else {
            rgb(0xcacaca)
        };
        let index_path = entry.index_path.clone();
        let is_dir = entry.is_dir;
        let row_path = entry.path.clone();
        let name = entry.name;
        let loading = entry.loading;
        let expanded = entry.expanded;

        if entry.is_load_more {
            let load_more_for = entry.load_more_for;
            return div()
                .id(flat_idx)
                .w_full()
                .flex()
                .flex_row()
                .items_center()
                .h(px(theme::PANEL_ITEM_HEIGHT))
                .pl(px(12.0 + indent))
                .pr(px(8.0))
                .cursor_pointer()
                .on_press(cx.listener(move |this, _event, _window, cx| {
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
                )
                .into_any_element();
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

        let is_selected = !is_dir
            && !row_path.is_empty()
            && self
                .selected_file_path
                .as_deref()
                .is_some_and(|selected| selected == row_path);

        let index_path_for_toggle = index_path.clone();
        let mut row = div()
            .id(flat_idx)
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(5.0))
            .h(px(theme::PANEL_ITEM_HEIGHT))
            .pl(px(12.0 + indent))
            .pr(px(8.0))
            .cursor_pointer()
            .on_press(cx.listener(move |this, _event, window, cx| {
                if is_dir {
                    this.toggle_dir(&index_path_for_toggle, cx);
                } else if !row_path.is_empty() {
                    this.selected_file_path = Some(row_path.clone());
                    window.dispatch_action(
                        workspace_action::OpenFile {
                            path: row_path.clone(),
                        }
                        .boxed_clone(),
                        cx,
                    );
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
        row.into_any_element()
    }
}

fn flatten_entry(entry: &FileEntry, depth: usize, path: &mut Vec<usize>, out: &mut Vec<FlatEntry>) {
    out.push(FlatEntry {
        name: entry.name.clone(),
        path: entry.path.clone(),
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
                path: String::new(),
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
                path: String::new(),
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

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use super::{
        FileEntry, FileExplorer, FlatEntry, drain_watched_paths_for_unwatch,
        event_path_to_entry_path, find_index_path_by_path, flatten_entries, normalize_watch_path,
    };

    fn expanded(mut entry: FileEntry) -> FileEntry {
        entry.expanded = true;
        entry
    }

    fn names(entries: &[FlatEntry]) -> Vec<&str> {
        entries.iter().map(|entry| entry.name.as_str()).collect()
    }

    #[test]
    fn flatten_entries_preserves_visible_paths_and_index_paths() {
        let src = expanded(FileEntry::dir(
            "src",
            "/repo/src",
            vec![
                FileEntry::file("lib.rs", "/repo/src/lib.rs"),
                FileEntry::dir(
                    "tests",
                    "/repo/src/tests",
                    vec![FileEntry::file("hidden.rs", "/repo/src/tests/hidden.rs")],
                ),
            ],
        ));
        let entries = vec![src, FileEntry::file("README.md", "/repo/README.md")];

        let flat = flatten_entries(&entries, entries.len() as u32);

        assert_eq!(names(&flat), vec!["src", "lib.rs", "tests", "README.md"]);
        assert_eq!(flat[0].path, "/repo/src");
        assert_eq!(flat[0].index_path, vec![0]);
        assert_eq!(flat[0].depth, 0);
        assert!(flat[0].is_dir);
        assert!(flat[0].expanded);

        assert_eq!(flat[1].path, "/repo/src/lib.rs");
        assert_eq!(flat[1].index_path, vec![0, 0]);
        assert_eq!(flat[1].depth, 1);
        assert!(!flat[1].is_dir);

        assert_eq!(flat[2].path, "/repo/src/tests");
        assert_eq!(flat[2].index_path, vec![0, 1]);
        assert_eq!(flat[2].depth, 1);
        assert!(flat[2].is_dir);
        assert!(!flat[2].expanded);

        assert_eq!(flat[3].path, "/repo/README.md");
        assert_eq!(flat[3].index_path, vec![1]);
    }

    #[test]
    fn flatten_entries_adds_nested_and_root_load_more_rows() {
        let mut src = expanded(FileEntry::dir(
            "src",
            "/repo/src",
            vec![FileEntry::file("lib.rs", "/repo/src/lib.rs")],
        ));
        src.children_total = 3;
        let entries = vec![src];

        let flat = flatten_entries(&entries, 4);

        assert_eq!(
            names(&flat),
            vec!["src", "lib.rs", "Load 2 more…", "Load 3 more…"]
        );

        let nested_more = &flat[2];
        assert!(nested_more.is_load_more);
        assert_eq!(nested_more.path, "");
        assert_eq!(nested_more.index_path, Vec::<usize>::new());
        assert_eq!(nested_more.load_more_for, vec![0]);
        assert_eq!(nested_more.depth, 1);

        let root_more = &flat[3];
        assert!(root_more.is_load_more);
        assert_eq!(root_more.path, "");
        assert_eq!(root_more.index_path, Vec::<usize>::new());
        assert_eq!(root_more.load_more_for, Vec::<usize>::new());
        assert_eq!(root_more.depth, 0);
    }

    #[test]
    fn flatten_entries_keeps_loading_row_non_actionable() {
        let mut src = expanded(FileEntry::dir(
            "src",
            "/repo/src",
            vec![FileEntry::file("lib.rs", "/repo/src/lib.rs")],
        ));
        src.loading = true;
        src.children_total = src.children.len() as u32;

        let flat = flatten_entries(&[src], 1);

        assert_eq!(names(&flat), vec!["src", "lib.rs", "Loading..."]);
        let loading = &flat[2];
        assert!(loading.loading);
        assert!(!loading.is_dir);
        assert!(!loading.is_load_more);
        assert_eq!(loading.path, "");
        assert_eq!(loading.index_path, Vec::<usize>::new());
        assert_eq!(loading.depth, 1);
    }

    #[test]
    fn collect_dir_paths_collects_only_directories_in_loaded_subtree() {
        let root = FileEntry::dir(
            "repo",
            "/repo",
            vec![
                FileEntry::file("README.md", "/repo/README.md"),
                FileEntry::dir(
                    "src",
                    "/repo/src",
                    vec![
                        FileEntry::file("lib.rs", "/repo/src/lib.rs"),
                        FileEntry::dir("nested", "/repo/src/nested", Vec::new()),
                    ],
                ),
                FileEntry::dir("empty-path", "", Vec::new()),
            ],
        );
        let mut paths = Vec::new();

        FileExplorer::collect_dir_paths(&root, &mut paths);

        assert_eq!(paths, vec!["/repo", "/repo/src", "/repo/src/nested"]);
    }

    #[test]
    fn collect_expanded_watch_paths_skips_collapsed_subtrees() {
        let src = FileEntry::dir(
            "src",
            "/repo/src",
            vec![expanded(FileEntry::dir(
                "hidden-expanded",
                "/repo/src/hidden-expanded",
                Vec::new(),
            ))],
        );
        let tests = expanded(FileEntry::dir("tests", "/repo/tests", Vec::new()));
        let entries = vec![expanded(FileEntry::dir(
            "repo",
            "/repo",
            vec![src, tests, FileEntry::file("README.md", "/repo/README.md")],
        ))];
        let mut paths = HashSet::new();

        FileExplorer::collect_expanded_watch_paths(&entries, &mut paths);

        assert_eq!(
            paths,
            HashSet::from(["/repo".to_string(), "/repo/tests".to_string()])
        );
    }

    #[test]
    fn restore_entry_state_reuses_cached_expanded_children_and_watch_paths() {
        let nested = expanded(FileEntry::dir(
            "nested",
            "/repo/src/nested",
            vec![FileEntry::file("mod.rs", "/repo/src/nested/mod.rs")],
        ));
        let mut cached_src = expanded(FileEntry::dir(
            "src",
            "/repo/src",
            vec![
                FileEntry::file("lib.rs", "/repo/src/lib.rs"),
                nested.clone(),
            ],
        ));
        cached_src.children_total = 9;
        cached_src.loading = true;

        let cache = HashMap::from([
            (cached_src.path.clone(), cached_src),
            (nested.path.clone(), nested),
        ]);
        let mut fresh_src = FileEntry::dir("src", "/repo/src", Vec::new());
        let mut watched_paths = Vec::new();

        FileExplorer::restore_entry_state(&mut fresh_src, &cache, &mut watched_paths);

        assert!(fresh_src.expanded);
        assert!(!fresh_src.loading);
        assert_eq!(fresh_src.children_total, 9);
        assert_eq!(
            fresh_src
                .children
                .iter()
                .map(|entry| entry.path.as_str())
                .collect::<Vec<_>>(),
            vec!["/repo/src/lib.rs", "/repo/src/nested"]
        );
        assert_eq!(
            watched_paths,
            vec!["/repo/src".to_string(), "/repo/src/nested".to_string()]
        );
    }

    #[test]
    fn normalize_watch_path_handles_relative_and_absolute_paths() {
        assert_eq!(normalize_watch_path(".", "/repo"), ".");
        assert_eq!(normalize_watch_path("", "/repo"), ".");
        assert_eq!(normalize_watch_path("./src/lib.rs", "/repo"), "src/lib.rs");
        assert_eq!(normalize_watch_path("/repo", "/repo"), ".");
        assert_eq!(normalize_watch_path("/repo/src", "/repo"), "src");
        assert_eq!(normalize_watch_path("/outside/src", "/repo"), ".");
        assert_eq!(normalize_watch_path("/repo/src", ""), ".");
    }

    #[test]
    fn event_path_to_entry_path_converts_host_relative_paths() {
        assert_eq!(event_path_to_entry_path(".", "/repo"), ".");
        assert_eq!(
            event_path_to_entry_path("/repo/src/lib.rs", "/repo"),
            "/repo/src/lib.rs"
        );
        assert_eq!(
            event_path_to_entry_path("src/lib.rs", "/repo"),
            "/repo/src/lib.rs"
        );
        assert_eq!(event_path_to_entry_path("src/lib.rs", ""), "src/lib.rs");
    }

    #[test]
    fn drain_watched_paths_for_unwatch_normalizes_deduplicates_and_mutates_watch_set() {
        let mut watched_paths = HashSet::from([
            "src".to_string(),
            "src/nested".to_string(),
            "keep".to_string(),
        ]);

        let unwatched = drain_watched_paths_for_unwatch(
            vec![
                "/repo/src".to_string(),
                "./src/nested".to_string(),
                "missing".to_string(),
                "/repo/src".to_string(),
            ],
            "/repo",
            &mut watched_paths,
        );

        assert_eq!(unwatched, vec!["src", "src/nested"]);
        assert_eq!(watched_paths, HashSet::from(["keep".to_string()]));
    }

    #[test]
    fn find_index_path_by_path_returns_nested_tree_position() {
        let entries = vec![
            FileEntry::file("README.md", "/repo/README.md"),
            FileEntry::dir(
                "src",
                "/repo/src",
                vec![
                    FileEntry::file("lib.rs", "/repo/src/lib.rs"),
                    FileEntry::dir(
                        "nested",
                        "/repo/src/nested",
                        vec![FileEntry::file("mod.rs", "/repo/src/nested/mod.rs")],
                    ),
                ],
            ),
        ];

        assert_eq!(
            find_index_path_by_path(&entries, "/repo/src/nested/mod.rs"),
            Some(vec![1, 1, 0])
        );
        assert_eq!(find_index_path_by_path(&entries, "/repo/missing"), None);
    }
}
