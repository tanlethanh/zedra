use std::ops::Range;
use std::rc::Rc;

use gpui::prelude::FluentBuilder;
use gpui::*;

use super::syntax_highlighter::{Highlighter, Language};
use super::syntax_theme::SyntaxTheme;
use super::text_buffer::Buffer;

use crate::platform_bridge;
use crate::theme;

const LINE_HEIGHT: f32 = theme::EDITOR_LINE_HEIGHT;
const GUTTER_WIDTH: f32 = theme::EDITOR_GUTTER_WIDTH;
const FONT_SIZE: f32 = theme::EDITOR_FONT_SIZE;
const GUTTER_FONT_SIZE: f32 = theme::EDITOR_GUTTER_FONT_SIZE;
const BOTTOM_INSET_MIN: f32 = 100.0;

/// Cached per-line data (text, line number, syntax highlights).
/// Recomputed only when the buffer content changes, NOT on every scroll frame.
struct CachedLine {
    text: String,
    number: String,
    highlights: Vec<(Range<usize>, HighlightStyle)>,
}

/// A code editor view with syntax highlighting and virtual scrolling.
pub struct EditorView {
    buffer: Buffer,
    highlighter: Highlighter,
    theme: SyntaxTheme,
    cursor_offset: usize,
    scroll_handle: UniformListScrollHandle,
    focus_handle: FocusHandle,
    /// Cached line data shared with the uniform_list closure via Rc.
    /// Only rebuilt when the buffer content changes.
    cached_lines: Rc<Vec<CachedLine>>,
    /// Whether cached_lines needs rebuilding.
    lines_dirty: bool,
    /// Horizontal scroll offset in logical pixels.
    h_scroll_offset: f32,
    /// Length (in chars) of the longest line — used to cap horizontal scroll.
    max_line_chars: usize,
    /// True once a gesture has been committed to horizontal scroll.
    /// Stays true until a clearly vertical event overrides it.
    h_scroll_active: bool,
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
            theme: SyntaxTheme::default_dark(),
            cursor_offset: 0,
            scroll_handle: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            cached_lines: Rc::new(Vec::new()),
            lines_dirty: true,
            h_scroll_offset: 0.0,
            max_line_chars: 0,
            h_scroll_active: false,
        }
    }

    /// Replace the entire buffer content (e.g. when loading a remote file).
    pub fn set_content(&mut self, content: String) {
        self.buffer.set_text(content);
        self.highlighter.parse(self.buffer.text());
        self.cursor_offset = 0;
        self.lines_dirty = true;
        self.h_scroll_offset = 0.0;
        self.h_scroll_active = false;
    }

    pub fn language(&self) -> Language {
        self.highlighter.language()
    }

    /// Rebuild the cached line data from the buffer and highlighter.
    fn rebuild_line_cache(&mut self) {
        let line_count = self.buffer.line_count();
        let lines: Vec<CachedLine> = (0..line_count)
            .map(|line| CachedLine {
                text: self.buffer.line_text(line).to_string(),
                number: format!("{:>4}", line + 1),
                highlights: self.line_highlights(line),
            })
            .collect();
        self.max_line_chars = lines.iter().map(|l| l.text.len()).max().unwrap_or(0);
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

        super::merge_highlights(result)
    }
}

impl Focusable for EditorView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for EditorView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Rebuild line cache only when buffer content changed.
        // During scroll, this is skipped entirely.
        if self.lines_dirty {
            self.rebuild_line_cache();
        }

        let line_count = self.cached_lines.len();
        let (cursor_row, cursor_col) = self.buffer.offset_to_point(self.cursor_offset);
        let cached_lines = self.cached_lines.clone();
        let bottom_inset = f32::max(platform_bridge::home_indicator_inset(), BOTTOM_INSET_MIN);
        // uniform_list forces all items to the same height (item 0's measured height = LINE_HEIGHT).
        // To get `bottom_inset` worth of scroll space we need enough extra items to cover it.
        let extra_items = (bottom_inset / LINE_HEIGHT).ceil() as usize;
        let h_scroll_offset = self.h_scroll_offset;
        // Captured before the scroll event fires; restored while h_scroll_active so
        // the vertical position doesn't drift during a horizontal swipe.
        let scroll_y_lock = self.scroll_handle.0.borrow().base_handle.offset().y;

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
            .bg(rgb(0x0e0c0c))
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
            .on_scroll_wheel(cx.listener(move |this, event: &ScrollWheelEvent, _window, cx| {
                let (delta_x, delta_y) = match event.delta {
                    ScrollDelta::Pixels(p) => (f32::from(p.x), f32::from(p.y)),
                    ScrollDelta::Lines(l) => (l.x * 20.0, l.y * 20.0),
                };
                // Enter H-scroll mode: strict threshold (2.5×, 5 px min) to commit.
                // Exit H-scroll mode: a strongly vertical event (3× vertical) overrides.
                // While locked, accept any event with non-zero horizontal delta so a
                // drifting finger doesn't break the scroll mid-gesture.
                if delta_y.abs() > delta_x.abs() * 3.0 {
                    this.h_scroll_active = false;
                } else if delta_x.abs() > delta_y.abs() * 2.5 && delta_x.abs() > 5.0 {
                    this.h_scroll_active = true;
                }
                if this.h_scroll_active && delta_x.abs() > 0.1 {
                    let char_width = FONT_SIZE * 0.6;
                    let max_offset = (this.max_line_chars as f32 * char_width).max(0.0);
                    this.h_scroll_offset = (this.h_scroll_offset - delta_x).clamp(0.0, max_offset);
                    // Undo any vertical drift: the uniform_list overflow scroll already fired
                    // (bubble phase, inner first) and may have nudged y. Restore it to the
                    // value captured at the start of this render so vertical position is locked
                    // for the duration of the horizontal gesture.
                    this.scroll_handle
                        .0
                        .borrow()
                        .base_handle
                        .set_offset(point(px(0.0), scroll_y_lock));
                    cx.notify();
                }
            }))
            .child(
                uniform_list("editor-lines", line_count + extra_items, {
                    let text_style = text_style.clone();
                    move |range: Range<usize>, _window: &mut Window, _cx: &mut App| {
                        range
                            .map(|line| -> AnyElement {
                                // Trailing spacer items for bottom safe-area clearance.
                                // Each renders at LINE_HEIGHT (uniform_list enforces uniform height),
                                // so extra_items * LINE_HEIGHT >= bottom_inset.
                                if line >= line_count {
                                    return div().h(px(LINE_HEIGHT)).into_any_element();
                                }

                                let cached = &cached_lines[line];
                                let show_cursor = cursor_row == line;

                                let styled_text = if cached.text.is_empty() {
                                    StyledText::new(" ")
                                        .with_default_highlights(&text_style, Vec::new())
                                } else {
                                    StyledText::new(cached.text.clone()).with_default_highlights(
                                        &text_style,
                                        cached.highlights.clone(),
                                    )
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
                                            .text_color(hsla(0.0, 0.0, 0.83, 0.3))
                                            .text_size(px(GUTTER_FONT_SIZE))
                                            .child(cached.number.clone()),
                                    )
                                    .child(
                                        // Clip container — stays within the row's flex width.
                                        div()
                                            .flex_1()
                                            .h(px(LINE_HEIGHT))
                                            .overflow_hidden()
                                            .relative()
                                            .child(
                                                // Scrollable content — shifted left by h_scroll_offset.
                                                div()
                                                    .absolute()
                                                    .top(px(0.0))
                                                    .left(px(-h_scroll_offset))
                                                    .h(px(LINE_HEIGHT))
                                                    .flex()
                                                    .items_center()
                                                    .text_size(px(FONT_SIZE))
                                                    .relative()
                                                    .when(show_cursor, |this| {
                                                        let char_width = FONT_SIZE * 0.6;
                                                        let cursor_x =
                                                            cursor_col as f32 * char_width;
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
