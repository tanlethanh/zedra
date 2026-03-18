//! GitDiffView - Standalone diff viewer for a single file
//!
//! Pushed onto the StackNavigator when a git file is selected from GitSidebar.
//! Also contains unified-diff data structures and parser.

use std::ops::Range;
use std::rc::Rc;

use gpui::*;

use super::syntax_highlighter::Highlighter;
use super::syntax_theme::SyntaxTheme;
use crate::platform_bridge;
use crate::theme;

// ── Diff data types ─────────────────────────────────────────────────────────

/// The kind of change a diff line represents.
#[derive(Clone, Debug, PartialEq)]
pub enum DiffLineKind {
    Header,
    Added,
    Removed,
    Unchanged,
}

/// A single line in a diff hunk.
#[derive(Clone, Debug)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub old_line_num: Option<usize>,
    pub new_line_num: Option<usize>,
    pub content: String,
}

/// A contiguous hunk of changes.
#[derive(Clone, Debug)]
pub struct DiffHunk {
    pub old_start: usize,
    pub old_count: usize,
    pub new_start: usize,
    pub new_count: usize,
    pub lines: Vec<DiffLine>,
}

/// A single file's diff (old path → new path with hunks).
#[derive(Clone, Debug)]
pub struct FileDiff {
    pub old_path: String,
    pub new_path: String,
    pub hunks: Vec<DiffHunk>,
}

// ── Unified-diff parser ─────────────────────────────────────────────────────

/// Parse a unified diff string into a list of per-file diffs.
pub fn parse_unified_diff(text: &str) -> Vec<FileDiff> {
    let mut diffs: Vec<FileDiff> = Vec::new();
    let mut current_diff: Option<FileDiff> = None;
    let mut current_hunk: Option<DiffHunk> = None;
    let mut old_line: usize = 0;
    let mut new_line: usize = 0;

    for raw_line in text.lines() {
        if let Some(path) = raw_line.strip_prefix("--- ") {
            if let (Some(diff), Some(hunk)) = (&mut current_diff, current_hunk.take()) {
                diff.hunks.push(hunk);
            }
            if let Some(diff) = current_diff.take() {
                diffs.push(diff);
            }
            let path = path.strip_prefix("a/").unwrap_or(path);
            current_diff = Some(FileDiff {
                old_path: path.to_string(),
                new_path: String::new(),
                hunks: Vec::new(),
            });
        } else if let Some(path) = raw_line.strip_prefix("+++ ") {
            let path = path.strip_prefix("b/").unwrap_or(path);
            if let Some(diff) = &mut current_diff {
                diff.new_path = path.to_string();
            }
        } else if raw_line.starts_with("@@ ") {
            if let (Some(diff), Some(hunk)) = (&mut current_diff, current_hunk.take()) {
                diff.hunks.push(hunk);
            }
            let (os, oc, ns, nc) = parse_hunk_header(raw_line);
            old_line = os;
            new_line = ns;
            current_hunk = Some(DiffHunk {
                old_start: os,
                old_count: oc,
                new_start: ns,
                new_count: nc,
                lines: Vec::new(),
            });
        } else if let Some(hunk) = &mut current_hunk {
            if let Some(content) = raw_line.strip_prefix('+') {
                hunk.lines.push(DiffLine {
                    kind: DiffLineKind::Added,
                    old_line_num: None,
                    new_line_num: Some(new_line),
                    content: content.to_string(),
                });
                new_line += 1;
            } else if let Some(content) = raw_line.strip_prefix('-') {
                hunk.lines.push(DiffLine {
                    kind: DiffLineKind::Removed,
                    old_line_num: Some(old_line),
                    new_line_num: None,
                    content: content.to_string(),
                });
                old_line += 1;
            } else {
                let content = raw_line.strip_prefix(' ').unwrap_or(raw_line);
                hunk.lines.push(DiffLine {
                    kind: DiffLineKind::Unchanged,
                    old_line_num: Some(old_line),
                    new_line_num: Some(new_line),
                    content: content.to_string(),
                });
                old_line += 1;
                new_line += 1;
            }
        }
    }

    if let (Some(diff), Some(hunk)) = (&mut current_diff, current_hunk.take()) {
        diff.hunks.push(hunk);
    }
    if let Some(diff) = current_diff.take() {
        diffs.push(diff);
    }

    diffs
}

fn parse_hunk_header(line: &str) -> (usize, usize, usize, usize) {
    let trimmed = line
        .strip_prefix("@@ ")
        .unwrap_or(line)
        .split(" @@")
        .next()
        .unwrap_or("");

    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    let (old_start, old_count) = parse_range(parts.first().unwrap_or(&""));
    let (new_start, new_count) = parse_range(parts.get(1).unwrap_or(&""));
    (old_start, old_count, new_start, new_count)
}

fn parse_range(s: &str) -> (usize, usize) {
    let s = s.trim_start_matches(['-', '+']);
    if let Some((start, count)) = s.split_once(',') {
        (start.parse().unwrap_or(1), count.parse().unwrap_or(0))
    } else {
        (s.parse().unwrap_or(1), 1)
    }
}

/// Return sample `FileDiff` entries for offline/demo use.
pub fn sample_diffs() -> Vec<FileDiff> {
    vec![FileDiff {
        old_path: "src/lib.rs".to_string(),
        new_path: "src/lib.rs".to_string(),
        hunks: Vec::new(),
    }]
}

// ── GitDiffView ─────────────────────────────────────────────────────────────

const LINE_HEIGHT: f32 = theme::EDITOR_LINE_HEIGHT;
const GUTTER_WIDTH: f32 = theme::EDITOR_GUTTER_WIDTH;
const FONT_SIZE: f32 = theme::EDITOR_FONT_SIZE;
const GUTTER_FONT_SIZE: f32 = theme::EDITOR_GUTTER_FONT_SIZE;
const BOTTOM_INSET_MIN: f32 = 100.0;

struct CachedDiffLine {
    line: Option<DiffLine>,
    highlights: Vec<(Range<usize>, HighlightStyle)>,
    /// Length in chars (used to cap horizontal scroll).
    char_len: usize,
}

pub struct GitDiffView {
    diff: FileDiff,
    highlighter: Highlighter,
    theme: SyntaxTheme,
    scroll_handle: UniformListScrollHandle,
    focus_handle: FocusHandle,
    cached_lines: Rc<Vec<CachedDiffLine>>,
    lines_dirty: bool,
    h_scroll_offset: f32,
    max_line_chars: usize,
    h_scroll_active: bool,
}

impl GitDiffView {
    pub fn new(diff: FileDiff, file_path: String, cx: &mut App) -> Self {
        let highlighter = Highlighter::from_filename(&file_path);
        Self {
            diff,
            highlighter,
            theme: SyntaxTheme::default_dark(),
            scroll_handle: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            cached_lines: Rc::new(Vec::new()),
            lines_dirty: true,
            h_scroll_offset: 0.0,
            max_line_chars: 0,
            h_scroll_active: false,
        }
    }

    fn rebuild_line_cache(&mut self) {
        let line_count = self.total_lines();
        let lines: Vec<CachedDiffLine> = (0..line_count)
            .map(|i| {
                let line = self.get_line(i);
                let (highlights, char_len) = match &line {
                    Some(l) if l.kind != DiffLineKind::Header => {
                        let h = self.line_highlights(&l.content);
                        // prefix ("+ ", "- ", "  ") adds 2 chars
                        let len = l.content.chars().count() + 2;
                        (h, len)
                    }
                    Some(l) => (Vec::new(), l.content.chars().count()),
                    None => (Vec::new(), 0),
                };
                CachedDiffLine { line, highlights, char_len }
            })
            .collect();
        self.max_line_chars = lines.iter().map(|l| l.char_len).max().unwrap_or(0);
        self.cached_lines = Rc::new(lines);
        self.lines_dirty = false;
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
        self.highlighter.parse_fresh(content);
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

        super::merge_highlights(result)
    }
}

impl EventEmitter<()> for GitDiffView {}

impl Focusable for GitDiffView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for GitDiffView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.lines_dirty {
            self.rebuild_line_cache();
        }

        let line_count = self.cached_lines.len();
        let cached_lines = self.cached_lines.clone();
        let bottom_inset = f32::max(platform_bridge::home_indicator_inset(), BOTTOM_INSET_MIN);
        let extra_items = (bottom_inset / LINE_HEIGHT).ceil() as usize;
        let h_scroll_offset = self.h_scroll_offset;
        let scroll_y_lock = self.scroll_handle.0.borrow().base_handle.offset().y;

        let text_style = {
            let mut style = window.text_style();
            style.color = rgb(0xcacaca).into();
            style.font_size = px(FONT_SIZE).into();
            style
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x0e0c0c))
            .track_focus(&self.focus_handle)
            .on_scroll_wheel(cx.listener(move |this, event: &ScrollWheelEvent, _window, cx| {
                let (delta_x, delta_y) = match event.delta {
                    ScrollDelta::Pixels(p) => (f32::from(p.x), f32::from(p.y)),
                    ScrollDelta::Lines(l) => (l.x * 20.0, l.y * 20.0),
                };
                if delta_y.abs() > delta_x.abs() * 3.0 {
                    this.h_scroll_active = false;
                } else if delta_x.abs() > delta_y.abs() * 2.5 && delta_x.abs() > 5.0 {
                    this.h_scroll_active = true;
                }
                if this.h_scroll_active && delta_x.abs() > 0.1 {
                    let char_width = FONT_SIZE * 0.6;
                    let max_offset = (this.max_line_chars as f32 * char_width).max(0.0);
                    this.h_scroll_offset = (this.h_scroll_offset - delta_x).clamp(0.0, max_offset);
                    this.scroll_handle
                        .0
                        .borrow()
                        .base_handle
                        .set_offset(point(px(0.0), scroll_y_lock));
                    cx.notify();
                }
            }))
            .child(
                uniform_list("git-diff-view-lines", line_count + extra_items, {
                    let text_style = text_style.clone();
                    move |range: Range<usize>, _window: &mut Window, _cx: &mut App| {
                        range
                            .map(|i| {
                                if i >= line_count {
                                    return div().h(px(LINE_HEIGHT)).into_any_element();
                                }

                                let cached = &cached_lines[i];
                                let Some(line) = &cached.line else {
                                    return div().h(px(LINE_HEIGHT)).into_any_element();
                                };

                                let (bg_color, gutter_text, prefix) = match line.kind {
                                    DiffLineKind::Header => (rgb(0x131313), "".to_string(), ""),
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

                                let adjusted_highlights: Vec<(Range<usize>, HighlightStyle)> =
                                    cached
                                        .highlights
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
                                    .child(
                                        div()
                                            .w(px(GUTTER_WIDTH))
                                            .h(px(LINE_HEIGHT))
                                            .flex()
                                            .items_center()
                                            .justify_end()
                                            .pr_2()
                                            .text_color(rgb(0x404040))
                                            .text_size(px(GUTTER_FONT_SIZE))
                                            .child(gutter_text),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .h(px(LINE_HEIGHT))
                                            .overflow_hidden()
                                            .relative()
                                            .child(
                                                div()
                                                    .absolute()
                                                    .top(px(0.0))
                                                    .left(px(-h_scroll_offset))
                                                    .h(px(LINE_HEIGHT))
                                                    .flex()
                                                    .items_center()
                                                    .text_size(px(FONT_SIZE))
                                                    .child(styled_text),
                                            ),
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
