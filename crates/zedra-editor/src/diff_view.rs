use std::ops::Range;

use gpui::prelude::FluentBuilder;
use gpui::*;

use crate::highlighter::Highlighter;
use crate::theme::SyntaxTheme;

const LINE_HEIGHT: f32 = 20.0;
const GUTTER_WIDTH: f32 = 40.0;
const FONT_SIZE: f32 = 13.0;

/// Type of change for a diff line
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffLineKind {
    /// Line exists in both versions (context)
    Unchanged,
    /// Line was added in new version
    Added,
    /// Line was removed from old version
    Removed,
    /// Header/separator line
    Header,
}

/// A single line in the diff view
#[derive(Clone, Debug)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub old_line_num: Option<usize>,
    pub new_line_num: Option<usize>,
    pub content: String,
}

/// A hunk of changes in a diff
#[derive(Clone, Debug)]
pub struct DiffHunk {
    pub old_start: usize,
    pub old_count: usize,
    pub new_start: usize,
    pub new_count: usize,
    pub lines: Vec<DiffLine>,
}

/// A file diff with metadata
#[derive(Clone, Debug)]
pub struct FileDiff {
    pub old_path: String,
    pub new_path: String,
    pub hunks: Vec<DiffHunk>,
}

/// VS Code-like git diff view with syntax highlighting
pub struct DiffView {
    diffs: Vec<FileDiff>,
    current_file: usize,
    highlighter: Highlighter,
    theme: SyntaxTheme,
    scroll_handle: UniformListScrollHandle,
    focus_handle: FocusHandle,
    /// Unified view mode (true) or side-by-side (false)
    unified_view: bool,
}

impl DiffView {
    pub fn new(cx: &mut App) -> Self {
        Self {
            diffs: Self::sample_diffs(),
            current_file: 0,
            highlighter: Highlighter::rust(),
            theme: SyntaxTheme::default_dark(),
            scroll_handle: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            unified_view: true,
        }
    }

    /// Generate sample diff data for preview
    fn sample_diffs() -> Vec<FileDiff> {
        vec![
            FileDiff {
                old_path: "src/main.rs".to_string(),
                new_path: "src/main.rs".to_string(),
                hunks: vec![
                    DiffHunk {
                        old_start: 1,
                        old_count: 8,
                        new_start: 1,
                        new_count: 12,
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
                                content: "use std::path::Path;".to_string(),
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
                                content: "    println!(\"Hello, world!\");".to_string(),
                            },
                            DiffLine {
                                kind: DiffLineKind::Added,
                                old_line_num: None,
                                new_line_num: Some(6),
                                content: "    let config = load_config();".to_string(),
                            },
                            DiffLine {
                                kind: DiffLineKind::Added,
                                old_line_num: None,
                                new_line_num: Some(7),
                                content: "    println!(\"Config loaded: {:?}\", config);".to_string(),
                            },
                            DiffLine {
                                kind: DiffLineKind::Added,
                                old_line_num: None,
                                new_line_num: Some(8),
                                content: "    run_app(config);".to_string(),
                            },
                            DiffLine {
                                kind: DiffLineKind::Unchanged,
                                old_line_num: Some(5),
                                new_line_num: Some(9),
                                content: "}".to_string(),
                            },
                        ],
                    },
                    DiffHunk {
                        old_start: 10,
                        old_count: 5,
                        new_start: 14,
                        new_count: 10,
                        lines: vec![
                            DiffLine {
                                kind: DiffLineKind::Unchanged,
                                old_line_num: Some(10),
                                new_line_num: Some(14),
                                content: "fn process_data(data: &str) -> Result<(), Error> {".to_string(),
                            },
                            DiffLine {
                                kind: DiffLineKind::Removed,
                                old_line_num: Some(11),
                                new_line_num: None,
                                content: "    // TODO: implement".to_string(),
                            },
                            DiffLine {
                                kind: DiffLineKind::Removed,
                                old_line_num: Some(12),
                                new_line_num: None,
                                content: "    Ok(())".to_string(),
                            },
                            DiffLine {
                                kind: DiffLineKind::Added,
                                old_line_num: None,
                                new_line_num: Some(15),
                                content: "    let parsed = parse_input(data)?;".to_string(),
                            },
                            DiffLine {
                                kind: DiffLineKind::Added,
                                old_line_num: None,
                                new_line_num: Some(16),
                                content: "    validate(&parsed)?;".to_string(),
                            },
                            DiffLine {
                                kind: DiffLineKind::Added,
                                old_line_num: None,
                                new_line_num: Some(17),
                                content: "    let result = transform(parsed);".to_string(),
                            },
                            DiffLine {
                                kind: DiffLineKind::Added,
                                old_line_num: None,
                                new_line_num: Some(18),
                                content: "    save_output(&result)?;".to_string(),
                            },
                            DiffLine {
                                kind: DiffLineKind::Added,
                                old_line_num: None,
                                new_line_num: Some(19),
                                content: "    Ok(())".to_string(),
                            },
                            DiffLine {
                                kind: DiffLineKind::Unchanged,
                                old_line_num: Some(13),
                                new_line_num: Some(20),
                                content: "}".to_string(),
                            },
                        ],
                    },
                ],
            },
            FileDiff {
                old_path: "src/config.rs".to_string(),
                new_path: "src/config.rs".to_string(),
                hunks: vec![DiffHunk {
                    old_start: 1,
                    old_count: 6,
                    new_start: 1,
                    new_count: 9,
                    lines: vec![
                        DiffLine {
                            kind: DiffLineKind::Added,
                            old_line_num: None,
                            new_line_num: Some(1),
                            content: "use serde::{Deserialize, Serialize};".to_string(),
                        },
                        DiffLine {
                            kind: DiffLineKind::Added,
                            old_line_num: None,
                            new_line_num: Some(2),
                            content: "".to_string(),
                        },
                        DiffLine {
                            kind: DiffLineKind::Added,
                            old_line_num: None,
                            new_line_num: Some(3),
                            content: "#[derive(Debug, Serialize, Deserialize)]".to_string(),
                        },
                        DiffLine {
                            kind: DiffLineKind::Unchanged,
                            old_line_num: Some(1),
                            new_line_num: Some(4),
                            content: "pub struct Config {".to_string(),
                        },
                        DiffLine {
                            kind: DiffLineKind::Removed,
                            old_line_num: Some(2),
                            new_line_num: None,
                            content: "    pub name: String,".to_string(),
                        },
                        DiffLine {
                            kind: DiffLineKind::Added,
                            old_line_num: None,
                            new_line_num: Some(5),
                            content: "    pub app_name: String,".to_string(),
                        },
                        DiffLine {
                            kind: DiffLineKind::Added,
                            old_line_num: None,
                            new_line_num: Some(6),
                            content: "    pub version: String,".to_string(),
                        },
                        DiffLine {
                            kind: DiffLineKind::Unchanged,
                            old_line_num: Some(3),
                            new_line_num: Some(7),
                            content: "    pub debug: bool,".to_string(),
                        },
                        DiffLine {
                            kind: DiffLineKind::Unchanged,
                            old_line_num: Some(4),
                            new_line_num: Some(8),
                            content: "}".to_string(),
                        },
                    ],
                }],
            },
        ]
    }

    fn current_diff(&self) -> Option<&FileDiff> {
        self.diffs.get(self.current_file)
    }

    fn total_lines(&self) -> usize {
        let Some(diff) = self.current_diff() else {
            return 0;
        };
        // File header + hunk headers + all lines
        let mut count = 1; // file header
        for hunk in &diff.hunks {
            count += 1; // hunk header
            count += hunk.lines.len();
        }
        count
    }

    fn get_line(&self, index: usize) -> Option<DiffLine> {
        let diff = self.current_diff()?;
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
            // Hunk header
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

            // Hunk lines
            for line in &hunk.lines {
                if index == current {
                    return Some(line.clone());
                }
                current += 1;
            }
        }
        None
    }

    /// Compute syntax highlights for a line
    fn line_highlights(&mut self, content: &str) -> Vec<(Range<usize>, HighlightStyle)> {
        if content.is_empty() {
            return Vec::new();
        }

        let content_len = content.len();

        // Parse content for syntax highlighting
        self.highlighter.parse(content);

        // Get highlights with bounds checking in highlighter
        let raw_highlights = self.highlighter.highlights(content, 0..content_len);

        let mut result: Vec<(Range<usize>, HighlightStyle)> = Vec::new();

        for (span_range, capture_name) in &raw_highlights {
            // Clamp ranges to content bounds to prevent out-of-bounds access
            let start = span_range.start.min(content_len);
            let end = span_range.end.min(content_len);
            if start < end {
                if let Some(style) = self.theme.get(capture_name) {
                    result.push((start..end, style));
                }
            }
        }

        // Sort and deduplicate
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

    fn toggle_view_mode(&mut self, cx: &mut Context<Self>) {
        self.unified_view = !self.unified_view;
        cx.notify();
    }

    fn next_file(&mut self, cx: &mut Context<Self>) {
        if self.current_file + 1 < self.diffs.len() {
            self.current_file += 1;
            cx.notify();
        }
    }

    fn prev_file(&mut self, cx: &mut Context<Self>) {
        if self.current_file > 0 {
            self.current_file -= 1;
            cx.notify();
        }
    }

    fn render_file_tabs(&self, cx: &Context<Self>) -> impl IntoElement {
        let current = self.current_file;
        div()
            .flex()
            .flex_row()
            .gap_1()
            .px_2()
            .py_1()
            .bg(rgb(0x21252b))
            .border_b_1()
            .border_color(rgb(0x181a1f))
            .children(self.diffs.iter().enumerate().map(|(i, diff)| {
                let is_active = i == current;
                let filename = diff.new_path.rsplit('/').next().unwrap_or(&diff.new_path);
                div()
                    .px_3()
                    .py_1()
                    .rounded(px(4.0))
                    .cursor_pointer()
                    .text_sm()
                    .when(is_active, |s| s.bg(rgb(0x2c313a)).text_color(rgb(0xabb2bf)))
                    .when(!is_active, |s| {
                        s.text_color(rgb(0x636d83)).hover(|s| s.bg(rgb(0x2c313a)))
                    })
                    .child(filename.to_string())
            }))
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let stats = self
            .current_diff()
            .map(|diff| {
                let (added, removed) = diff.hunks.iter().fold((0, 0), |(a, r), hunk| {
                    let adds = hunk
                        .lines
                        .iter()
                        .filter(|l| l.kind == DiffLineKind::Added)
                        .count();
                    let removes = hunk
                        .lines
                        .iter()
                        .filter(|l| l.kind == DiffLineKind::Removed)
                        .count();
                    (a + adds, r + removes)
                });
                (added, removed)
            })
            .unwrap_or((0, 0));

        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px_3()
            .py_2()
            .bg(rgb(0x21252b))
            .border_b_1()
            .border_color(rgb(0x181a1f))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_3()
                    .items_center()
                    .child(
                        div()
                            .text_color(rgb(0x98c379))
                            .text_sm()
                            .child(format!("+{}", stats.0)),
                    )
                    .child(
                        div()
                            .text_color(rgb(0xe06c75))
                            .text_sm()
                            .child(format!("-{}", stats.1)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_2()
                    // Previous file button
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .rounded(px(4.0))
                            .cursor_pointer()
                            .text_color(rgb(0xabb2bf))
                            .hover(|s| s.bg(rgb(0x2c313a)))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| this.prev_file(cx)),
                            )
                            .child("<-"),
                    )
                    // File counter
                    .child(
                        div()
                            .text_color(rgb(0x636d83))
                            .text_sm()
                            .child(format!("{}/{}", self.current_file + 1, self.diffs.len())),
                    )
                    // Next file button
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .rounded(px(4.0))
                            .cursor_pointer()
                            .text_color(rgb(0xabb2bf))
                            .hover(|s| s.bg(rgb(0x2c313a)))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| this.next_file(cx)),
                            )
                            .child("->"),
                    )
                    // View toggle
                    .child(
                        div()
                            .ml_2()
                            .px_2()
                            .py_1()
                            .rounded(px(4.0))
                            .cursor_pointer()
                            .bg(rgb(0x2c313a))
                            .text_color(rgb(0xabb2bf))
                            .text_sm()
                            .hover(|s| s.bg(rgb(0x3e4451)))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| this.toggle_view_mode(cx)),
                            )
                            .child(if self.unified_view {
                                "Split"
                            } else {
                                "Unified"
                            }),
                    ),
            )
    }
}

impl Focusable for DiffView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for DiffView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let line_count = self.total_lines();

        // Pre-compute line data for the list
        let line_data: Vec<Option<DiffLine>> =
            (0..line_count).map(|i| self.get_line(i)).collect();

        let text_style = {
            let mut style = window.text_style();
            style.color = rgb(0xabb2bf).into();
            style.font_size = px(FONT_SIZE).into();
            style
        };

        // Pre-compute highlights for non-header lines
        let highlights: Vec<Vec<(Range<usize>, HighlightStyle)>> = line_data
            .iter()
            .map(|line| {
                line.as_ref()
                    .filter(|l| l.kind != DiffLineKind::Header)
                    .map(|l| self.line_highlights(&l.content))
                    .unwrap_or_default()
            })
            .collect();

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x282c34))
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                match event.keystroke.key.as_str() {
                    "left" | "[" => this.prev_file(cx),
                    "right" | "]" => this.next_file(cx),
                    "space" | "v" => this.toggle_view_mode(cx),
                    _ => {}
                }
            }))
            // File tabs
            .child(self.render_file_tabs(cx))
            // Toolbar with stats and controls
            .child(self.render_toolbar(cx))
            // Diff content
            .child(
                uniform_list("diff-lines", line_count, {
                    let text_style = text_style.clone();
                    move |range: Range<usize>, _window: &mut Window, _cx: &mut App| {
                        range
                            .map(|i| {
                                let Some(line) = &line_data[i] else {
                                    return div().h(px(LINE_HEIGHT));
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

                                // For headers, use a special style
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
                                        );
                                }

                                // Apply syntax highlighting with offset for prefix
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
                                    StyledText::new(" ")
                                        .with_default_highlights(&text_style, Vec::new())
                                } else {
                                    StyledText::new(display_content.clone())
                                        .with_default_highlights(&text_style, adjusted_highlights)
                                };

                                div()
                                    .flex()
                                    .flex_row()
                                    .h(px(LINE_HEIGHT))
                                    .bg(bg_color)
                                    // Line number gutter
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
                                    // Content
                                    .child(
                                        div()
                                            .flex_1()
                                            .h(px(LINE_HEIGHT))
                                            .flex()
                                            .items_center()
                                            .child(styled_text),
                                    )
                            })
                            .collect()
                    }
                })
                .track_scroll(&self.scroll_handle)
                .flex_1(),
            )
    }
}
