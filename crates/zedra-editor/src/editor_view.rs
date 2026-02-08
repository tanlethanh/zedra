use std::ops::Range;

use gpui::prelude::FluentBuilder;
use gpui::*;

use crate::buffer::Buffer;
use crate::highlighter::Highlighter;
use crate::theme::SyntaxTheme;

const LINE_HEIGHT: f32 = 20.0;
const GUTTER_WIDTH: f32 = 48.0;
const FONT_SIZE: f32 = 14.0;

/// A code editor view with syntax highlighting and virtual scrolling.
pub struct EditorView {
    buffer: Buffer,
    highlighter: Highlighter,
    theme: SyntaxTheme,
    cursor_offset: usize,
    scroll_handle: UniformListScrollHandle,
    focus_handle: FocusHandle,
}

impl EditorView {
    pub fn new(content: String, cx: &mut App) -> Self {
        let mut highlighter = Highlighter::rust();
        highlighter.parse(&content);

        Self {
            buffer: Buffer::new(content),
            highlighter,
            theme: SyntaxTheme::default_dark(),
            cursor_offset: 0,
            scroll_handle: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
        }
    }

    /// Replace the entire buffer content (e.g. when loading a remote file).
    pub fn set_content(&mut self, content: String) {
        self.buffer.set_text(content);
        self.highlighter.parse(self.buffer.text());
        self.cursor_offset = 0;
    }

    fn move_cursor_left(&mut self) {
        if self.cursor_offset > 0 {
            // Move back one character (handle UTF-8 properly)
            let text = self.buffer.text();
            self.cursor_offset = text[..self.cursor_offset]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
        }
    }

    fn move_cursor_right(&mut self) {
        let text = self.buffer.text();
        if self.cursor_offset < text.len() {
            self.cursor_offset = text[self.cursor_offset..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.cursor_offset + i)
                .unwrap_or(text.len());
        }
    }

    fn move_cursor_up(&mut self) {
        let (row, col) = self.buffer.offset_to_point(self.cursor_offset);
        if row > 0 {
            self.cursor_offset = self.buffer.point_to_offset(row - 1, col);
        }
    }

    fn move_cursor_down(&mut self) {
        let (row, col) = self.buffer.offset_to_point(self.cursor_offset);
        if row + 1 < self.buffer.line_count() {
            self.cursor_offset = self.buffer.point_to_offset(row + 1, col);
        }
    }

    fn insert_char(&mut self, character: &str) {
        self.buffer.insert(self.cursor_offset, character);
        self.cursor_offset += character.len();
        self.highlighter.parse(self.buffer.text());
    }

    fn insert_newline(&mut self) {
        self.buffer.insert(self.cursor_offset, "\n");
        self.cursor_offset += 1;
        self.highlighter.parse(self.buffer.text());
    }

    fn backspace(&mut self) {
        if self.cursor_offset > 0 {
            let prev = self.buffer.text()[..self.cursor_offset]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.buffer.delete(prev..self.cursor_offset);
            self.cursor_offset = prev;
            self.highlighter.parse(self.buffer.text());
        }
    }

    fn delete_forward(&mut self) {
        let text = self.buffer.text();
        if self.cursor_offset < text.len() {
            let next = text[self.cursor_offset..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.cursor_offset + i)
                .unwrap_or(text.len());
            self.buffer.delete(self.cursor_offset..next);
            self.highlighter.parse(self.buffer.text());
        }
    }

    /// Compute syntax highlights for a single line, with byte ranges relative
    /// to the start of that line's text (stripping trailing newline).
    /// Ranges are sorted and non-overlapping (required by GPUI's compute_runs).
    fn line_highlights(&self, line: usize) -> Vec<(Range<usize>, HighlightStyle)> {
        let byte_range = self.buffer.line_byte_range(line);
        let source = self.buffer.text();
        let line_text = self.buffer.line_text(line);

        let raw_highlights = self.highlighter.highlights(source, byte_range.clone());
        let line_start = byte_range.start;
        let line_end = line_start + line_text.len();

        let mut result: Vec<(Range<usize>, HighlightStyle)> = Vec::new();
        for (span_range, capture_name) in &raw_highlights {
            if let Some(style) = self.theme.get(capture_name) {
                let start = span_range.start.max(line_start) - line_start;
                let end = span_range.end.min(line_end) - line_start;
                if start < end {
                    result.push((start..end, style));
                }
            }
        }

        // Sort by start position, then by shorter range (more specific captures win)
        result.sort_by(|a, b| a.0.start.cmp(&b.0.start).then(a.0.len().cmp(&b.0.len())));

        // Remove overlaps: for each byte position, keep only the first (most specific) highlight
        let mut merged: Vec<(Range<usize>, HighlightStyle)> = Vec::new();
        let mut cursor = 0usize;
        for (range, style) in result {
            if range.start >= cursor {
                merged.push((range.clone(), style));
                cursor = range.end;
            } else if range.start < cursor && range.end > cursor {
                // Partially overlapping: trim the start
                merged.push((cursor..range.end, style));
                cursor = range.end;
            }
            // Fully overlapping ranges are skipped
        }
        merged
    }

}

impl Focusable for EditorView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for EditorView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let line_count = self.buffer.line_count();

        // Capture data needed by the uniform_list closure
        let line_highlights: Vec<(String, String, Vec<(Range<usize>, HighlightStyle)>, bool, usize)> =
            (0..line_count)
                .map(|line| {
                    let line_text = self.buffer.line_text(line).to_string();
                    let line_number = format!("{:>4}", line + 1);
                    let highlights = self.line_highlights(line);
                    let (cursor_row, cursor_col) = self.buffer.offset_to_point(self.cursor_offset);
                    let show_cursor = cursor_row == line;
                    (line_text, line_number, highlights, show_cursor, cursor_col)
                })
                .collect();

        let text_style = {
            let mut style = window.text_style();
            style.color = rgb(0xabb2bf).into();
            style.font_size = px(FONT_SIZE).into();
            style
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x282c34))
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, cx| {
                let keystroke = &event.keystroke;
                let handled = match keystroke.key.as_str() {
                    "backspace" => {
                        this.backspace();
                        true
                    }
                    "delete" => {
                        this.delete_forward();
                        true
                    }
                    "enter" => {
                        this.insert_newline();
                        true
                    }
                    "left" => {
                        this.move_cursor_left();
                        true
                    }
                    "right" => {
                        this.move_cursor_right();
                        true
                    }
                    "up" => {
                        this.move_cursor_up();
                        true
                    }
                    "down" => {
                        this.move_cursor_down();
                        true
                    }
                    _ => false,
                };
                if !handled {
                    if let Some(ref key_char) = keystroke.key_char {
                        if !keystroke.modifiers.control
                            && !keystroke.modifiers.alt
                            && !keystroke.modifiers.platform
                        {
                            this.insert_char(key_char);
                        }
                    }
                }
                cx.notify();
            }))
            .child(
                uniform_list("editor-lines", line_count, {
                    let text_style = text_style.clone();
                    move |range: Range<usize>, _window: &mut Window, _cx: &mut App| {
                        range
                            .map(|line| {
                                let (ref line_text, ref line_number, ref highlights, show_cursor, cursor_col) =
                                    line_highlights[line];

                                let styled_text = if line_text.is_empty() {
                                    StyledText::new(" ")
                                        .with_default_highlights(&text_style, Vec::new())
                                } else {
                                    StyledText::new(line_text.clone())
                                        .with_default_highlights(&text_style, highlights.clone())
                                };

                                div()
                                    .flex()
                                    .flex_row()
                                    .h(px(LINE_HEIGHT))
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
                                            .child(line_number.clone()),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .h(px(LINE_HEIGHT))
                                            .flex()
                                            .items_center()
                                            .relative()
                                            .when(show_cursor, |this| {
                                                let char_width = FONT_SIZE * 0.6;
                                                let cursor_x = cursor_col as f32 * char_width;
                                                this.child(
                                                    div()
                                                        .absolute()
                                                        .left(px(cursor_x))
                                                        .top(px(0.0))
                                                        .w(px(2.0))
                                                        .h(px(LINE_HEIGHT))
                                                        .bg(rgb(0x528bff)),
                                                )
                                            })
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
