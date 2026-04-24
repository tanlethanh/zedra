//! GitSidebar - Scrollable git file list for the drawer
//!
//! Shows staged/unstaged/untracked files with expand/collapse sections,
//! commit controls, and branch info. Emits GitFileSelected when a file is tapped.
//! Also owns the git state types used by the sidebar and app drawer.

use gpui::prelude::FluentBuilder;
use gpui::*;

use crate::theme;
use crate::ui::input::Input;
use crate::ui::{InputChanged, InputSubmit};

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
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Modified => "M",
            Self::Added => "A",
            Self::Deleted => "D",
            Self::Renamed => "R",
            Self::Untracked => "U",
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitFileSection {
    Staged,
    Unstaged,
    Untracked,
}

/// A single file entry in the git sidebar.
#[derive(Clone, Debug)]
pub struct GitFileEntry {
    pub path: String,
    pub filename: String,
    pub status: GitFileStatus,
    pub section: GitFileSection,
    pub insertions: usize,
    pub deletions: usize,
}

impl GitFileEntry {
    pub fn new(
        path: &str,
        status: GitFileStatus,
        section: GitFileSection,
        insertions: usize,
        deletions: usize,
    ) -> Self {
        let is_dir = path.ends_with('/');
        let trimmed = path.trim_end_matches('/');
        let name = trimmed.rsplit('/').next().unwrap_or(trimmed);
        let filename = if is_dir {
            format!("{}/", name)
        } else {
            name.to_string()
        };
        Self {
            path: path.to_string(),
            filename,
            status,
            section,
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
}

impl GitRepoState {
    pub fn sample() -> Self {
        Self {
            branch: "main".to_string(),
            staged_files: vec![GitFileEntry::new(
                "src/lib.rs",
                GitFileStatus::Modified,
                GitFileSection::Staged,
                12,
                3,
            )],
            unstaged_files: vec![GitFileEntry::new(
                "src/main.rs",
                GitFileStatus::Modified,
                GitFileSection::Unstaged,
                5,
                1,
            )],
            untracked_files: vec![GitFileEntry::new(
                "src/new_file.rs",
                GitFileStatus::Untracked,
                GitFileSection::Untracked,
                0,
                0,
            )],
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
    pub section: GitFileSection,
}

impl EventEmitter<GitFileSelected> for GitSidebar {}

#[derive(Clone, Debug)]
pub struct GitFileLongPressed {
    pub path: String,
    pub section: GitFileSection,
}

impl EventEmitter<GitFileLongPressed> for GitSidebar {}

#[derive(Clone, Debug)]
pub struct GitCommitRequested {
    pub message: String,
    pub paths: Vec<String>,
}

impl EventEmitter<GitCommitRequested> for GitSidebar {}

pub struct GitSidebar {
    repo_state: GitRepoState,
    section_expanded: [bool; 3], // [staged, unstaged, untracked]
    focus_handle: FocusHandle,
    commit_input: Entity<Input>,
    commit_message: String,
    committing: bool,
    _subscriptions: Vec<Subscription>,
}

impl GitSidebar {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let commit_input = cx.new(|cx| {
            Input::new(cx)
                .compact(true)
                .placeholder("Commit message")
                .trailing_gutter(40.0)
                .multiline(true)
                .max_lines(8)
        });
        let mut subscriptions = Vec::new();

        subscriptions.push(cx.subscribe(
            &commit_input,
            |this: &mut Self, _input, event: &InputChanged, cx| {
                this.commit_message = event.value.clone();
                cx.notify();
            },
        ));
        subscriptions.push(cx.subscribe(
            &commit_input,
            |this: &mut Self, _input, _event: &InputSubmit, cx| {
                this.request_commit(cx);
            },
        ));

        Self {
            repo_state: GitRepoState {
                branch: String::new(),
                staged_files: Vec::new(),
                unstaged_files: Vec::new(),
                untracked_files: Vec::new(),
            },
            section_expanded: [true, true, true],
            focus_handle: cx.focus_handle(),
            commit_input,
            commit_message: String::new(),
            committing: false,
            _subscriptions: subscriptions,
        }
    }

    pub fn branch(&self) -> &str {
        &self.repo_state.branch
    }

    pub fn set_repo_state(&mut self, state: GitRepoState, cx: &mut Context<Self>) {
        self.repo_state = state;
        cx.notify();
    }

    pub fn set_committing(&mut self, committing: bool, cx: &mut Context<Self>) {
        self.committing = committing;
        cx.notify();
    }

    pub fn clear_commit_message(&mut self, cx: &mut Context<Self>) {
        self.commit_message.clear();
        self.commit_input
            .update(cx, |input, _cx| input.set_value(String::new()));
        cx.notify();
    }

    fn toggle_section(&mut self, section: usize, cx: &mut Context<Self>) {
        if section < 3 {
            self.section_expanded[section] = !self.section_expanded[section];
            cx.notify();
        }
    }

    fn staged_paths(&self) -> Vec<String> {
        self.repo_state
            .staged_files
            .iter()
            .map(|file| file.path.clone())
            .collect()
    }

    fn can_commit(&self) -> bool {
        !self.committing
            && !self.commit_message.trim().is_empty()
            && !self.repo_state.staged_files.is_empty()
    }

    fn request_commit(&mut self, cx: &mut Context<Self>) {
        if !self.can_commit() {
            return;
        }
        cx.emit(GitCommitRequested {
            message: self.commit_message.trim().to_string(),
            paths: self.staged_paths(),
        });
    }

    fn render_section_header(
        &self,
        title: &str,
        count: usize,
        section_idx: usize,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let is_expanded = self.section_expanded[section_idx];
        let title = title.to_string();

        div()
            .id(section_idx)
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .h(px(28.0))
            .px(px(theme::DRAWER_PADDING))
            .cursor_pointer()
            .on_press(cx.listener(move |this, _, _, cx| {
                this.toggle_section(section_idx, cx);
            }))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .child(
                        svg()
                            .path(if is_expanded {
                                "icons/chevron-down.svg"
                            } else {
                                "icons/chevron-right.svg"
                            })
                            .size(px(theme::FONT_DETAIL))
                            .text_color(rgb(theme::TEXT_MUTED))
                            .left(px(-2.0)),
                    )
                    .child(
                        div()
                            .text_color(rgb(theme::TEXT_MUTED))
                            .text_size(px(theme::FONT_DETAIL))
                            .font_weight(FontWeight::MEDIUM)
                            .child(title),
                    ),
            )
            .child(
                div()
                    .px_1()
                    .text_color(rgb(theme::TEXT_MUTED))
                    .text_size(px(theme::FONT_DETAIL))
                    .child(count.to_string()),
            )
    }

    fn render_file_entry(&self, file: &GitFileEntry, cx: &mut Context<Self>) -> impl IntoElement {
        let path = file.path.clone();
        let filename = file.filename.clone();
        let status = file.status;
        let section = file.section;
        let insertions = file.insertions;
        let deletions = file.deletions;

        div()
            .id(SharedString::from(path.clone()))
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .h(px(28.0))
            .px(px(theme::DRAWER_PADDING))
            .cursor_pointer()
            .on_press({
                let path = path.clone();
                cx.listener(move |_this, _, _, cx| {
                    cx.emit(GitFileSelected {
                        path: path.clone(),
                        section,
                    });
                })
            })
            .on_long_press({
                let path = path.clone();
                cx.listener(move |_this, _, _, cx| {
                    cx.emit(GitFileLongPressed {
                        path: path.clone(),
                        section,
                    });
                })
            })
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .overflow_hidden()
                    .child(
                        div()
                            .w(px(ICON_SIZE))
                            .text_color(rgb(theme::TEXT_MUTED))
                            .text_size(px(theme::FONT_BODY))
                            .child(status.icon()),
                    )
                    .child(
                        div()
                            .text_color(rgb(theme::TEXT_SECONDARY))
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
                                .text_color(rgb(theme::TEXT_MUTED))
                                .child(format!("+{}", insertions)),
                        )
                    })
                    .when(deletions > 0, |s| {
                        s.child(
                            div()
                                .text_color(rgb(theme::TEXT_MUTED))
                                .child(format!("-{}", deletions)),
                        )
                    }),
            )
    }

    fn render_commit_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let is_enabled = self.can_commit();
        let icon_path = if self.committing {
            "icons/refresh-ccw.svg"
        } else {
            "icons/check.svg"
        };
        let icon_color = if is_enabled || self.committing {
            theme::TEXT_SECONDARY
        } else {
            theme::TEXT_MUTED
        };
        let icon_size = if self.committing { px(14.0) } else { px(20.0) };

        div()
            .id("git-commit-button")
            .w(px(32.0))
            .h(px(32.0))
            .flex()
            .items_center()
            .justify_center()
            .opacity(if is_enabled || self.committing {
                1.0
            } else {
                0.35
            })
            .cursor_pointer()
            .on_pointer_down(|_, _, cx| {
                cx.stop_propagation();
            })
            .on_press(cx.listener(|this, _, _, cx| {
                this.request_commit(cx);
            }))
            .child(
                svg()
                    .path(icon_path)
                    .size(icon_size)
                    .text_color(rgb(icon_color)),
            )
    }

    fn render_commit_composer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let explicit_multiline = self.commit_message.contains('\n');
        div()
            .px(px(theme::DRAWER_PADDING))
            .pt(px(theme::SPACING_SM))
            .pb(px(theme::SPACING_SM))
            .child(
                div()
                    .relative()
                    .w_full()
                    .child(self.commit_input.clone())
                    .child(if explicit_multiline {
                        div()
                            .absolute()
                            .right(px(4.0))
                            .top(px(8.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(self.render_commit_button(cx))
                    } else {
                        div()
                            .absolute()
                            .right(px(4.0))
                            .top_0()
                            .bottom_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(self.render_commit_button(cx))
                    }),
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
        // let branch = self.repo_state.branch.clone();

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

        let staged_header = self
            .render_section_header("Staged changes", self.repo_state.total_staged(), 0, cx)
            .into_any_element();
        let unstaged_header = self
            .render_section_header("Changes", self.repo_state.total_unstaged(), 1, cx)
            .into_any_element();
        let untracked_header = self
            .render_section_header("Untracked", self.repo_state.total_untracked(), 2, cx)
            .into_any_element();

        let show_staged = self.section_expanded[0];
        let show_unstaged = self.section_expanded[1];
        let show_untracked = self.section_expanded[2];

        div()
            .track_focus(&self.focus_handle)
            .on_pointer_down(|_, window, _cx| {
                window.hide_soft_keyboard();
            })
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(theme::BG_PRIMARY))
            .child(self.render_commit_composer(cx))
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
