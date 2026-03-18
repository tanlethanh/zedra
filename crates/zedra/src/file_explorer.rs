use gpui::*;

use crate::pending::{SharedPendingSlot, shared_pending_slot};
use crate::theme;

#[derive(Clone, Debug)]
pub struct FileSelected {
    pub path: String,
}

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
    /// Pending children from async fs/list (index_path + entries + total)
    pending_children: SharedPendingSlot<(Vec<usize>, Vec<FileEntry>, u32)>,
    /// Pending appended entries from "load more" (index_path + entries; [] = root)
    pending_more: SharedPendingSlot<(Vec<usize>, Vec<FileEntry>)>,
    /// Cached flattened entry list; rebuilt only when entries change.
    flat_entries: Vec<FlatEntry>,
    /// Whether `flat_entries` needs to be rebuilt.
    flat_dirty: bool,
    session_handle: Option<zedra_session::SessionHandle>,
}

impl FileExplorer {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut explorer = Self {
            entries: Vec::new(),
            focus_handle: cx.focus_handle(),
            remote_loaded: false,
            root_total: 0,
            pending_entries: shared_pending_slot(),
            pending_children: shared_pending_slot(),
            pending_more: shared_pending_slot(),
            flat_entries: Vec::new(),
            flat_dirty: false,
            session_handle: None,
        };

        // If there's an active session, load root entries from remote
        explorer.try_load_remote_root(cx);

        explorer
    }

    /// Set the session handle so the explorer can make RPC calls.
    pub fn set_session_handle(
        &mut self,
        handle: zedra_session::SessionHandle,
        cx: &mut Context<Self>,
    ) {
        self.session_handle = Some(handle);
        self.try_load_remote_root(cx);
    }

    /// Reset to empty state (e.g. after disconnect)
    pub fn reset_to_demo(&mut self, cx: &mut Context<Self>) {
        self.entries = Vec::new();
        self.root_total = 0;
        self.remote_loaded = false;
        self.flat_dirty = true;
        cx.notify();
    }

    /// Reload the root directory from the currently active session.
    /// Used when switching workspaces so the explorer reflects the new session.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.remote_loaded = false;
        self.root_total = 0;
        // Clear any in-flight pending results from the old session.
        let _ = self.pending_entries.take();
        let _ = self.pending_children.take();
        let _ = self.pending_more.take();
        self.try_load_remote_root(cx);
        cx.notify();
    }

    /// Attempt to load the root directory listing from the active remote session
    fn try_load_remote_root(&mut self, _cx: &mut Context<Self>) {
        let handle = match self.session_handle.as_ref() {
            Some(h) if h.is_connected() => h.clone(),
            _ => return,
        };

        self.entries = vec![FileEntry {
            name: "Loading...".to_string(),
            path: String::new(),
            is_dir: false,
            expanded: false,
            children: Vec::new(),
            loading: true,
            children_total: 0,
        }];
        self.remote_loaded = true;

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
                    zedra_session::push_callback(Box::new(|| {}));
                }
                Err(e) => {
                    log::error!("fs/list failed: {}", e);
                    pending.set((vec![FileEntry {
                        name: format!("Error: {}", e),
                        path: String::new(),
                        is_dir: false,
                        expanded: false,
                        children: Vec::new(),
                        loading: false,
                        children_total: 0,
                    }], 0));
                    zedra_session::push_callback(Box::new(|| {}));
                }
            }
        });
    }

    /// Check for pending entries from async fs/list and apply them
    fn apply_pending_entries(&mut self) {
        if let Some((entries, total)) = self.pending_entries.take() {
            self.entries = entries;
            self.root_total = total;
            self.flat_dirty = true;
        }
    }

    /// Load children for a directory at the given index path from remote
    fn load_remote_children(&mut self, index_path: &[usize], cx: &mut Context<Self>) {
        let handle = match self.session_handle.as_ref() {
            Some(h) if h.is_connected() => h.clone(),
            _ => return,
        };

        // Get the full path of the directory to list
        let dir_path = match self.entry_at_path(index_path) {
            Some(entry) => entry.path.clone(),
            None => return,
        };

        // Mark as loading
        if let Some(entry) = self.entry_at_path_mut(index_path) {
            entry.loading = true;
            entry.expanded = true;
        }
        cx.notify();

        let path_for_entries = index_path.to_vec();
        let pending = self.pending_children.clone();

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

                    pending.set((path_for_entries, file_entries, total));
                    zedra_session::push_callback(Box::new(|| {}));
                }
                Err(e) => {
                    log::error!("fs/list for {:?} failed: {}", dir_path, e);
                }
            }
        });
    }

    /// Check for pending children from async fs/list and apply them
    fn apply_pending_children(&mut self) {
        if let Some((path, children, total)) = self.pending_children.take() {
            if let Some(entry) = self.entry_at_path_mut(&path) {
                entry.children = children;
                entry.children_total = total;
                entry.loading = false;
                self.flat_dirty = true;
            }
        }
    }

    /// Load the next page of entries for a directory or the root.
    /// `load_more_path` is `[]` for root, or the index path of a dir entry.
    fn load_more_entries(&mut self, load_more_path: Vec<usize>, cx: &mut Context<Self>) {
        let handle = match self.session_handle.as_ref() {
            Some(h) if h.is_connected() => h.clone(),
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
            match handle.fs_list_page(&dir_path, offset, zedra_rpc::proto::FS_LIST_DEFAULT_LIMIT).await {
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
                    zedra_session::push_callback(Box::new(|| {}));
                }
                Err(e) => {
                    log::error!("fs/list load_more for {:?} failed: {}", dir_path, e);
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
                name: format!("Load {} more…", remaining.min(zedra_rpc::proto::FS_LIST_DEFAULT_LIMIT as usize)),
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

        if let Some(entry) = self.entry_at_path_mut(index_path) {
            if entry.is_dir {
                entry.expanded = !entry.expanded;
                self.flat_dirty = true;
                cx.notify();
            }
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
                .map_or(false, |h| h.is_connected())
        {
            self.try_load_remote_root(cx);
        }

        // Apply any pending async results
        self.apply_pending_entries();
        self.apply_pending_children();
        self.apply_pending_more();

        if self.flat_dirty {
            self.flat_entries = self.flatten();
            log::debug!(
                "[PERF] file_explorer: {} entries, remote={}",
                self.flat_entries.len(),
                self.remote_loaded
            );
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
                        .hover(|s| s.bg(hsla(0.0, 0.0, 1.0, 0.05)))
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

            list = list.child(
                div()
                    .id(flat_idx)
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(5.0))
                    .py(px(4.0))
                    .pl(px(12.0 + indent))
                    .pr(px(8.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(hsla(0.0, 0.0, 1.0, 0.05)))
                    .on_click(cx.listener(move |this, _event, _window, cx| {
                        if is_dir {
                            this.toggle_dir(&index_path, cx);
                        } else {
                            let path = this.full_path_for(&index_path_for_path);
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
                    ),
            );
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
        if entry.loading {
            // Show loading indicator as a child
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
        } else {
            for (i, child) in entry.children.iter().enumerate() {
                path.push(i);
                flatten_entry(child, depth + 1, path, out);
                path.pop();
            }
            // Load-more row if more children exist on the server
            if entry.children.len() < entry.children_total as usize {
                let remaining = entry.children_total as usize - entry.children.len();
                out.push(FlatEntry {
                    name: format!("Load {} more…", remaining.min(zedra_rpc::proto::FS_LIST_DEFAULT_LIMIT as usize)),
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
}

