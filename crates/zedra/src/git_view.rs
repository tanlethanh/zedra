// Git view: displays git status, diff, and log.
//
// Shows the current branch, changed files, and recent commits.
// Actions: stage, commit, view diff. All data comes from the RPC layer
// but for now uses demo data to validate the UI.

use gpui::prelude::FluentBuilder;
use gpui::*;

// ---------------------------------------------------------------------------
// Types (mirror zedra-rpc types but without the dependency for Android builds)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct GitFileEntry {
    pub path: String,
    pub status: String,
}

#[derive(Clone, Debug)]
pub struct GitCommitEntry {
    pub id: String,
    pub message: String,
    pub author: String,
}

#[derive(Clone, Debug)]
pub struct GitDiffRequested {
    pub path: String,
}

#[derive(Clone, Debug)]
pub struct GitCommitRequested {
    pub message: String,
    pub paths: Vec<String>,
}

// ---------------------------------------------------------------------------
// GitView
// ---------------------------------------------------------------------------

pub struct GitView {
    branch: String,
    changes: Vec<GitFileEntry>,
    log: Vec<GitCommitEntry>,
    diff_text: Option<String>,
    focus_handle: FocusHandle,
}

impl EventEmitter<GitDiffRequested> for GitView {}
impl EventEmitter<GitCommitRequested> for GitView {}

impl GitView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            branch: "main".into(),
            changes: demo_changes(),
            log: demo_log(),
            diff_text: None,
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn set_status(
        &mut self,
        branch: String,
        changes: Vec<GitFileEntry>,
        cx: &mut Context<Self>,
    ) {
        self.branch = branch;
        self.changes = changes;
        cx.notify();
    }

    pub fn set_log(&mut self, log: Vec<GitCommitEntry>, cx: &mut Context<Self>) {
        self.log = log;
        cx.notify();
    }

    pub fn set_diff(&mut self, diff: String, cx: &mut Context<Self>) {
        self.diff_text = Some(diff);
        cx.notify();
    }
}

impl Focusable for GitView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for GitView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut content = div().flex().flex_col().gap_4().p_4();

        // Branch indicator
        content = content.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .px_2()
                        .py_1()
                        .bg(rgb(0x98c379))
                        .rounded(px(4.0))
                        .text_color(rgb(0x282c34))
                        .text_sm()
                        .child(format!("⎇ {}", self.branch)),
                ),
        );

        // Changed files section
        if !self.changes.is_empty() {
            content = content.child(
                div()
                    .text_color(rgb(0xabb2bf))
                    .text_sm()
                    .child(format!("Changes ({})", self.changes.len())),
            );

            let mut changes_list = div().flex().flex_col().gap_1();
            for (i, change) in self.changes.iter().enumerate() {
                let status_color = match change.status.as_str() {
                    "modified" => rgb(0xe5c07b),
                    "added" | "untracked" => rgb(0x98c379),
                    "deleted" => rgb(0xe06c75),
                    _ => rgb(0xabb2bf),
                };
                let status_icon = match change.status.as_str() {
                    "modified" => "M",
                    "added" => "A",
                    "deleted" => "D",
                    "untracked" => "?",
                    _ => " ",
                };
                let path = change.path.clone();
                changes_list = changes_list.child(
                    div()
                        .id(ElementId::Name(format!("change-{}", i).into()))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py_1()
                        .rounded(px(4.0))
                        .cursor_pointer()
                        .hover(|s| s.bg(rgb(0x2c313a)))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |_this, _event, _window, cx| {
                                cx.emit(GitDiffRequested { path: path.clone() });
                            }),
                        )
                        .child(
                            div()
                                .w(px(16.0))
                                .text_color(status_color)
                                .text_sm()
                                .child(status_icon.to_string()),
                        )
                        .child(
                            div()
                                .text_color(rgb(0xabb2bf))
                                .text_sm()
                                .child(change.path.clone()),
                        ),
                );
            }
            content = content.child(changes_list);
        }

        // Diff view (if any)
        if let Some(ref diff) = self.diff_text {
            content = content.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_color(rgb(0xabb2bf))
                            .text_sm()
                            .child("Diff"),
                    )
                    .child(
                        div()
                            .p_3()
                            .bg(rgb(0x282c34))
                            .rounded(px(6.0))
                            .text_color(rgb(0x98c379))
                            .text_sm()
                            .child(diff.clone()),
                    ),
            );
        }

        // Recent commits
        if !self.log.is_empty() {
            content = content.child(
                div()
                    .text_color(rgb(0xabb2bf))
                    .text_sm()
                    .child("Recent Commits"),
            );

            let mut log_list = div().flex().flex_col().gap_1();
            for commit in &self.log {
                log_list = log_list.child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py_1()
                        .child(
                            div()
                                .text_color(rgb(0x61afef))
                                .text_sm()
                                .child(commit.id[..7].to_string()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .text_color(rgb(0xabb2bf))
                                .text_sm()
                                .child(commit.message.clone()),
                        )
                        .child(
                            div()
                                .text_color(rgb(0x5c6370))
                                .text_sm()
                                .child(commit.author.clone()),
                        ),
                );
            }
            content = content.child(log_list);
        }

        div()
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x1e1e1e))
            .child(
                div()
                    .id("git-view-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(content),
            )
    }
}

fn demo_changes() -> Vec<GitFileEntry> {
    vec![
        GitFileEntry {
            path: "src/lib.rs".into(),
            status: "modified".into(),
        },
        GitFileEntry {
            path: "src/main.rs".into(),
            status: "modified".into(),
        },
        GitFileEntry {
            path: "src/new_file.rs".into(),
            status: "untracked".into(),
        },
        GitFileEntry {
            path: "old_module.rs".into(),
            status: "deleted".into(),
        },
    ]
}

fn demo_log() -> Vec<GitCommitEntry> {
    vec![
        GitCommitEntry {
            id: "a1b2c3d4e5f6789012345678901234567890abcd".into(),
            message: "feat: add git view".into(),
            author: "Dev".into(),
        },
        GitCommitEntry {
            id: "b2c3d4e5f67890123456789012345678901abcde".into(),
            message: "fix: resolve layout issue".into(),
            author: "Dev".into(),
        },
        GitCommitEntry {
            id: "c3d4e5f678901234567890123456789012abcdef".into(),
            message: "chore: update dependencies".into(),
            author: "Dev".into(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_data_valid() {
        let changes = demo_changes();
        assert!(!changes.is_empty());
        assert!(changes
            .iter()
            .all(|c| !c.path.is_empty() && !c.status.is_empty()));

        let log = demo_log();
        assert!(!log.is_empty());
        assert!(log.iter().all(|l| l.id.len() >= 7));
    }

    #[test]
    fn git_file_entry_clone() {
        let entry = GitFileEntry {
            path: "test.rs".into(),
            status: "modified".into(),
        };
        let clone = entry.clone();
        assert_eq!(clone.path, entry.path);
    }

    #[test]
    fn git_commit_entry_debug() {
        let entry = GitCommitEntry {
            id: "abc1234".into(),
            message: "test".into(),
            author: "Dev".into(),
        };
        let dbg = format!("{:?}", entry);
        assert!(dbg.contains("abc1234"));
    }
}
