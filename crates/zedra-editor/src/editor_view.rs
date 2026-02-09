use std::ops::Range;

use gpui::prelude::FluentBuilder;
use gpui::*;

use crate::buffer::Buffer;
use crate::highlighter::{Highlighter, Language};
use crate::theme::SyntaxTheme;

// Editor styling constants
const LINE_HEIGHT: f32 = 22.0;
const GUTTER_WIDTH: f32 = 52.0;
const FONT_SIZE: f32 = 13.0;
const HORIZONTAL_PADDING: f32 = 12.0;

// Colors (One Dark inspired)
const BG_PRIMARY: u32 = 0x1e2127;
const BG_EDITOR: u32 = 0x282c34;
const BG_GUTTER: u32 = 0x21252b;
const BG_CURRENT_LINE: u32 = 0x2c313c;
const BG_STATUS_BAR: u32 = 0x21252b;
const BORDER_SUBTLE: u32 = 0x3e4451;
const TEXT_PRIMARY: u32 = 0xabb2bf;
const TEXT_SECONDARY: u32 = 0x5c6370;
const TEXT_GUTTER: u32 = 0x4b5263;
const TEXT_GUTTER_ACTIVE: u32 = 0x737984;
const CURSOR_COLOR: u32 = 0x528bff;
const ACCENT_COLOR: u32 = 0x61afef;

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
    /// Create a new editor view with content and filename for language detection.
    pub fn new(content: String, filename: &str, cx: &mut App) -> Self {
        let mut highlighter = Highlighter::from_filename(filename);
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

    /// Create a new editor view with explicit language.
    pub fn with_language(content: String, language: Language, cx: &mut App) -> Self {
        let mut highlighter = Highlighter::new(language);
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

    /// Get the current language.
    pub fn language(&self) -> Language {
        self.highlighter.language()
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

    /// Compute syntax highlights for a single line.
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

    /// Render the status bar at the bottom.
    fn render_status_bar(&self, cx: &Context<Self>) -> impl IntoElement {
        let language = self.highlighter.language();
        let language_name: SharedString = language.display_name().into();
        let (cursor_row, cursor_col) = self.buffer.offset_to_point(self.cursor_offset);
        let position_text: SharedString =
            format!("Ln {}, Col {}", cursor_row + 1, cursor_col + 1).into();
        let line_count_text: SharedString = format!("{} lines", self.buffer.line_count()).into();

        // Language badge color
        let badge_color = match language {
            Language::Rust => rgb(0xdea584),
            Language::Python => rgb(0x3572a5),
            Language::Go => rgb(0x00add8),
            Language::JavaScript => rgb(0xf1e05a),
            Language::TypeScript | Language::Tsx => rgb(0x3178c6),
            Language::C => rgb(0x555555),
            Language::Cpp => rgb(0xf34b7d),
            Language::Css => rgb(0x563d7c),
            Language::Json => rgb(0x292929),
            Language::Yaml => rgb(0xcb171e),
            Language::Bash => rgb(0x89e051),
            Language::Markdown => rgb(0x083fa1),
            Language::PlainText => rgb(0x5c6370),
        };

        div()
            .h(px(28.0))
            .w_full()
            .bg(rgb(BG_STATUS_BAR))
            .border_t_1()
            .border_color(rgb(BORDER_SUBTLE))
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px(px(12.0))
            .child(
                // Left side: language badge
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .px(px(8.0))
                            .py(px(2.0))
                            .rounded(px(3.0))
                            .bg(badge_color)
                            .text_xs()
                            .text_color(rgb(0xffffff))
                            .child(language_name),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(TEXT_SECONDARY))
                            .child(line_count_text),
                    ),
            )
            .child(
                // Right side: cursor position
                div()
                    .text_xs()
                    .text_color(rgb(TEXT_SECONDARY))
                    .child(position_text),
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
        let line_count = self.buffer.line_count();
        let (cursor_row, _) = self.buffer.offset_to_point(self.cursor_offset);

        // Pre-compute line data
        let line_highlights: Vec<(
            String,
            String,
            Vec<(Range<usize>, HighlightStyle)>,
            bool,
            usize,
        )> = (0..line_count)
            .map(|line| {
                let line_text = self.buffer.line_text(line).to_string();
                let line_number = format!("{}", line + 1);
                let highlights = self.line_highlights(line);
                let (crow, cursor_col) = self.buffer.offset_to_point(self.cursor_offset);
                let show_cursor = crow == line;
                (line_text, line_number, highlights, show_cursor, cursor_col)
            })
            .collect();

        let text_style = {
            let mut style = window.text_style();
            style.color = rgb(TEXT_PRIMARY).into();
            style.font_size = px(FONT_SIZE).into();
            style
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(BG_PRIMARY))
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
            // Editor content area
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_row()
                    .overflow_hidden()
                    // Gutter
                    .child(
                        div()
                            .w(px(GUTTER_WIDTH))
                            .h_full()
                            .bg(rgb(BG_GUTTER))
                            .border_r_1()
                            .border_color(rgb(BORDER_SUBTLE)),
                    )
                    // Code area with uniform list
                    .child(
                        div().flex_1().bg(rgb(BG_EDITOR)).child(
                            uniform_list("editor-lines", line_count, {
                                let text_style = text_style.clone();
                                move |range: Range<usize>, _window: &mut Window, _cx: &mut App| {
                                    range
                                        .map(|line| {
                                            let (
                                                ref line_text,
                                                ref line_number,
                                                ref highlights,
                                                show_cursor,
                                                cursor_col,
                                            ) = line_highlights[line];

                                            let is_current_line = line == cursor_row;

                                            let styled_text = if line_text.is_empty() {
                                                StyledText::new(" ").with_default_highlights(
                                                    &text_style,
                                                    Vec::new(),
                                                )
                                            } else {
                                                StyledText::new(line_text.clone())
                                                    .with_default_highlights(
                                                        &text_style,
                                                        highlights.clone(),
                                                    )
                                            };

                                            // Line container
                                            div()
                                                .w_full()
                                                .h(px(LINE_HEIGHT))
                                                .flex()
                                                .flex_row()
                                                // Current line highlight
                                                .when(is_current_line, |el| {
                                                    el.bg(rgb(BG_CURRENT_LINE))
                                                })
                                                // Gutter (line numbers) - positioned absolutely over the gutter column
                                                .child(
                                                    div()
                                                        .w(px(GUTTER_WIDTH))
                                                        .h(px(LINE_HEIGHT))
                                                        .flex()
                                                        .items_center()
                                                        .justify_end()
                                                        .pr(px(12.0))
                                                        .text_color(if is_current_line {
                                                            rgb(TEXT_GUTTER_ACTIVE)
                                                        } else {
                                                            rgb(TEXT_GUTTER)
                                                        })
                                                        .text_size(px(FONT_SIZE - 1.0))
                                                        .child(line_number.clone()),
                                                )
                                                // Code content
                                                .child(
                                                    div()
                                                        .flex_1()
                                                        .h(px(LINE_HEIGHT))
                                                        .pl(px(HORIZONTAL_PADDING))
                                                        .flex()
                                                        .items_center()
                                                        .relative()
                                                        // Cursor
                                                        .when(show_cursor, |el| {
                                                            let char_width = FONT_SIZE * 0.602;
                                                            let cursor_x =
                                                                cursor_col as f32 * char_width;
                                                            el.child(
                                                                div()
                                                                    .absolute()
                                                                    .left(px(HORIZONTAL_PADDING
                                                                        + cursor_x))
                                                                    .top(px(2.0))
                                                                    .w(px(2.0))
                                                                    .h(px(LINE_HEIGHT - 4.0))
                                                                    .rounded(px(1.0))
                                                                    .bg(rgb(CURSOR_COLOR)),
                                                            )
                                                        })
                                                        .child(styled_text),
                                                )
                                        })
                                        .collect()
                                }
                            })
                            .track_scroll(&self.scroll_handle)
                            .size_full(),
                        ),
                    ),
            )
            // Status bar
            .child(self.render_status_bar(cx))
    }
}
