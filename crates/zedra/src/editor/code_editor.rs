use std::ops::Range;
use std::rc::Rc;

use gpui::*;

use super::syntax_highlighter::{Highlighter, Language};
use super::syntax_theme::SyntaxTheme;
use super::text_buffer::Buffer;

use crate::fonts;
use crate::platform_bridge;
use crate::theme;
use crate::workspace_action::AddSelectionToChat;

const LINE_HEIGHT: f32 = theme::EDITOR_LINE_HEIGHT;
const GUTTER_WIDTH: f32 = theme::EDITOR_GUTTER_WIDTH;
const FONT_SIZE: f32 = theme::EDITOR_FONT_SIZE;
const GUTTER_FONT_SIZE: f32 = theme::EDITOR_GUTTER_FONT_SIZE;
const BOTTOM_INSET_MIN: f32 = 100.0;
const CODE_TEXT_COLOR: u32 = 0xabb2bf;
const PENDING_SYNTAX_TEXT_COLOR: u32 = 0x7b8494;
pub const CODE_EDITOR_SELECTION_AREA_ID: &str = "code-editor-selection";

type LineHighlights = Vec<(Range<usize>, HighlightStyle)>;

/// Cached per-line data (text, line number).
/// Recomputed only when the buffer content changes, NOT on every scroll frame.
struct CachedLine {
    text: String,
    number: String,
}

pub struct ParsedEditorSyntax {
    highlighter: Highlighter,
    line_highlights: Vec<LineHighlights>,
}

impl ParsedEditorSyntax {
    pub fn build(filename: &str, content: String) -> Self {
        let mut highlighter = Highlighter::from_filename(filename);
        highlighter.parse(&content);
        let theme = SyntaxTheme::default_dark();
        let buffer = Buffer::new(content);
        let line_highlights = build_line_highlights(&buffer, &highlighter, &theme);
        Self {
            highlighter,
            line_highlights,
        }
    }
}

/// A code editor view with syntax highlighting and virtual scrolling.
pub struct EditorView {
    buffer: Buffer,
    highlighter: Rc<Highlighter>,
    theme: SyntaxTheme,
    scroll_handle: UniformListScrollHandle,
    /// Cached line data shared with the uniform_list closure via Rc.
    /// Only rebuilt when the buffer content changes.
    cached_lines: Rc<Vec<CachedLine>>,
    /// Syntax highlight data matching `cached_lines` by index. Built off the UI thread for
    /// remote file opens, then swapped in without rebuilding the line text cache.
    cached_line_highlights: Rc<Vec<LineHighlights>>,
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
    /// Create with automatic language detection from filename.
    pub fn new(_cx: &mut App) -> Self {
        Self::build(
            "".to_string(),
            Highlighter::from_language(Language::PlainText),
        )
    }

    pub fn new_with_content(filename: &str, content: String, _cx: &mut App) -> Self {
        let mut highlighter = Highlighter::from_filename(filename);
        highlighter.parse(&content);
        Self::build(content, highlighter)
    }

    fn build(content: String, highlighter: Highlighter) -> Self {
        Self {
            buffer: Buffer::new(content),
            highlighter: Rc::new(highlighter),
            theme: SyntaxTheme::default_dark(),
            scroll_handle: UniformListScrollHandle::new(),
            cached_lines: Rc::new(Vec::new()),
            cached_line_highlights: Rc::new(Vec::new()),
            lines_dirty: true,
            h_scroll_offset: 0.0,
            max_line_chars: 0,
            h_scroll_active: false,
        }
    }

    /// Replace the entire buffer content (e.g. when loading a remote file).
    /// The language is detected from the filename.
    pub fn set_content(&mut self, filename: &str, content: String) {
        self.highlighter = Rc::new(Highlighter::from_filename(filename));
        self.buffer.set_text(content);
        self.cached_line_highlights = Rc::new(Vec::new());
        self.lines_dirty = true;
        self.h_scroll_offset = 0.0;
        self.h_scroll_active = false;
    }

    pub fn apply_parsed_syntax(&mut self, parsed: ParsedEditorSyntax) {
        let ParsedEditorSyntax {
            highlighter,
            line_highlights,
        } = parsed;
        self.highlighter = Rc::new(highlighter);
        self.cached_line_highlights = Rc::new(line_highlights);
    }

    pub fn language(&self) -> Language {
        self.highlighter.language()
    }

    pub fn line_range_for_selection(&self, range_utf16: Range<usize>) -> Option<(u32, u32)> {
        let lines = self
            .cached_lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>();
        line_range_for_selection_lines(&lines, range_utf16)
    }

    /// Rebuild the cached line data from the buffer text only.
    fn rebuild_line_cache(&mut self) {
        let line_count = self.buffer.line_count();
        let lines: Vec<CachedLine> = (0..line_count)
            .map(|line| CachedLine {
                text: self.buffer.line_text(line).to_string(),
                number: format!("{:>4}", line + 1),
            })
            .collect();
        self.max_line_chars = lines.iter().map(|l| l.text.len()).max().unwrap_or(0);
        if self.cached_line_highlights.len() != line_count {
            self.cached_line_highlights = if self.highlighter.is_waiting_for_syntax() {
                Rc::new(vec![Vec::new(); line_count])
            } else {
                Rc::new(build_line_highlights(
                    &self.buffer,
                    &self.highlighter,
                    &self.theme,
                ))
            };
        }
        self.cached_lines = Rc::new(lines);
        self.lines_dirty = false;
    }
}

fn code_text_color_for_highlighter(highlighter: &Highlighter) -> u32 {
    if highlighter.is_waiting_for_syntax() {
        PENDING_SYNTAX_TEXT_COLOR
    } else {
        CODE_TEXT_COLOR
    }
}

fn build_line_highlights(
    buffer: &Buffer,
    highlighter: &Highlighter,
    theme: &SyntaxTheme,
) -> Vec<LineHighlights> {
    (0..buffer.line_count())
        .map(|line| {
            let line_text = buffer.line_text(line);
            line_highlights_for_source(
                buffer.text(),
                highlighter,
                theme,
                buffer.line_byte_range(line),
                line_text.len(),
            )
        })
        .collect()
}

fn line_highlights_for_source(
    source: &str,
    highlighter: &Highlighter,
    theme: &SyntaxTheme,
    byte_range: Range<usize>,
    line_text_len: usize,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let raw_highlights = highlighter.highlights(source, byte_range.clone());
    let line_start = byte_range.start;
    let line_end = line_start + line_text_len;

    let mut result: Vec<(Range<usize>, HighlightStyle)> = Vec::new();
    for (span_range, capture_name) in &raw_highlights {
        if let Some(style) = theme.get(capture_name) {
            let start = span_range.start.max(line_start) - line_start;
            let end = span_range.end.min(line_end) - line_start;
            if start < end {
                result.push((start..end, style));
            }
        }
    }

    super::merge_highlights(result)
}

fn selectable_line_len_utf16(line: &str) -> usize {
    if line.is_empty() {
        " ".encode_utf16().count()
    } else {
        line.encode_utf16().count()
    }
}

fn line_range_for_selection_lines(lines: &[&str], range_utf16: Range<usize>) -> Option<(u32, u32)> {
    if lines.is_empty() || range_utf16.is_empty() {
        return None;
    }

    let selection_start = range_utf16.start;
    let selection_end = range_utf16.end.saturating_sub(1);
    let mut offset = 0;
    let mut start_line = None;

    for (line_index, line) in lines.iter().enumerate() {
        let content_len = selectable_line_len_utf16(line);
        let separator_len = usize::from(line_index + 1 < lines.len());
        let segment_end = offset + content_len + separator_len;
        let line_number = line_index as u32 + 1;

        if start_line.is_none() && selection_start < segment_end {
            start_line = Some(line_number);
        }

        if selection_end < segment_end {
            return start_line.map(|start| (start, line_number));
        }

        offset = segment_end;
    }

    start_line.map(|start| (start, lines.len() as u32))
}

impl Render for EditorView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Rebuild line cache only when buffer content changed.
        // During scroll, this is skipped entirely.
        if self.lines_dirty {
            self.rebuild_line_cache();
        }

        let line_count = self.cached_lines.len();
        let cached_lines = self.cached_lines.clone();
        let cached_line_highlights = self.cached_line_highlights.clone();
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
            style.color = rgb(code_text_color_for_highlighter(&self.highlighter)).into();
            style.font_family = fonts::MONO_FONT_FAMILY.into();
            style.font_size = px(FONT_SIZE).into();
            style
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x0e0c0c))
            .font_family(fonts::MONO_FONT_FAMILY)
            .on_scroll_wheel(
                cx.listener(move |this, event: &ScrollWheelEvent, _window, cx| {
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
                        this.h_scroll_offset =
                            (this.h_scroll_offset - delta_x).clamp(0.0, max_offset);
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
                }),
            )
            .child(
                selection_area(
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
                                    let highlights = cached_line_highlights
                                        .get(line)
                                        .cloned()
                                        .unwrap_or_default();

                                    let styled_text = if cached.text.is_empty() {
                                        StyledText::new(" ")
                                            .with_default_highlights(&text_style, Vec::new())
                                    } else {
                                        StyledText::new(cached.text.clone())
                                            .with_default_highlights(&text_style, highlights)
                                    }
                                    .selectable()
                                    .selection_order(line as u64)
                                    .selection_separator_after(if line + 1 < line_count {
                                        "\n"
                                    } else {
                                        ""
                                    });

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
                .id(CODE_EDITOR_SELECTION_AREA_ID)
                .action("Add to Chat", AddSelectionToChat),
            )
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use super::{
        CODE_TEXT_COLOR, EditorView, PENDING_SYNTAX_TEXT_COLOR, ParsedEditorSyntax,
        code_text_color_for_highlighter, line_range_for_selection_lines,
    };
    use crate::editor::syntax_highlighter::{Highlighter, Language};

    #[test]
    fn maps_utf16_selection_to_code_lines() {
        let lines = ["alpha", "beta", "gamma"];

        assert_eq!(line_range_for_selection_lines(&lines, 0..5), Some((1, 1)));
        assert_eq!(line_range_for_selection_lines(&lines, 2..8), Some((1, 2)));
        assert_eq!(line_range_for_selection_lines(&lines, 8..16), Some((2, 3)));
    }

    #[test]
    fn maps_empty_rendered_lines_to_their_own_line_number() {
        let lines = ["alpha", "", "gamma"];

        assert_eq!(line_range_for_selection_lines(&lines, 6..7), Some((2, 2)));
        assert_eq!(line_range_for_selection_lines(&lines, 6..13), Some((2, 3)));
    }

    #[test]
    fn applies_parsed_syntax_to_cached_line_highlights() {
        let content = "fn main() {\n    let value = 1;\n}\n".to_string();
        let mut editor = EditorView::build(content.clone(), Highlighter::from_filename("main.rs"));
        editor.rebuild_line_cache();

        let original_lines = editor.cached_lines.clone();
        assert!(editor.cached_line_highlights[0].is_empty());

        let parsed = ParsedEditorSyntax::build("main.rs", content);
        assert_eq!(parsed.line_highlights.len(), editor.cached_lines.len());
        let parsed_first_line_highlights = parsed.line_highlights[0].clone();
        assert!(!parsed_first_line_highlights.is_empty());
        editor.apply_parsed_syntax(parsed);

        assert!(Rc::ptr_eq(&original_lines, &editor.cached_lines));
        assert_eq!(
            editor.cached_line_highlights[0],
            parsed_first_line_highlights
        );
        assert!(
            editor.cached_line_highlights[0]
                .iter()
                .all(|(range, _)| range.end <= editor.cached_lines[0].text.len())
        );
    }

    #[test]
    fn dims_code_text_only_while_syntax_is_pending() {
        let pending = Highlighter::from_filename("main.rs");
        assert_eq!(
            code_text_color_for_highlighter(&pending),
            PENDING_SYNTAX_TEXT_COLOR
        );

        let plain_text = Highlighter::from_language(Language::PlainText);
        assert_eq!(
            code_text_color_for_highlighter(&plain_text),
            CODE_TEXT_COLOR
        );

        let mut parsed = Highlighter::from_filename("main.rs");
        parsed.parse("fn main() {}\n");
        assert_eq!(code_text_color_for_highlighter(&parsed), CODE_TEXT_COLOR);
    }
}
