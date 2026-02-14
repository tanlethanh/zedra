use std::sync::{Arc, Mutex};

use gpui::*;

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
    pending_entries: Arc<Mutex<Option<Vec<FileEntry>>>>,
    /// Pending children from async fs/list (per-instance, not global)
    pending_children: Arc<Mutex<Option<(Vec<usize>, Vec<FileEntry>)>>>,
}

impl FileExplorer {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut explorer = Self {
            entries: demo_entries(),
            focus_handle: cx.focus_handle(),
            remote_loaded: false,
            pending_entries: Arc::new(Mutex::new(None)),
            pending_children: Arc::new(Mutex::new(None)),
        };

        // If there's an active session, load root entries from remote
        explorer.try_load_remote_root(cx);

        explorer
    }

    /// Attempt to load the root directory listing from the active remote session
    fn try_load_remote_root(&mut self, cx: &mut Context<Self>) {
        let session = match zedra_session::active_session() {
            Some(s) => s,
            None => return,
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
            match session.fs_list(".").await {
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

                    if let Ok(mut slot) = pending.lock() {
                        *slot = Some(file_entries);
                    }
                    zedra_session::signal_terminal_data(); // trigger re-render
                }
                Err(e) => {
                    log::error!("fs/list failed: {}", e);
                    if let Ok(mut slot) = pending.lock() {
                        *slot = Some(vec![FileEntry {
                            name: format!("Error: {}", e),
                            path: String::new(),
                            is_dir: false,
                            expanded: false,
                            children: Vec::new(),
                            loading: false,
                        }]);
                    }
                    zedra_session::signal_terminal_data();
                }
            }
        });
    }

    /// Check for pending entries from async fs/list and apply them
    fn apply_pending_entries(&mut self) {
        let taken = self
            .pending_entries
            .try_lock()
            .ok()
            .and_then(|mut s| s.take());
        if let Some(entries) = taken {
            self.entries = entries;
        }
    }

    /// Load children for a directory at the given index path from remote
    fn load_remote_children(&mut self, index_path: &[usize], cx: &mut Context<Self>) {
        let session = match zedra_session::active_session() {
            Some(s) => s,
            None => return,
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
            match session.fs_list(&dir_path).await {
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

                    if let Ok(mut slot) = pending.lock() {
                        *slot = Some((path_for_entries, file_entries));
                    }
                    zedra_session::signal_terminal_data();
                }
                Err(e) => {
                    log::error!("fs/list for {:?} failed: {}", dir_path, e);
                }
            }
        });
    }

    /// Check for pending children from async fs/list and apply them
    fn apply_pending_children(&mut self) {
        let taken = self
            .pending_children
            .try_lock()
            .ok()
            .and_then(|mut s| s.take());
        if let Some((path, children)) = taken {
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
        // Apply any pending async results
        self.apply_pending_entries();
        self.apply_pending_children();

        let flat = self.flatten();

        let mut list = div().id("file-list").flex().flex_col().overflow_y_scroll();

        for entry in flat {
            let indent = entry.depth as f32 * 16.0;
            let icon = if entry.loading {
                "  ..."
            } else if entry.is_dir {
                if entry.expanded {
                    "▼ 📁"
                } else {
                    "▶ 📁"
                }
            } else {
                "  📄"
            };
            let text_color = if entry.is_dir {
                rgb(0x61afef) // blue for dirs
            } else {
                rgb(0xabb2bf) // light gray for files
            };
            let index_path = entry.index_path.clone();
            let is_dir = entry.is_dir;
            let name = entry.name.clone();
            let index_path_for_path = index_path.clone();

            list = list.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .py(px(6.0))
                    .pl(px(12.0 + indent))
                    .pr(px(8.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(0x2c313a)))
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
                    .child(
                        div()
                            .text_color(text_color)
                            .text_sm()
                            .child(format!("{} {}", icon, name)),
                    ),
            );
        }

        div()
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x21252b))
            // Header
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .h(px(44.0))
                    .px(px(12.0))
                    .border_b_1()
                    .border_color(rgb(0x3e4451))
                    .child(div().text_color(rgb(0x61afef)).text_sm().child("Files"))
                    .child(
                        div()
                            .px(px(8.0))
                            .py(px(4.0))
                            .rounded(px(4.0))
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(0x2c313a)))
                            .text_color(rgb(0xabb2bf))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|_this, _event, _window, cx| {
                                    cx.emit(FileSelected {
                                        path: String::new(),
                                    });
                                }),
                            )
                            .child("✕"),
                    ),
            )
            // File list
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
