//! GitSidebar - Scrollable git file list for the drawer
//!
//! Shows staged/unstaged/untracked files with expand/collapse sections,
//! commit controls, and branch info. Emits GitFileSelected when a file is tapped.
//! Also owns the git state types used by the sidebar and app drawer.

use gpui::prelude::FluentBuilder;
use gpui::*;

use crate::theme;

// ── Git state types ─────────────────────────────────────────────────────────

/// Status of a file in the git working tree.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GitFileStatus {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
}

impl GitFileStatus {
    pub fn color(&self) -> Hsla {
        match self {
            Self::Modified => rgb(0xe5c07b).into(),
            Self::Added => rgb(0x98c379).into(),
            Self::Deleted => rgb(0xe06c75).into(),
            Self::Renamed => rgb(0x61afef).into(),
            Self::Untracked => rgb(0x808080).into(),
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Self::Modified => "M",
            Self::Added => "A",
            Self::Deleted => "D",
            Self::Renamed => "R",
            Self::Untracked => "?",
        }
    }

    pub fn from_status_str(s: &str) -> Self {
        match s {
            "added" => Self::Added,
            "deleted" => Self::Deleted,
            "renamed" => Self::Renamed,
            "untracked" => Self::Untracked,
            _ => Self::Modified,
        }
    }
}

/// A single file entry in the git sidebar.
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

/// Repository state shown in the git sidebar.
#[derive(Clone, Debug)]
pub struct GitRepoState {
    pub branch: String,
    pub staged_files: Vec<GitFileEntry>,
    pub unstaged_files: Vec<GitFileEntry>,
    pub untracked_files: Vec<GitFileEntry>,
    pub commit_message: String,
}

impl GitRepoState {
    pub fn sample() -> Self {
        Self {
            branch: "main".to_string(),
            staged_files: vec![GitFileEntry::new("src/lib.rs", GitFileStatus::Modified, 12, 3)],
            unstaged_files: vec![GitFileEntry::new(
                "src/main.rs",
                GitFileStatus::Modified,
                5,
                1,
            )],
            untracked_files: vec![GitFileEntry::new(
                "src/new_file.rs",
                GitFileStatus::Untracked,
                0,
                0,
            )],
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
}

// ── Sidebar view ────────────────────────────────────────────────────────────

const ICON_SIZE: f32 = 14.0;

/// Emitted when a file entry is tapped in the sidebar.
#[derive(Clone, Debug)]
pub struct GitFileSelected {
    pub path: String,
}

impl EventEmitter<GitFileSelected> for GitSidebar {}

pub struct GitSidebar {
    repo_state: GitRepoState,
    section_expanded: [bool; 3], // [staged, unstaged, untracked]
    focus_handle: FocusHandle,
}

impl GitSidebar {
    pub fn new(cx: &mut App) -> Self {
        Self {
            repo_state: GitRepoState::sample(),
            section_expanded: [true, true, true],
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn set_repo_state(&mut self, state: GitRepoState, cx: &mut Context<Self>) {
        self.repo_state = state;
        cx.notify();
    }

    fn toggle_section(&mut self, section: usize, cx: &mut Context<Self>) {
        if section < 3 {
            self.section_expanded[section] = !self.section_expanded[section];
            cx.notify();
        }
    }

    fn render_section_header(
        &self,
        title: &str,
        count: usize,
        section_idx: usize,
        action_label: Option<&str>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let is_expanded = self.section_expanded[section_idx];
        let title = title.to_string();
        let action_label = action_label.map(|s| s.to_string());

        div()
            .id(section_idx)
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .h(px(28.0))
            .px_2()
            .cursor_pointer()
            .hover(|s| s.bg(hsla(0.0, 0.0, 1.0, 0.05)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.toggle_section(section_idx, cx);
            }))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_color(rgb(0x505050))
                            .text_size(px(theme::FONT_DETAIL))
                            .child(if is_expanded { "v" } else { ">" }),
                    )
                    .child(
                        div()
                            .text_color(rgb(0xcacaca))
                            .text_size(px(11.0))
                            .font_weight(FontWeight::MEDIUM)
                            .child(title),
                    )
                    .child(
                        div()
                            .px_1()
                            .rounded(px(8.0))
                            .bg(rgb(0x252525))
                            .text_color(rgb(0xcacaca))
                            .text_size(px(theme::FONT_DETAIL))
                            .child(count.to_string()),
                    ),
            )
            .when_some(action_label, |el, label| {
                el.child(
                    div()
                        .px_2()
                        .py_px()
                        .rounded(px(3.0))
                        .text_color(rgb(0x505050))
                        .text_size(px(theme::FONT_DETAIL))
                        .hover(|s| s.bg(rgb(0x252525)).text_color(rgb(0xcacaca)))
                        .child(label),
                )
            })
    }

    fn render_file_entry(&self, file: &GitFileEntry, cx: &mut Context<Self>) -> impl IntoElement {
        let path = file.path.clone();
        let filename = file.filename.clone();
        let status = file.status;
        let insertions = file.insertions;
        let deletions = file.deletions;

        div()
            .id(SharedString::from(path.clone()))
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .h(px(26.0))
            .px_2()
            .cursor_pointer()
            .rounded(px(3.0))
            .hover(|s| s.bg(hsla(0.0, 0.0, 1.0, 0.05)))
            .on_click({
                let path = path.clone();
                cx.listener(move |_this, _, _, cx| {
                    cx.emit(GitFileSelected { path: path.clone() });
                })
            })
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .overflow_hidden()
                    .child(
                        div()
                            .w(px(ICON_SIZE))
                            .text_color(status.color())
                            .text_size(px(11.0))
                            .child(status.icon()),
                    )
                    .child(
                        div()
                            .text_color(rgb(0xcacaca))
                            .text_size(px(theme::FONT_BODY))
                            .overflow_hidden()
                            .child(filename),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .text_size(px(theme::FONT_DETAIL))
                    .when(insertions > 0, |s| {
                        s.child(
                            div()
                                .text_color(rgb(0x98c379))
                                .child(format!("+{}", insertions)),
                        )
                    })
                    .when(deletions > 0, |s| {
                        s.child(
                            div()
                                .text_color(rgb(0xe06c75))
                                .child(format!("-{}", deletions)),
                        )
                    }),
            )
    }

    fn render_commit_section(&self) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .p_2()
            .gap_2()
            .border_b_1()
            .border_color(rgb(0x1a1a1a))
            .child(
                div()
                    .h(px(60.0))
                    .px_2()
                    .py_1()
                    .bg(rgb(0x131313))
                    .rounded(px(4.0))
                    .border_1()
                    .border_color(rgb(0x252525))
                    .text_color(rgb(0x505050))
                    .text_size(px(theme::FONT_BODY))
                    .child("Commit message..."),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .h(px(28.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(rgb(0x252525))
                            .rounded(px(4.0))
                            .text_color(rgb(0xcacaca))
                            .text_size(px(theme::FONT_BODY))
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(0x303030)))
                            .child("Commit"),
                    )
                    .child(
                        div()
                            .w(px(60.0))
                            .h(px(28.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(rgb(0x61afef))
                            .rounded(px(4.0))
                            .text_color(rgb(0x0e0c0c))
                            .text_size(px(theme::FONT_BODY))
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(0x71bfff)))
                            .child("Push"),
                    ),
            )
    }
}

impl Focusable for GitSidebar {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for GitSidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let branch = self.repo_state.branch.clone();

        // Pre-compute entries
        let staged_entries: Vec<AnyElement> = self
            .repo_state
            .staged_files
            .iter()
            .map(|f| self.render_file_entry(f, cx).into_any_element())
            .collect();
        let unstaged_entries: Vec<AnyElement> = self
            .repo_state
            .unstaged_files
            .iter()
            .map(|f| self.render_file_entry(f, cx).into_any_element())
            .collect();
        let untracked_entries: Vec<AnyElement> = self
            .repo_state
            .untracked_files
            .iter()
            .map(|f| self.render_file_entry(f, cx).into_any_element())
            .collect();

        let commit_section = self.render_commit_section().into_any_element();
        let staged_header = self
            .render_section_header(
                "STAGED CHANGES",
                self.repo_state.total_staged(),
                0,
                Some("-"),
                cx,
            )
            .into_any_element();
        let unstaged_header = self
            .render_section_header(
                "CHANGES",
                self.repo_state.total_unstaged(),
                1,
                Some("+"),
                cx,
            )
            .into_any_element();
        let untracked_header = self
            .render_section_header(
                "UNTRACKED",
                self.repo_state.total_untracked(),
                2,
                Some("+"),
                cx,
            )
            .into_any_element();

        let show_staged = self.section_expanded[0];
        let show_unstaged = self.section_expanded[1];
        let show_untracked = self.section_expanded[2];

        div()
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x0e0c0c))
            // Branch header
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .h(px(36.0))
                    .px_3()
                    .gap_2()
                    .border_b_1()
                    .border_color(rgb(0x1a1a1a))
                    .child(
                        div()
                            .text_color(rgb(0x61afef))
                            .text_size(px(theme::FONT_DETAIL))
                            .child("*"),
                    )
                    .child(
                        div()
                            .text_color(rgb(0xcacaca))
                            .text_size(px(theme::FONT_DETAIL))
                            .child(branch),
                    ),
            )
            // Commit section
            .child(commit_section)
            // File sections (scrollable)
            .child(
                div()
                    .id("git-sidebar-files")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(staged_header)
                    .when(show_staged, |el| el.children(staged_entries))
                    .child(unstaged_header)
                    .when(show_unstaged, |el| el.children(unstaged_entries))
                    .child(untracked_header)
                    .when(show_untracked, |el| el.children(untracked_entries)),
            )
    }
}
