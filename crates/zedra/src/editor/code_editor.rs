use std::ops::Range;
use std::rc::Rc;

use gpui::prelude::FluentBuilder;
use gpui::*;

use super::syntax_highlighter::{Highlighter, Language};
use super::syntax_theme::SyntaxTheme;
use super::text_buffer::Buffer;

use crate::theme::{EditorColors, LanguageColors, EDITOR_FONT_SIZE, EDITOR_GUTTER_WIDTH,
    EDITOR_LINE_HEIGHT};

/// Cached per-line data. Recomputed only when the buffer changes, not on scroll.
struct CachedLine {
    text: String,
    number: String,
    highlights: Vec<(Range<usize>, HighlightStyle)>,
}

/// A code editor view with syntax highlighting and virtual scrolling.
pub struct EditorView {
    buffer: Buffer,
    highlighter: Highlighter,
    syntax_theme: SyntaxTheme,
    colors: EditorColors,
    cursor_offset: usize,
    scroll_handle: UniformListScrollHandle,
    focus_handle: FocusHandle,
    cached_lines: Rc<Vec<CachedLine>>,
    lines_dirty: bool,
}

impl EditorView {
    pub fn new(content: String, cx: &mut App) -> Self {
        let mut highlighter = Highlighter::rust();
        highlighter.parse(&content);
        Self::build(content, highlighter, cx)
    }

    /// Create with automatic language detection from filename.
    pub fn with_filename(content: String, filename: &str, cx: &mut App) -> Self {
        let mut highlighter = Highlighter::from_filename(filename);
        highlighter.parse(&content);
        Self::build(content, highlighter, cx)
    }

    fn build(content: String, highlighter: Highlighter, cx: &mut App) -> Self {
        Self {
            buffer: Buffer::new(content),
            highlighter,
            syntax_theme: SyntaxTheme::default(),
            colors: EditorColors::default(),
            cursor_offset: 0,
            scroll_handle: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            cached_lines: Rc::new(Vec::new()),
            lines_dirty: true,
        }
    }

    /// Replace the entire buffer content (e.g. when loading a remote file).
    pub fn set_content(&mut self, content: String) {
        self.buffer.set_text(content);
        self.highlighter.parse(self.buffer.text());
        self.cursor_offset = 0;
        self.lines_dirty = true;
    }

    pub fn language(&self) -> Language {
        self.highlighter.language()
    }

    fn rebuild_line_cache(&mut self) {
        let lines: Vec<CachedLine> = (0..self.buffer.line_count())
            .map(|line| CachedLine {
                text: self.buffer.line_text(line).to_string(),
                number: format!("{}", line + 1),
                highlights: self.line_highlights(line),
            })
            .collect();
        log::info!(
            "[PERF] editor: rebuilt cache, {} lines, {} chars",
            lines.len(),
            lines.iter().map(|l| l.text.len()).sum::<usize>()
        );
        self.cached_lines = Rc::new(lines);
        self.lines_dirty = false;
    }

    fn move_cursor_left(&mut self) {
        if self.cursor_offset > 0 {
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
        self.lines_dirty = true;
    }

    fn insert_newline(&mut self) {
        self.buffer.insert(self.cursor_offset, "\n");
        self.cursor_offset += 1;
        self.highlighter.parse(self.buffer.text());
        self.lines_dirty = true;
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
            self.lines_dirty = true;
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
            self.lines_dirty = true;
        }
    }

    /// Compute syntax highlights for a single line. Ranges are sorted and
    /// non-overlapping (required by GPUI's compute_runs).
    fn line_highlights(&self, line: usize) -> Vec<(Range<usize>, HighlightStyle)> {
        let byte_range = self.buffer.line_byte_range(line);
        let source = self.buffer.text();
        let line_text = self.buffer.line_text(line);
        let line_start = byte_range.start;
        let line_end = line_start + line_text.len();

        let mut result: Vec<(Range<usize>, HighlightStyle)> = self
            .highlighter
            .highlights(source, byte_range)
            .iter()
            .filter_map(|(span, name)| {
                self.syntax_theme.get(name).map(|style| {
                    let start = span.start.max(line_start) - line_start;
                    let end = span.end.min(line_end) - line_start;
                    (start..end, style)
                })
            })
            .filter(|(r, _)| !r.is_empty())
            .collect();

        result.sort_by(|a, b| a.0.start.cmp(&b.0.start).then(a.0.len().cmp(&b.0.len())));

        // Remove overlaps — keep first (most specific) highlight at each position.
        let mut merged: Vec<(Range<usize>, HighlightStyle)> = Vec::new();
        let mut cursor = 0usize;
        for (range, style) in result {
            if range.start >= cursor {
                cursor = range.end;
                merged.push((range, style));
            } else if range.end > cursor {
                merged.push((cursor..range.end, style));
                cursor = range.end;
            }
        }
        merged
    }

    fn render_status_bar(&self) -> impl IntoElement {
        let language = self.highlighter.language();
        let (cursor_row, cursor_col) = self.buffer.offset_to_point(self.cursor_offset);
        let lang_name: SharedString = language.display_name().into();
        let position: SharedString =
            format!("Ln {}, Col {}", cursor_row + 1, cursor_col + 1).into();
        let line_count: SharedString = format!("{} lines", self.buffer.line_count()).into();

        let c = &self.colors;
        let badge_color = LanguageColors::for_language(language.display_name());

        div()
            .h(px(22.0))
            .w_full()
            .bg(c.status_bar_bg)
            .border_t_1()
            .border_color(c.border_subtle)
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px(px(10.0))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .px(px(5.0))
                            .py(px(1.0))
                            .rounded(px(3.0))
                            .bg(badge_color)
                            .text_color(gpui::rgb(0xffffff))
                            .text_size(px(9.0))
                            .child(lang_name),
                    )
                    .child(
                        div()
                            .text_size(px(9.0))
                            .text_color(c.status_bar_text)
                            .child(line_count),
                    ),
            )
            .child(
                div()
                    .text_size(px(9.0))
                    .text_color(c.status_bar_text)
                    .child(position),
            )
    }
}

impl Focusable for EditorView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for EditorView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.lines_dirty {
            self.rebuild_line_cache();
        }

        let line_count = self.cached_lines.len();
        let (cursor_row, cursor_col) = self.buffer.offset_to_point(self.cursor_offset);
        let cached_lines = self.cached_lines.clone();
        let c = &self.colors;

        let text_style = {
            let mut style = window.text_style();
            style.color = c.text_primary;
            style.font_size = px(EDITOR_FONT_SIZE).into();
            style
        };

        let bg_current_line = c.bg_current_line;
        let bg_gutter = c.bg_gutter;
        let border_subtle = c.border_subtle;
        let text_gutter = c.text_gutter;
        let text_gutter_active = c.text_gutter_active;
        let cursor_color = c.cursor;

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(c.bg)
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, cx| {
                let k = &event.keystroke;
                let handled = match k.key.as_str() {
                    "backspace" => { this.backspace(); true }
                    "delete"    => { this.delete_forward(); true }
                    "enter"     => { this.insert_newline(); true }
                    "left"      => { this.move_cursor_left(); true }
                    "right"     => { this.move_cursor_right(); true }
                    "up"        => { this.move_cursor_up(); true }
                    "down"      => { this.move_cursor_down(); true }
                    _ => false,
                };
                if !handled {
                    if let Some(ref ch) = k.key_char {
                        if !k.modifiers.control && !k.modifiers.alt && !k.modifiers.platform {
                            this.insert_char(ch);
                        }
                    }
                }
                cx.notify();
            }))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_row()
                    .overflow_hidden()
                    // Gutter separator panel
                    .child(
                        div()
                            .w(px(EDITOR_GUTTER_WIDTH))
                            .h_full()
                            .bg(bg_gutter)
                            .border_r_1()
                            .border_color(border_subtle),
                    )
                    // Scrollable code area
                    .child(div().flex_1().bg(c.bg).child(
                        uniform_list("editor-lines", line_count, {
                            let text_style = text_style.clone();
                            move |range: Range<usize>, _window: &mut Window, _cx: &mut App| {
                                range
                                    .map(|line| {
                                        let cached = &cached_lines[line];
                                        let is_current = line == cursor_row;

                                        let styled = if cached.text.is_empty() {
                                            StyledText::new(" ")
                                                .with_default_highlights(&text_style, vec![])
                                        } else {
                                            StyledText::new(cached.text.clone())
                                                .with_default_highlights(
                                                    &text_style,
                                                    cached.highlights.clone(),
                                                )
                                        };

                                        div()
                                            .w_full()
                                            .h(px(EDITOR_LINE_HEIGHT))
                                            .flex()
                                            .flex_row()
                                            .when(is_current, |el| el.bg(bg_current_line))
                                            // Line number
                                            .child(
                                                div()
                                                    .w(px(EDITOR_GUTTER_WIDTH))
                                                    .h(px(EDITOR_LINE_HEIGHT))
                                                    .flex()
                                                    .items_center()
                                                    .justify_end()
                                                    .pr(px(8.0))
                                                    .text_color(if is_current {
                                                        text_gutter_active
                                                    } else {
                                                        text_gutter
                                                    })
                                                    .text_size(px(EDITOR_FONT_SIZE - 1.0))
                                                    .child(cached.number.clone()),
                                            )
                                            // Code content
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .h(px(EDITOR_LINE_HEIGHT))
                                                    .pl(px(4.0))
                                                    .flex()
                                                    .items_center()
                                                    .relative()
                                                    .when(is_current, |el| {
                                                        let x = cursor_col as f32
                                                            * (EDITOR_FONT_SIZE * 0.6);
                                                        el.child(
                                                            div()
                                                                .absolute()
                                                                .left(px(4.0 + x))
                                                                .top(px(2.0))
                                                                .w(px(2.0))
                                                                .h(px(EDITOR_LINE_HEIGHT - 4.0))
                                                                .rounded(px(1.0))
                                                                .bg(cursor_color),
                                                        )
                                                    })
                                                    .child(styled),
                                            )
                                    })
                                    .collect()
                            }
                        })
                        .track_scroll(&self.scroll_handle)
                        .size_full(),
                    )),
            )
            .child(self.render_status_bar())
    }
}
