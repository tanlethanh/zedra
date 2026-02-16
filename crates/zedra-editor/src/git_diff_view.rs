//! GitDiffView - Standalone diff viewer for a single file
//!
//! Pushed onto the StackNavigator when a git file is selected from GitSidebar.

use std::ops::Range;

use gpui::prelude::FluentBuilder;
use gpui::*;

use crate::diff_view::{DiffHunk, DiffLine, DiffLineKind, FileDiff};
use crate::highlighter::Highlighter;
use crate::theme::SyntaxTheme;

const LINE_HEIGHT: f32 = 20.0;
const GUTTER_WIDTH: f32 = 40.0;
const FONT_SIZE: f32 = 13.0;

pub struct GitDiffView {
    diff: FileDiff,
    file_path: String,
    highlighter: Highlighter,
    theme: SyntaxTheme,
    scroll_handle: UniformListScrollHandle,
    focus_handle: FocusHandle,
}

impl GitDiffView {
    pub fn new(diff: FileDiff, file_path: String, cx: &mut App) -> Self {
        Self {
            diff,
            file_path,
            highlighter: Highlighter::rust(),
            theme: SyntaxTheme::default_dark(),
            scroll_handle: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
        }
    }

    fn total_lines(&self) -> usize {
        let mut count = 1; // file header
        for hunk in &self.diff.hunks {
            count += 1; // hunk header
            count += hunk.lines.len();
        }
        count
    }

    fn get_line(&self, index: usize) -> Option<DiffLine> {
        let mut current = 0;

        if index == current {
            return Some(DiffLine {
                kind: DiffLineKind::Header,
                old_line_num: None,
                new_line_num: None,
                content: format!("{} -> {}", self.diff.old_path, self.diff.new_path),
            });
        }
        current += 1;

        for hunk in &self.diff.hunks {
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

    fn line_highlights(&mut self, content: &str) -> Vec<(Range<usize>, HighlightStyle)> {
        if content.is_empty() {
            return Vec::new();
        }

        let content_len = content.len();
        self.highlighter.parse(content);
        let raw_highlights = self.highlighter.highlights(content, 0..content_len);
        let mut result: Vec<(Range<usize>, HighlightStyle)> = Vec::new();

        for (span_range, capture_name) in &raw_highlights {
            if let Some(style) = self.theme.get(capture_name) {
                let start = span_range.start.min(content_len);
                let end = span_range.end.min(content_len);
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
}

impl EventEmitter<()> for GitDiffView {}

impl Focusable for GitDiffView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for GitDiffView {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let line_count = self.total_lines();

        let line_data: Vec<Option<DiffLine>> =
            (0..line_count).map(|i| self.get_line(i)).collect();

        let text_style = {
            let mut style = window.text_style();
            style.color = rgb(0xcacaca).into();
            style.font_size = px(FONT_SIZE).into();
            style
        };

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
            .bg(rgb(0x0e0c0c))
            .track_focus(&self.focus_handle)
            .child(
                uniform_list("git-diff-view-lines", line_count, {
                    let text_style = text_style.clone();
                    move |range: Range<usize>, _window: &mut Window, _cx: &mut App| {
                        range
                            .map(|i| {
                                let Some(line) = &line_data[i] else {
                                    return div().h(px(LINE_HEIGHT)).into_any_element();
                                };

                                let (bg_color, gutter_text, prefix) = match line.kind {
                                    DiffLineKind::Header => {
                                        (rgb(0x131313), "".to_string(), "")
                                    }
                                    DiffLineKind::Added => {
                                        let num = line
                                            .new_line_num
                                            .map(|n| format!("{:>3}", n))
                                            .unwrap_or_default();
                                        (rgb(0x162016), num, "+ ")
                                    }
                                    DiffLineKind::Removed => {
                                        let num = line
                                            .old_line_num
                                            .map(|n| format!("{:>3}", n))
                                            .unwrap_or_default();
                                        (rgb(0x201616), num, "- ")
                                    }
                                    DiffLineKind::Unchanged => {
                                        let num = line
                                            .new_line_num
                                            .map(|n| format!("{:>3}", n))
                                            .unwrap_or_default();
                                        (rgb(0x0e0c0c), num, "  ")
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
                                let adjusted_highlights: Vec<(
                                    Range<usize>,
                                    HighlightStyle,
                                )> = line_highlights
                                    .iter()
                                    .map(|(range, style)| {
                                        let offset = prefix.len();
                                        (
                                            (range.start + offset)..(range.end + offset),
                                            *style,
                                        )
                                    })
                                    .collect();

                                let styled_text = if display_content.is_empty() {
                                    StyledText::new(" ")
                                        .with_default_highlights(&text_style, Vec::new())
                                } else {
                                    StyledText::new(display_content.clone())
                                        .with_default_highlights(
                                            &text_style,
                                            adjusted_highlights,
                                        )
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
                                            .text_color(rgb(0x404040))
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
                .flex_1(),
            )
    }
}
