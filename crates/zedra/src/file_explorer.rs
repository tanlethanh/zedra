use gpui::prelude::FluentBuilder;
use gpui::*;

#[derive(Clone, Debug)]
pub struct FileSelected {
    pub path: String,
}

pub struct FileEntry {
    pub name: String,
    pub is_dir: bool,
    pub expanded: bool,
    pub children: Vec<FileEntry>,
}

impl FileEntry {
    pub fn dir(name: &str, children: Vec<FileEntry>) -> Self {
        Self {
            name: name.to_string(),
            is_dir: true,
            expanded: false,
            children,
        }
    }

    pub fn file(name: &str) -> Self {
        Self {
            name: name.to_string(),
            is_dir: false,
            expanded: false,
            children: Vec::new(),
        }
    }
}

/// Flat representation of a file entry for rendering.
struct FlatEntry {
    name: String,
    is_dir: bool,
    depth: usize,
    expanded: bool,
    /// Index path into the tree for toggling (e.g. [0, 2] = root.children[0].children[2])
    index_path: Vec<usize>,
}

pub struct FileExplorer {
    entries: Vec<FileEntry>,
    focus_handle: FocusHandle,
}

impl FileExplorer {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            entries: demo_entries(),
            focus_handle: cx.focus_handle(),
        }
    }

    /// Create a file explorer with custom entries (for remote filesystem).
    pub fn with_entries(entries: Vec<FileEntry>, cx: &mut Context<Self>) -> Self {
        Self {
            entries,
            focus_handle: cx.focus_handle(),
        }
    }

    /// Replace the file tree (e.g., after fetching from remote).
    pub fn set_entries(&mut self, entries: Vec<FileEntry>, cx: &mut Context<Self>) {
        self.entries = entries;
        cx.notify();
    }

    /// Build a `FileEntry` tree from flat (name, is_dir) pairs — convenience for RPC results.
    pub fn entries_from_flat(items: &[(String, bool)]) -> Vec<FileEntry> {
        items
            .iter()
            .map(|(name, is_dir)| {
                if *is_dir {
                    FileEntry::dir(name, vec![])
                } else {
                    FileEntry::file(name)
                }
            })
            .collect()
    }

    fn flatten(&self) -> Vec<FlatEntry> {
        let mut flat = Vec::new();
        for (i, entry) in self.entries.iter().enumerate() {
            flatten_entry(entry, 0, &mut vec![i], &mut flat);
        }
        flat
    }

    fn toggle_dir(&mut self, index_path: &[usize], cx: &mut Context<Self>) {
        if let Some(entry) = self.entry_at_path_mut(index_path) {
            if entry.is_dir {
                entry.expanded = !entry.expanded;
                cx.notify();
            }
        }
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
        let flat = self.flatten();

        let mut list = div().id("file-list").flex().flex_col().overflow_y_scroll();

        for entry in flat {
            let indent = entry.depth as f32 * 16.0;
            let icon = if entry.is_dir {
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
                                    // Parent (DrawerHost) handles close via event subscription
                                    cx.emit(FileSelected {
                                        path: String::new(),
                                    });
                                }),
                            )
                            .child("✕"),
                    ),
            )
            // File list
            .child(div().id("file-list-container").flex_1().overflow_y_scroll().child(list))
    }
}

fn flatten_entry(entry: &FileEntry, depth: usize, path: &mut Vec<usize>, out: &mut Vec<FlatEntry>) {
    out.push(FlatEntry {
        name: entry.name.clone(),
        is_dir: entry.is_dir,
        depth,
        expanded: entry.expanded,
        index_path: path.clone(),
    });

    if entry.is_dir && entry.expanded {
        for (i, child) in entry.children.iter().enumerate() {
            path.push(i);
            flatten_entry(child, depth + 1, path, out);
            path.pop();
        }
    }
}

/// Demo file tree data (no filesystem access on Android yet).
fn demo_entries() -> Vec<FileEntry> {
    vec![
        FileEntry::dir(
            "src",
            vec![
                FileEntry::dir(
                    "components",
                    vec![
                        FileEntry::file("App.tsx"),
                        FileEntry::file("Header.tsx"),
                        FileEntry::file("Sidebar.tsx"),
                    ],
                ),
                FileEntry::dir(
                    "utils",
                    vec![FileEntry::file("helpers.ts"), FileEntry::file("api.ts")],
                ),
                FileEntry::file("main.ts"),
                FileEntry::file("index.html"),
            ],
        ),
        FileEntry::dir(
            "tests",
            vec![
                FileEntry::file("app.test.ts"),
                FileEntry::file("helpers.test.ts"),
            ],
        ),
        FileEntry::file("Cargo.toml"),
        FileEntry::file("README.md"),
        FileEntry::file(".gitignore"),
    ]
}
