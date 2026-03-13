use gpui::*;

use crate::pending::{shared_pending_slot, SharedPendingSlot};
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
}

pub struct FileExplorer {
    entries: Vec<FileEntry>,
    focus_handle: FocusHandle,
    /// Whether entries were loaded from the remote host
    remote_loaded: bool,
    /// Pending root entries from async fs/list (per-instance, not global)
    pending_entries: SharedPendingSlot<Vec<FileEntry>>,
    /// Pending children from async fs/list (per-instance, not global)
    pending_children: SharedPendingSlot<(Vec<usize>, Vec<FileEntry>)>,
    /// Last flattened entry count, for change-only logging
    last_flat_count: usize,
    session_handle: Option<zedra_session::SessionHandle>,
}

impl FileExplorer {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut explorer = Self {
            entries: demo_entries(),
            focus_handle: cx.focus_handle(),
            remote_loaded: false,
            pending_entries: shared_pending_slot(),
            pending_children: shared_pending_slot(),
            last_flat_count: 0,
            session_handle: None,
        };

        // If there's an active session, load root entries from remote
        explorer.try_load_remote_root(cx);

        explorer
    }

    /// Set the session handle so the explorer can make RPC calls.
    pub fn set_session_handle(&mut self, handle: zedra_session::SessionHandle, cx: &mut Context<Self>) {
        self.session_handle = Some(handle);
        self.try_load_remote_root(cx);
    }

    /// Reset to demo data (e.g. after disconnect)
    pub fn reset_to_demo(&mut self, cx: &mut Context<Self>) {
        self.entries = demo_entries();
        self.remote_loaded = false;
        cx.notify();
    }

    /// Reload the root directory from the currently active session.
    /// Used when switching workspaces so the explorer reflects the new session.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.remote_loaded = false;
        // Clear any in-flight pending results from the old session.
        let _ = self.pending_entries.take();
        let _ = self.pending_children.take();
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
        }];
        self.remote_loaded = true;

        let pending = self.pending_entries.clone();
        zedra_session::session_runtime().spawn(async move {
            match handle.fs_list(".").await {
                Ok(entries) => {
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

                    pending.set(file_entries);
                    zedra_session::push_callback(Box::new(|| {}));
                }
                Err(e) => {
                    log::error!("fs/list failed: {}", e);
                    pending.set(vec![FileEntry {
                        name: format!("Error: {}", e),
                        path: String::new(),
                        is_dir: false,
                        expanded: false,
                        children: Vec::new(),
                        loading: false,
                    }]);
                    zedra_session::push_callback(Box::new(|| {}));
                }
            }
        });
    }

    /// Check for pending entries from async fs/list and apply them
    fn apply_pending_entries(&mut self) {
        if let Some(entries) = self.pending_entries.take() {
            self.entries = entries;
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
                Ok(entries) => {
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

                    pending.set((path_for_entries, file_entries));
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
        if let Some((path, children)) = self.pending_children.take() {
            if let Some(entry) = self.entry_at_path_mut(&path) {
                entry.children = children;
                entry.loading = false;
            }
        }
    }

    fn flatten(&self) -> Vec<FlatEntry> {
        let mut flat = Vec::new();
        for (i, entry) in self.entries.iter().enumerate() {
            flatten_entry(entry, 0, &mut vec![i], &mut flat);
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
        if !self.remote_loaded && self.session_handle.as_ref().map_or(false, |h| h.is_connected()) {
            self.try_load_remote_root(cx);
        }

        // Apply any pending async results
        self.apply_pending_entries();
        self.apply_pending_children();

        let flat = self.flatten();

        if flat.len() != self.last_flat_count {
            log::info!(
                "[PERF] file_explorer: {} entries, remote={}",
                flat.len(), self.remote_loaded
            );
            self.last_flat_count = flat.len();
        }

        let mut list = div().id("file-list").flex().flex_col();

        for entry in flat {
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
            let index_path_for_path = index_path.clone();

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
                    .text_color(rgb(theme::TEXT_SECONDARY))
                    .into_any_element()
            } else {
                svg()
                    .path("icons/file.svg")
                    .size(px(theme::ICON_FILE))
                    .text_color(rgb(0x808080))
                    .into_any_element()
            };

            list = list.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(5.0))
                    .py(px(4.0))
                    .pl(px(12.0 + indent))
                    .pr(px(8.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(hsla(0.0, 0.0, 1.0, 0.05)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _event, _window, cx| {
                            if is_dir {
                                this.toggle_dir(&index_path, cx);
                            } else {
                                let path = this.full_path_for(&index_path_for_path);
                                cx.emit(FileSelected { path });
                            }
                        }),
                    )
                    .child(icon_element)
                    .child(
                        div()
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
            });
        } else {
            for (i, child) in entry.children.iter().enumerate() {
                path.push(i);
                flatten_entry(child, depth + 1, path, out);
                path.pop();
            }
        }
    }
}

/// Demo file tree data (used when no remote session is active).
fn demo_entries() -> Vec<FileEntry> {
    vec![
        FileEntry::dir(
            "src",
            "src",
            vec![
                FileEntry::dir(
                    "components",
                    "src/components",
                    vec![
                        FileEntry::file("App.tsx", "src/components/App.tsx"),
                        FileEntry::file("Header.tsx", "src/components/Header.tsx"),
                        FileEntry::file("Sidebar.tsx", "src/components/Sidebar.tsx"),
                    ],
                ),
                FileEntry::dir(
                    "utils",
                    "src/utils",
                    vec![
                        FileEntry::file("helpers.ts", "src/utils/helpers.ts"),
                        FileEntry::file("api.ts", "src/utils/api.ts"),
                    ],
                ),
                FileEntry::file("main.ts", "src/main.ts"),
                FileEntry::file("index.html", "src/index.html"),
            ],
        ),
        FileEntry::dir(
            "tests",
            "tests",
            vec![
                FileEntry::file("app.test.ts", "tests/app.test.ts"),
                FileEntry::file("helpers.test.ts", "tests/helpers.test.ts"),
            ],
        ),
        FileEntry::file("Cargo.toml", "Cargo.toml"),
        FileEntry::file("README.md", "README.md"),
        FileEntry::file(".gitignore", ".gitignore"),
    ]
}
