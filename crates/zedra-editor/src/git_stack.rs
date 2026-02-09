//! GitStack - A fullscreen git source control component
//!
//! This component provides a VS Code/Zed-like git interface with:
//! - Left sidebar showing changed files organized by status (swipe-to-open drawer)
//! - Git actions panel with stage/unstage/commit controls
//! - Main content area showing the diff for the selected file
//!
//! Designed to be reusable for any git project.

use std::ops::Range;
use std::sync::{Arc, Mutex};

use gpui::prelude::FluentBuilder;
use gpui::*;
use zedra_gesture::{pan_gesture, GestureState, PanGestureEvent};

use crate::diff_view::{DiffHunk, DiffLine, DiffLineKind, FileDiff};
use crate::highlighter::Highlighter;
use crate::theme::SyntaxTheme;

// Layout constants
const SIDEBAR_WIDTH: f32 = 260.0;
const LINE_HEIGHT: f32 = 20.0;
const GUTTER_WIDTH: f32 = 40.0;
const FONT_SIZE: f32 = 13.0;
const ICON_SIZE: f32 = 14.0;

/// Git file status
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitFileStatus {
    /// File is staged for commit
    Staged,
    /// File has unstaged changes
    Modified,
    /// File is untracked (new)
    Untracked,
    /// File was deleted
    Deleted,
    /// File was renamed
    Renamed,
    /// File has conflicts
    Conflict,
}

impl GitFileStatus {
    fn icon(&self) -> &'static str {
        match self {
            GitFileStatus::Staged => "+",
            GitFileStatus::Modified => "M",
            GitFileStatus::Untracked => "U",
            GitFileStatus::Deleted => "D",
            GitFileStatus::Renamed => "R",
            GitFileStatus::Conflict => "!",
        }
    }

    fn color(&self) -> Hsla {
        match self {
            GitFileStatus::Staged => rgb(0x98c379).into(),    // green
            GitFileStatus::Modified => rgb(0xe5c07b).into(),  // yellow
            GitFileStatus::Untracked => rgb(0x98c379).into(), // green
            GitFileStatus::Deleted => rgb(0xe06c75).into(),   // red
            GitFileStatus::Renamed => rgb(0x61afef).into(),   // blue
            GitFileStatus::Conflict => rgb(0xe06c75).into(),  // red
        }
    }
}

/// A file entry in the git file list
#[derive(Clone, Debug)]
pub struct GitFileEntry {
    pub path: String,
    pub filename: String,
    pub status: GitFileStatus,
    pub insertions: usize,
    pub deletions: usize,
}

impl GitFileEntry {
    pub fn new(path: &str, status: GitFileStatus, insertions: usize, deletions: usize) -> Self {
        let filename = path.rsplit('/').next().unwrap_or(path).to_string();
        Self {
            path: path.to_string(),
            filename,
            status,
            insertions,
            deletions,
        }
    }
}

/// Git repository state for preview/demo
#[derive(Clone, Debug)]
pub struct GitRepoState {
    pub branch: String,
    pub staged_files: Vec<GitFileEntry>,
    pub unstaged_files: Vec<GitFileEntry>,
    pub untracked_files: Vec<GitFileEntry>,
    pub commit_message: String,
}

impl GitRepoState {
    /// Create sample repository state for preview
    pub fn sample() -> Self {
        Self {
            branch: "feat/diff-view".to_string(),
            staged_files: vec![
                GitFileEntry::new("src/lib.rs", GitFileStatus::Staged, 15, 3),
                GitFileEntry::new("src/config.rs", GitFileStatus::Staged, 42, 8),
            ],
            unstaged_files: vec![
                GitFileEntry::new("src/main.rs", GitFileStatus::Modified, 28, 12),
                GitFileEntry::new("src/utils/helpers.rs", GitFileStatus::Modified, 5, 2),
                GitFileEntry::new("README.md", GitFileStatus::Modified, 10, 0),
            ],
            untracked_files: vec![
                GitFileEntry::new("src/new_feature.rs", GitFileStatus::Untracked, 85, 0),
                GitFileEntry::new("tests/integration_test.rs", GitFileStatus::Untracked, 120, 0),
            ],
            commit_message: String::new(),
        }
    }

    pub fn total_staged(&self) -> usize {
        self.staged_files.len()
    }

    pub fn total_unstaged(&self) -> usize {
        self.unstaged_files.len()
    }

    pub fn total_untracked(&self) -> usize {
        self.untracked_files.len()
    }

    pub fn all_files(&self) -> Vec<&GitFileEntry> {
        self.staged_files
            .iter()
            .chain(self.unstaged_files.iter())
            .chain(self.untracked_files.iter())
            .collect()
    }
}

/// Event emitted when a git action is triggered
#[derive(Clone, Debug)]
pub enum GitAction {
    StageFile(String),
    UnstageFile(String),
    StageAll,
    UnstageAll,
    DiscardFile(String),
    DiscardAll,
    Commit(String),
    Refresh,
    Push,
    Pull,
}

/// Drawer state for gesture-based opening/closing
#[derive(Clone)]
struct DrawerState {
    /// Current drawer offset (0 = closed, SIDEBAR_WIDTH = fully open)
    offset: f32,
    /// Whether a drag gesture is in progress
    is_dragging: bool,
    /// Starting offset when drag began
    start_offset: f32,
}

impl Default for DrawerState {
    fn default() -> Self {
        Self {
            offset: SIDEBAR_WIDTH, // Start open
            is_dragging: false,
            start_offset: 0.0,
        }
    }
}

/// Main GitStack component
pub struct GitStack {
    repo_state: GitRepoState,
    selected_file: Option<String>,
    sidebar_visible: bool,
    sidebar_section_expanded: [bool; 3], // [staged, unstaged, untracked]
    commit_message: String,
    diffs: Vec<FileDiff>,
    highlighter: Highlighter,
    theme: SyntaxTheme,
    scroll_handle: UniformListScrollHandle,
    focus_handle: FocusHandle,
    /// Shared drawer state for gesture handling
    drawer_state: Arc<Mutex<DrawerState>>,
}

impl GitStack {
    pub fn new(cx: &mut App) -> Self {
        let repo_state = GitRepoState::sample();
        let diffs = Self::sample_diffs();
        let selected_file = diffs.first().map(|d| d.new_path.clone());

        Self {
            repo_state,
            selected_file,
            sidebar_visible: true,
            sidebar_section_expanded: [true, true, true],
            commit_message: String::new(),
            diffs,
            highlighter: Highlighter::rust(),
            theme: SyntaxTheme::default_dark(),
            scroll_handle: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            drawer_state: Arc::new(Mutex::new(DrawerState::default())),
        }
    }

    /// Load from external git state (for real git integration later)
    pub fn with_repo_state(mut self, state: GitRepoState) -> Self {
        self.repo_state = state;
        self
    }

    /// Generate sample diffs matching the repo state
    fn sample_diffs() -> Vec<FileDiff> {
        vec![
            FileDiff {
                old_path: "src/main.rs".to_string(),
                new_path: "src/main.rs".to_string(),
                hunks: vec![DiffHunk {
                    old_start: 1,
                    old_count: 10,
                    new_start: 1,
                    new_count: 15,
                    lines: vec![
                        DiffLine {
                            kind: DiffLineKind::Unchanged,
                            old_line_num: Some(1),
                            new_line_num: Some(1),
                            content: "use std::io;".to_string(),
                        },
                        DiffLine {
                            kind: DiffLineKind::Added,
                            old_line_num: None,
                            new_line_num: Some(2),
                            content: "use std::fs;".to_string(),
                        },
                        DiffLine {
                            kind: DiffLineKind::Added,
                            old_line_num: None,
                            new_line_num: Some(3),
                            content: "use std::path::PathBuf;".to_string(),
                        },
                        DiffLine {
                            kind: DiffLineKind::Unchanged,
                            old_line_num: Some(2),
                            new_line_num: Some(4),
                            content: "".to_string(),
                        },
                        DiffLine {
                            kind: DiffLineKind::Unchanged,
                            old_line_num: Some(3),
                            new_line_num: Some(5),
                            content: "fn main() {".to_string(),
                        },
                        DiffLine {
                            kind: DiffLineKind::Removed,
                            old_line_num: Some(4),
                            new_line_num: None,
                            content: "    println!(\"Hello\");".to_string(),
                        },
                        DiffLine {
                            kind: DiffLineKind::Added,
                            old_line_num: None,
                            new_line_num: Some(6),
                            content: "    let config = Config::load()?;".to_string(),
                        },
                        DiffLine {
                            kind: DiffLineKind::Added,
                            old_line_num: None,
                            new_line_num: Some(7),
                            content: "    println!(\"Loaded: {:?}\", config);".to_string(),
                        },
                        DiffLine {
                            kind: DiffLineKind::Unchanged,
                            old_line_num: Some(5),
                            new_line_num: Some(8),
                            content: "}".to_string(),
                        },
                    ],
                }],
            },
            FileDiff {
                old_path: "src/lib.rs".to_string(),
                new_path: "src/lib.rs".to_string(),
                hunks: vec![DiffHunk {
                    old_start: 1,
                    old_count: 5,
                    new_start: 1,
                    new_count: 8,
                    lines: vec![
                        DiffLine {
                            kind: DiffLineKind::Added,
                            old_line_num: None,
                            new_line_num: Some(1),
                            content: "//! Library documentation".to_string(),
                        },
                        DiffLine {
                            kind: DiffLineKind::Added,
                            old_line_num: None,
                            new_line_num: Some(2),
                            content: "".to_string(),
                        },
                        DiffLine {
                            kind: DiffLineKind::Unchanged,
                            old_line_num: Some(1),
                            new_line_num: Some(3),
                            content: "mod config;".to_string(),
                        },
                        DiffLine {
                            kind: DiffLineKind::Added,
                            old_line_num: None,
                            new_line_num: Some(4),
                            content: "mod utils;".to_string(),
                        },
                        DiffLine {
                            kind: DiffLineKind::Unchanged,
                            old_line_num: Some(2),
                            new_line_num: Some(5),
                            content: "".to_string(),
                        },
                        DiffLine {
                            kind: DiffLineKind::Unchanged,
                            old_line_num: Some(3),
                            new_line_num: Some(6),
                            content: "pub use config::Config;".to_string(),
                        },
                        DiffLine {
                            kind: DiffLineKind::Added,
                            old_line_num: None,
                            new_line_num: Some(7),
                            content: "pub use utils::*;".to_string(),
                        },
                    ],
                }],
            },
            FileDiff {
                old_path: "src/config.rs".to_string(),
                new_path: "src/config.rs".to_string(),
                hunks: vec![DiffHunk {
                    old_start: 10,
                    old_count: 6,
                    new_start: 10,
                    new_count: 12,
                    lines: vec![
                        DiffLine {
                            kind: DiffLineKind::Unchanged,
                            old_line_num: Some(10),
                            new_line_num: Some(10),
                            content: "impl Config {".to_string(),
                        },
                        DiffLine {
                            kind: DiffLineKind::Removed,
                            old_line_num: Some(11),
                            new_line_num: None,
                            content: "    pub fn new() -> Self {".to_string(),
                        },
                        DiffLine {
                            kind: DiffLineKind::Removed,
                            old_line_num: Some(12),
                            new_line_num: None,
                            content: "        Self::default()".to_string(),
                        },
                        DiffLine {
                            kind: DiffLineKind::Added,
                            old_line_num: None,
                            new_line_num: Some(11),
                            content: "    pub fn load() -> Result<Self, Error> {".to_string(),
                        },
                        DiffLine {
                            kind: DiffLineKind::Added,
                            old_line_num: None,
                            new_line_num: Some(12),
                            content: "        let path = Self::config_path()?;".to_string(),
                        },
                        DiffLine {
                            kind: DiffLineKind::Added,
                            old_line_num: None,
                            new_line_num: Some(13),
                            content: "        let content = fs::read_to_string(&path)?;".to_string(),
                        },
                        DiffLine {
                            kind: DiffLineKind::Added,
                            old_line_num: None,
                            new_line_num: Some(14),
                            content: "        let config: Config = toml::from_str(&content)?;".to_string(),
                        },
                        DiffLine {
                            kind: DiffLineKind::Added,
                            old_line_num: None,
                            new_line_num: Some(15),
                            content: "        Ok(config)".to_string(),
                        },
                        DiffLine {
                            kind: DiffLineKind::Unchanged,
                            old_line_num: Some(13),
                            new_line_num: Some(16),
                            content: "    }".to_string(),
                        },
                        DiffLine {
                            kind: DiffLineKind::Unchanged,
                            old_line_num: Some(14),
                            new_line_num: Some(17),
                            content: "}".to_string(),
                        },
                    ],
                }],
            },
        ]
    }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_visible = !self.sidebar_visible;
        if let Ok(mut state) = self.drawer_state.lock() {
            state.offset = if self.sidebar_visible { SIDEBAR_WIDTH } else { 0.0 };
        }
        cx.notify();
    }

    fn handle_drawer_gesture(&mut self, event: &PanGestureEvent, cx: &mut Context<Self>) {
        match event.state {
            GestureState::Began => {
                if let Ok(mut state) = self.drawer_state.lock() {
                    state.is_dragging = true;
                    state.start_offset = state.offset;
                }
            }
            GestureState::Changed => {
                if let Ok(mut state) = self.drawer_state.lock() {
                    // Horizontal drag: positive translation.x means swipe right (open)
                    let new_offset = (state.start_offset + event.translation.x).clamp(0.0, SIDEBAR_WIDTH);
                    state.offset = new_offset;
                }
                cx.notify();
            }
            GestureState::Ended | GestureState::Cancelled => {
                if let Ok(mut state) = self.drawer_state.lock() {
                    state.is_dragging = false;
                    // Snap to open or closed based on velocity and position
                    let should_open = if event.velocity.x.abs() > 100.0 {
                        // Use velocity direction if fast enough
                        event.velocity.x > 0.0
                    } else {
                        // Otherwise use position threshold
                        state.offset > SIDEBAR_WIDTH / 2.0
                    };
                    state.offset = if should_open { SIDEBAR_WIDTH } else { 0.0 };
                    self.sidebar_visible = should_open;
                }
                cx.notify();
            }
            _ => {}
        }
    }

    fn get_drawer_offset(&self) -> f32 {
        self.drawer_state.lock().map(|s| s.offset).unwrap_or(0.0)
    }

    fn toggle_section(&mut self, section: usize, cx: &mut Context<Self>) {
        if section < 3 {
            self.sidebar_section_expanded[section] = !self.sidebar_section_expanded[section];
            cx.notify();
        }
    }

    fn select_file(&mut self, path: String, cx: &mut Context<Self>) {
        self.selected_file = Some(path);
        cx.notify();
    }

    fn get_selected_diff(&self) -> Option<&FileDiff> {
        let selected = self.selected_file.as_ref()?;
        self.diffs.iter().find(|d| &d.new_path == selected)
    }

    fn total_lines_for_diff(diff: &FileDiff) -> usize {
        let mut count = 1; // file header
        for hunk in &diff.hunks {
            count += 1; // hunk header
            count += hunk.lines.len();
        }
        count
    }

    fn get_diff_line(&self, diff: &FileDiff, index: usize) -> Option<DiffLine> {
        let mut current = 0;

        // File header
        if index == current {
            return Some(DiffLine {
                kind: DiffLineKind::Header,
                old_line_num: None,
                new_line_num: None,
                content: format!("{} -> {}", diff.old_path, diff.new_path),
            });
        }
        current += 1;

        for hunk in &diff.hunks {
            if index == current {
                return Some(DiffLine {
                    kind: DiffLineKind::Header,
                    old_line_num: None,
                    new_line_num: None,
                    content: format!(
                        "@@ -{},{} +{},{} @@",
                        hunk.old_start, hunk.old_count, hunk.new_start, hunk.new_count
                    ),
                });
            }
            current += 1;

            for line in &hunk.lines {
                if index == current {
                    return Some(line.clone());
                }
                current += 1;
            }
        }
        None
    }

    fn line_highlights(&self, content: &str) -> Vec<(Range<usize>, HighlightStyle)> {
        if content.is_empty() {
            return Vec::new();
        }

        self.highlighter.parse(content);
        let raw_highlights = self.highlighter.highlights(content, 0..content.len());
        let mut result: Vec<(Range<usize>, HighlightStyle)> = Vec::new();

        for (span_range, capture_name) in &raw_highlights {
            if let Some(style) = self.theme.get(capture_name) {
                let start = span_range.start.min(content.len());
                let end = span_range.end.min(content.len());
                if start < end {
                    result.push((start..end, style));
                }
            }
        }

        result.sort_by(|a, b| a.0.start.cmp(&b.0.start).then(a.0.len().cmp(&b.0.len())));

        let mut merged: Vec<(Range<usize>, HighlightStyle)> = Vec::new();
        let mut cursor = 0usize;
        for (range, style) in result {
            if range.start >= cursor {
                merged.push((range.clone(), style));
                cursor = range.end;
            } else if range.start < cursor && range.end > cursor {
                merged.push((cursor..range.end, style));
                cursor = range.end;
            }
        }
        merged
    }

    // =========================================================================
    // Render methods
    // =========================================================================

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let branch = self.repo_state.branch.clone();
        let staged_count = self.repo_state.total_staged();
        let unstaged_count = self.repo_state.total_unstaged();
        let untracked_count = self.repo_state.total_untracked();

        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .h(px(44.0))
            .px_3()
            .bg(rgb(0x21252b))
            .border_b_1()
            .border_color(rgb(0x181a1f))
            // Left side: toggle + branch
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    // Sidebar toggle
                    .child(
                        div()
                            .w(px(28.0))
                            .h(px(28.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.0))
                            .cursor_pointer()
                            .text_color(rgb(0xabb2bf))
                            .hover(|s| s.bg(rgb(0x2c313a)))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| this.toggle_sidebar(cx)),
                            )
                            .child(if self.sidebar_visible { "<" } else { ">" }),
                    )
                    // Branch name
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_color(rgb(0x61afef))
                                    .text_sm()
                                    .child("*"),
                            )
                            .child(
                                div()
                                    .text_color(rgb(0xabb2bf))
                                    .text_sm()
                                    .child(branch),
                            ),
                    ),
            )
            // Right side: stats
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_1()
                            .text_sm()
                            .child(div().text_color(rgb(0x98c379)).child(format!("+{}", staged_count)))
                            .child(div().text_color(rgb(0x636d83)).child("staged")),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_1()
                            .text_sm()
                            .child(div().text_color(rgb(0xe5c07b)).child(format!("~{}", unstaged_count)))
                            .child(div().text_color(rgb(0x636d83)).child("modified")),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_1()
                            .text_sm()
                            .child(div().text_color(rgb(0x56b6c2)).child(format!("?{}", untracked_count)))
                            .child(div().text_color(rgb(0x636d83)).child("untracked")),
                    ),
            )
    }

    fn render_file_entry(
        &self,
        file: &GitFileEntry,
        is_selected: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let path = file.path.clone();
        let filename = file.filename.clone();
        let status = file.status;
        let insertions = file.insertions;
        let deletions = file.deletions;

        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .h(px(26.0))
            .px_2()
            .cursor_pointer()
            .rounded(px(3.0))
            .when(is_selected, |s| s.bg(rgb(0x2c313a)))
            .hover(|s| s.bg(rgb(0x2c313a)))
            .on_mouse_down(MouseButton::Left, {
                let path = path.clone();
                cx.listener(move |this, _, _, cx| {
                    this.select_file(path.clone(), cx);
                })
            })
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .overflow_hidden()
                    // Status icon
                    .child(
                        div()
                            .w(px(ICON_SIZE))
                            .text_color(status.color())
                            .text_size(px(11.0))
                            .child(status.icon()),
                    )
                    // Filename
                    .child(
                        div()
                            .text_color(rgb(0xabb2bf))
                            .text_size(px(12.0))
                            .overflow_hidden()
                            .child(filename),
                    ),
            )
            // Stats
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .text_size(px(10.0))
                    .when(insertions > 0, |s| {
                        s.child(div().text_color(rgb(0x98c379)).child(format!("+{}", insertions)))
                    })
                    .when(deletions > 0, |s| {
                        s.child(div().text_color(rgb(0xe06c75)).child(format!("-{}", deletions)))
                    }),
            )
    }

    fn render_section_header(
        &self,
        title: &str,
        count: usize,
        section_idx: usize,
        action_label: Option<&str>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let is_expanded = self.sidebar_section_expanded[section_idx];
        let title = title.to_string();
        let action_label = action_label.map(|s| s.to_string());

        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .h(px(28.0))
            .px_2()
            .cursor_pointer()
            .hover(|s| s.bg(rgb(0x2c313a)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.toggle_section(section_idx, cx);
                }),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    // Expand/collapse icon
                    .child(
                        div()
                            .text_color(rgb(0x636d83))
                            .text_size(px(10.0))
                            .child(if is_expanded { "v" } else { ">" }),
                    )
                    // Title
                    .child(
                        div()
                            .text_color(rgb(0xabb2bf))
                            .text_size(px(11.0))
                            .font_weight(FontWeight::MEDIUM)
                            .child(title),
                    )
                    // Count badge
                    .child(
                        div()
                            .px_1()
                            .rounded(px(8.0))
                            .bg(rgb(0x3e4451))
                            .text_color(rgb(0xabb2bf))
                            .text_size(px(10.0))
                            .child(count.to_string()),
                    ),
            )
            // Action button
            .when_some(action_label, |el, label| {
                el.child(
                    div()
                        .px_2()
                        .py_px()
                        .rounded(px(3.0))
                        .text_color(rgb(0x636d83))
                        .text_size(px(10.0))
                        .hover(|s| s.bg(rgb(0x3e4451)).text_color(rgb(0xabb2bf)))
                        .child(label),
                )
            })
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.selected_file.clone();

        div()
            .flex()
            .flex_col()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .bg(rgb(0x21252b))
            .border_r_1()
            .border_color(rgb(0x181a1f))
            // Commit section
            .child(self.render_commit_section(cx))
            // File sections
            .child(
                div()
                    .flex_1()
                    .overflow_y_scroll()
                    // Staged section
                    .child(self.render_section_header(
                        "STAGED CHANGES",
                        self.repo_state.total_staged(),
                        0,
                        Some("-"),
                        cx,
                    ))
                    .when(self.sidebar_section_expanded[0], |el| {
                        el.children(self.repo_state.staged_files.iter().map(|f| {
                            let is_selected = selected.as_ref() == Some(&f.path);
                            self.render_file_entry(f, is_selected, cx)
                        }))
                    })
                    // Unstaged section
                    .child(self.render_section_header(
                        "CHANGES",
                        self.repo_state.total_unstaged(),
                        1,
                        Some("+"),
                        cx,
                    ))
                    .when(self.sidebar_section_expanded[1], |el| {
                        el.children(self.repo_state.unstaged_files.iter().map(|f| {
                            let is_selected = selected.as_ref() == Some(&f.path);
                            self.render_file_entry(f, is_selected, cx)
                        }))
                    })
                    // Untracked section
                    .child(self.render_section_header(
                        "UNTRACKED",
                        self.repo_state.total_untracked(),
                        2,
                        Some("+"),
                        cx,
                    ))
                    .when(self.sidebar_section_expanded[2], |el| {
                        el.children(self.repo_state.untracked_files.iter().map(|f| {
                            let is_selected = selected.as_ref() == Some(&f.path);
                            self.render_file_entry(f, is_selected, cx)
                        }))
                    }),
            )
    }

    fn render_commit_section(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .p_2()
            .gap_2()
            .border_b_1()
            .border_color(rgb(0x181a1f))
            // Commit message input placeholder
            .child(
                div()
                    .h(px(60.0))
                    .px_2()
                    .py_1()
                    .bg(rgb(0x1e1e1e))
                    .rounded(px(4.0))
                    .border_1()
                    .border_color(rgb(0x3e4451))
                    .text_color(rgb(0x5c6370))
                    .text_size(px(12.0))
                    .child("Commit message..."),
            )
            // Action buttons
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_2()
                    // Commit button
                    .child(
                        div()
                            .flex_1()
                            .h(px(28.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(rgb(0x3e4451))
                            .rounded(px(4.0))
                            .text_color(rgb(0xabb2bf))
                            .text_size(px(12.0))
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(0x4e5561)))
                            .child("Commit"),
                    )
                    // Push button
                    .child(
                        div()
                            .w(px(60.0))
                            .h(px(28.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(rgb(0x61afef))
                            .rounded(px(4.0))
                            .text_color(rgb(0x21252b))
                            .text_size(px(12.0))
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(0x71bfff)))
                            .child("Push"),
                    ),
            )
    }

    fn render_diff_content(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(diff) = self.get_selected_diff() else {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(0x5c6370))
                .child("Select a file to view changes")
                .into_any_element();
        };

        let line_count = Self::total_lines_for_diff(diff);
        let diff_clone = diff.clone();

        // Pre-compute line data
        let line_data: Vec<Option<DiffLine>> = (0..line_count)
            .map(|i| self.get_diff_line(&diff_clone, i))
            .collect();

        let text_style = {
            let mut style = window.text_style();
            style.color = rgb(0xabb2bf).into();
            style.font_size = px(FONT_SIZE).into();
            style
        };

        // Pre-compute highlights
        let highlights: Vec<Vec<(Range<usize>, HighlightStyle)>> = line_data
            .iter()
            .map(|line| {
                line.as_ref()
                    .filter(|l| l.kind != DiffLineKind::Header)
                    .map(|l| self.line_highlights(&l.content))
                    .unwrap_or_default()
            })
            .collect();

        uniform_list("git-diff-lines", line_count, {
            let text_style = text_style.clone();
            move |range: Range<usize>, _window: &mut Window, _cx: &mut App| {
                range
                    .map(|i| {
                        let Some(line) = &line_data[i] else {
                            return div().h(px(LINE_HEIGHT)).into_any_element();
                        };

                        let (bg_color, gutter_text, prefix) = match line.kind {
                            DiffLineKind::Header => (rgb(0x21252b), "".to_string(), ""),
                            DiffLineKind::Added => {
                                let num = line
                                    .new_line_num
                                    .map(|n| format!("{:>3}", n))
                                    .unwrap_or_default();
                                (rgb(0x2d3b2d), num, "+ ")
                            }
                            DiffLineKind::Removed => {
                                let num = line
                                    .old_line_num
                                    .map(|n| format!("{:>3}", n))
                                    .unwrap_or_default();
                                (rgb(0x3b2d2d), num, "- ")
                            }
                            DiffLineKind::Unchanged => {
                                let num = line
                                    .new_line_num
                                    .map(|n| format!("{:>3}", n))
                                    .unwrap_or_default();
                                (rgb(0x282c34), num, "  ")
                            }
                        };

                        let content = &line.content;
                        let display_content = format!("{}{}", prefix, content);

                        if line.kind == DiffLineKind::Header {
                            return div()
                                .flex()
                                .flex_row()
                                .h(px(LINE_HEIGHT + 4.0))
                                .bg(bg_color)
                                .px_2()
                                .items_center()
                                .child(
                                    div()
                                        .text_color(rgb(0x61afef))
                                        .text_size(px(FONT_SIZE))
                                        .child(content.clone()),
                                )
                                .into_any_element();
                        }

                        let line_highlights = &highlights[i];
                        let adjusted_highlights: Vec<(Range<usize>, HighlightStyle)> =
                            line_highlights
                                .iter()
                                .map(|(range, style)| {
                                    let offset = prefix.len();
                                    ((range.start + offset)..(range.end + offset), *style)
                                })
                                .collect();

                        let styled_text = if display_content.is_empty() {
                            StyledText::new(" ").with_default_highlights(&text_style, Vec::new())
                        } else {
                            StyledText::new(display_content.clone())
                                .with_default_highlights(&text_style, adjusted_highlights)
                        };

                        div()
                            .flex()
                            .flex_row()
                            .h(px(LINE_HEIGHT))
                            .bg(bg_color)
                            .child(
                                div()
                                    .w(px(GUTTER_WIDTH))
                                    .h(px(LINE_HEIGHT))
                                    .flex()
                                    .items_center()
                                    .justify_end()
                                    .pr_2()
                                    .text_color(rgb(0x495162))
                                    .text_size(px(FONT_SIZE))
                                    .child(gutter_text),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .h(px(LINE_HEIGHT))
                                    .flex()
                                    .items_center()
                                    .child(styled_text),
                            )
                            .into_any_element()
                    })
                    .collect()
            }
        })
        .track_scroll(&self.scroll_handle)
        .flex_1()
        .into_any_element()
    }

    fn render_empty_state(&self) -> impl IntoElement {
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_4()
            .child(
                div()
                    .text_color(rgb(0x636d83))
                    .text_size(px(48.0))
                    .child("*"),
            )
            .child(
                div()
                    .text_color(rgb(0x5c6370))
                    .text_size(px(14.0))
                    .child("No changes in working tree"),
            )
            .child(
                div()
                    .text_color(rgb(0x636d83))
                    .text_size(px(12.0))
                    .child("Make some changes to see them here"),
            )
    }
}

impl EventEmitter<GitAction> for GitStack {}

impl Focusable for GitStack {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for GitStack {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let has_changes = !self.repo_state.staged_files.is_empty()
            || !self.repo_state.unstaged_files.is_empty()
            || !self.repo_state.untracked_files.is_empty();

        let drawer_offset = self.get_drawer_offset();

        // Main content area wrapped with pan gesture for drawer control
        let main_content = div()
            .flex()
            .flex_row()
            .flex_1()
            .overflow_hidden()
            // Sidebar - always rendered, positioned based on offset
            .child(
                div()
                    .absolute()
                    .left(px(drawer_offset - SIDEBAR_WIDTH))
                    .top_0()
                    .bottom_0()
                    .child(self.render_sidebar(cx)),
            )
            // Backdrop overlay when drawer is partially or fully open
            .when(drawer_offset > 0.0, |el| {
                el.child(
                    div()
                        .absolute()
                        .left(px(drawer_offset))
                        .top_0()
                        .right_0()
                        .bottom_0()
                        .bg(rgba(0x000000, (drawer_offset / SIDEBAR_WIDTH * 0.5) as f32))
                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                            // Tap on backdrop closes the drawer
                            if let Ok(mut state) = this.drawer_state.lock() {
                                state.offset = 0.0;
                            }
                            this.sidebar_visible = false;
                            cx.notify();
                        })),
                )
            })
            // Diff content or empty state - offset by drawer
            .child(
                div()
                    .ml(px(drawer_offset))
                    .flex_1()
                    .flex()
                    .flex_col()
                    .bg(rgb(0x282c34))
                    .when(has_changes, |el| el.child(self.render_diff_content(window, cx)))
                    .when(!has_changes, |el| el.child(self.render_empty_state())),
            );

        // Wrap content with pan gesture for drawer control
        let gesture_content = pan_gesture()
            .min_distance(10.0)
            .on_pan(cx.listener(|this, event: &PanGestureEvent, _, cx| {
                this.handle_drawer_gesture(event, cx);
            }))
            .child(main_content);

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x282c34))
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                match event.keystroke.key.as_str() {
                    "b" if event.keystroke.modifiers.control => this.toggle_sidebar(cx),
                    _ => {}
                }
            }))
            // Header bar
            .child(self.render_header(cx))
            // Main content area with gesture handling
            .child(gesture_content)
    }
}
