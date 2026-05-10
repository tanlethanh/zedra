use std::ops::Range;

use alacritty_terminal::term::cell::Flags as CellFlags;
use gpui::{Bounds, Pixels, Point, point, px, size};
use smallvec::SmallVec;

use crate::terminal::{IndexedCell, TerminalContent, is_blank};

#[derive(Clone, Debug)]
pub struct TerminalSelectionDocument {
    text: String,
    len_utf16: usize,
    chars: Vec<TerminalSelectionChar>,
    lines: Vec<TerminalSelectionLine>,
}

#[derive(Clone, Debug)]
struct TerminalSelectionChar {
    start_utf16: usize,
    end_utf16: usize,
    byte_start: usize,
    ch: char,
}

#[derive(Clone, Debug)]
struct TerminalSelectionLine {
    start_utf16: usize,
    content_end_utf16: usize,
    separator_end_utf16: usize,
    cells: Vec<TerminalSelectionCell>,
    x: Pixels,
    y: Pixels,
    line_height: Pixels,
}

#[derive(Clone, Debug)]
struct TerminalSelectionCell {
    start_utf16: usize,
    end_utf16: usize,
    bounds: Bounds<Pixels>,
}

impl TerminalSelectionDocument {
    pub fn empty() -> Self {
        Self {
            text: String::new(),
            len_utf16: 0,
            chars: Vec::new(),
            lines: Vec::new(),
        }
    }

    pub fn has_selectable_text(content: &TerminalContent) -> bool {
        content.cells.iter().any(|cell| {
            let visible_row = cell.point.line.0 + content.display_offset as i32;
            visible_row >= 0
                && visible_row < content.grid_rows as i32
                && selectable_nonblank_cell(cell)
        })
    }

    pub fn new(
        content: &TerminalContent,
        origin: Point<Pixels>,
        cell_width: Pixels,
        line_height: Pixels,
    ) -> Self {
        let mut visible_rows = vec![Vec::new(); content.grid_rows];
        for cell in &content.cells {
            let visible_row = cell.point.line.0 + content.display_offset as i32;
            if visible_row < 0 || visible_row >= content.grid_rows as i32 {
                continue;
            }
            visible_rows[visible_row as usize].push(cell);
        }

        let first_content_row = visible_rows
            .iter()
            .position(|row| row.iter().any(|cell| selectable_nonblank_cell(cell)));
        let last_content_row = visible_rows
            .iter()
            .rposition(|row| row.iter().any(|cell| selectable_nonblank_cell(cell)));

        let (Some(first_content_row), Some(last_content_row)) =
            (first_content_row, last_content_row)
        else {
            return Self::empty();
        };

        let mut text = String::new();
        let mut len_utf16 = 0;
        let mut chars = Vec::new();
        let mut lines = Vec::new();

        for row_idx in first_content_row..=last_content_row {
            let row = &mut visible_rows[row_idx];
            row.sort_by_key(|cell| cell.point.column.0);

            let line_start = len_utf16;
            let last_nonblank_col = row
                .iter()
                .filter(|cell| is_selectable_cell(cell))
                .filter(|cell| !is_blank(cell))
                .map(|cell| cell.point.column.0)
                .max();

            let y = origin.y + line_height * row_idx as f32;
            let mut cells = Vec::new();

            if let Some(last_nonblank_col) = last_nonblank_col {
                let mut col = 0;
                while col <= last_nonblank_col {
                    let indexed_cell = row
                        .iter()
                        .find(|cell| cell.point.column.0 == col && is_selectable_cell(cell));
                    let (ch, width_cells) = indexed_cell.map_or((' ', 1), |cell| {
                        let width_cells = if cell.cell.flags.contains(CellFlags::WIDE_CHAR) {
                            2
                        } else {
                            1
                        };
                        (visible_cell_char(cell), width_cells)
                    });

                    let start_utf16 = len_utf16;
                    let byte_start = text.len();
                    text.push(ch);
                    len_utf16 += ch.len_utf16();
                    chars.push(TerminalSelectionChar {
                        start_utf16,
                        end_utf16: len_utf16,
                        byte_start,
                        ch,
                    });
                    let bounds = Bounds {
                        origin: point(origin.x + cell_width * col as f32, y),
                        size: size(cell_width * width_cells as f32, line_height),
                    };
                    cells.push(TerminalSelectionCell {
                        start_utf16,
                        end_utf16: len_utf16,
                        bounds,
                    });
                    col += width_cells;
                }
            }

            let content_end_utf16 = len_utf16;
            if row_idx != last_content_row && !row_soft_wraps(row, content.grid_cols) {
                let byte_start = text.len();
                text.push('\n');
                len_utf16 += 1;
                chars.push(TerminalSelectionChar {
                    start_utf16: content_end_utf16,
                    end_utf16: len_utf16,
                    byte_start,
                    ch: '\n',
                });
            }

            lines.push(TerminalSelectionLine {
                start_utf16: line_start,
                content_end_utf16,
                separator_end_utf16: len_utf16,
                cells,
                x: origin.x,
                y,
                line_height,
            });
        }

        Self {
            text,
            len_utf16,
            chars,
            lines,
        }
    }

    pub fn len_utf16(&self) -> usize {
        self.len_utf16
    }

    pub fn word_range_at(&self, offset_utf16: usize) -> Range<usize> {
        if self.text.is_empty() {
            return 0..0;
        }

        let Some(ix) = self.char_index_for_utf16(offset_utf16) else {
            return self.len_utf16..self.len_utf16;
        };

        if self.chars[ix].ch.is_whitespace() {
            return self.chars[ix].start_utf16..self.chars[ix].start_utf16;
        }

        let mut start_ix = ix;
        while start_ix > 0 && !self.chars[start_ix - 1].ch.is_whitespace() {
            start_ix -= 1;
        }

        let mut end_ix = ix + 1;
        while end_ix < self.chars.len() && !self.chars[end_ix].ch.is_whitespace() {
            end_ix += 1;
        }

        self.chars[start_ix].start_utf16..self.chars[end_ix - 1].end_utf16
    }

    pub fn text_for_range(&self, range_utf16: Range<usize>) -> (Range<usize>, String) {
        let range = self.clamp_range(range_utf16);
        let start_byte = self.byte_offset_for_utf16(range.start);
        let end_byte = self.byte_offset_for_utf16(range.end);
        (range, self.text[start_byte..end_byte].to_string())
    }

    pub fn bounds_for_range(&self, range_utf16: Range<usize>) -> Option<Bounds<Pixels>> {
        let range = self.clamp_range(range_utf16);
        if range.is_empty() {
            return self.caret_bounds(range.start);
        }

        let mut result = None;
        for line in &self.lines {
            for cell in &line.cells {
                if cell.end_utf16 <= range.start || cell.start_utf16 >= range.end {
                    continue;
                }
                result = Some(result.map_or(cell.bounds, |bounds: Bounds<Pixels>| {
                    bounds.union(&cell.bounds)
                }));
            }
        }
        result
    }

    pub fn rects_for_range(&self, range_utf16: Range<usize>) -> SmallVec<[Bounds<Pixels>; 4]> {
        let range = self.clamp_range(range_utf16);
        if range.is_empty() {
            return SmallVec::new();
        }

        let mut rects = SmallVec::new();
        for line in &self.lines {
            let mut line_bounds = None;
            for cell in &line.cells {
                if cell.end_utf16 <= range.start || cell.start_utf16 >= range.end {
                    continue;
                }
                line_bounds = Some(line_bounds.map_or(cell.bounds, |bounds: Bounds<Pixels>| {
                    bounds.union(&cell.bounds)
                }));
            }
            if let Some(bounds) = line_bounds {
                rects.push(bounds);
            }
        }
        rects
    }

    pub fn character_index_for_point(&self, point: Point<Pixels>) -> Option<usize> {
        let line = self.line_for_y(point.y)?;
        let cell = cell_for_x(line, point.x)?;
        cell.bounds
            .contains(&point)
            .then(|| cell.index_for_x(point.x))
    }

    pub fn nearest_character_index_for_point(&self, point: Point<Pixels>) -> Option<usize> {
        let line = self.nearest_line_for_y(point.y)?;

        let Some(first_cell) = line.cells.first() else {
            return Some(line.start_utf16);
        };
        if point.x <= first_cell.bounds.origin.x {
            return Some(line.start_utf16);
        }

        let Some(last_cell) = line.cells.last() else {
            return Some(line.start_utf16);
        };
        let last_cell_right = last_cell.bounds.origin.x + last_cell.bounds.size.width;
        if point.x >= last_cell_right {
            return Some(line.content_end_utf16);
        }

        if let Some(cell) = cell_for_x(line, point.x).filter(|cell| cell.bounds.contains(&point)) {
            return Some(cell.index_for_x(point.x));
        }

        Some(line.content_end_utf16.min(line.separator_end_utf16))
    }

    pub(crate) fn clamp_range(&self, range: Range<usize>) -> Range<usize> {
        let start = range.start.min(self.len_utf16);
        let end = range.end.min(self.len_utf16);
        start.min(end)..start.max(end)
    }

    fn caret_bounds(&self, offset_utf16: usize) -> Option<Bounds<Pixels>> {
        if self.lines.is_empty() {
            return None;
        }

        for line in &self.lines {
            if offset_utf16 < line.start_utf16 || offset_utf16 > line.separator_end_utf16 {
                continue;
            }

            if let Some(cell) = line
                .cells
                .iter()
                .find(|cell| offset_utf16 <= cell.end_utf16)
            {
                let x = if offset_utf16 >= cell.end_utf16 {
                    cell.bounds.origin.x + cell.bounds.size.width
                } else {
                    cell.bounds.origin.x
                };
                return Some(Bounds {
                    origin: point(x, line.y),
                    size: size(px(1.0), line.line_height),
                });
            }

            return Some(Bounds {
                origin: point(line.x, line.y),
                size: size(px(1.0), line.line_height),
            });
        }

        self.lines.last().and_then(|line| {
            line.cells.last().map(|cell| Bounds {
                origin: point(cell.bounds.origin.x + cell.bounds.size.width, line.y),
                size: size(px(1.0), line.line_height),
            })
        })
    }

    fn char_index_for_utf16(&self, offset_utf16: usize) -> Option<usize> {
        if self.chars.is_empty() {
            return None;
        }
        let offset_utf16 = offset_utf16.min(self.len_utf16.saturating_sub(1));
        let index = self
            .chars
            .partition_point(|ch| ch.end_utf16 <= offset_utf16);
        self.chars
            .get(index)
            .filter(|ch| ch.start_utf16 <= offset_utf16)
            .map(|_| index)
    }

    fn byte_offset_for_utf16(&self, offset_utf16: usize) -> usize {
        if offset_utf16 >= self.len_utf16 {
            return self.text.len();
        }
        self.char_index_for_utf16(offset_utf16)
            .and_then(|index| self.chars.get(index))
            .map_or(self.text.len(), |ch| ch.byte_start)
    }

    fn line_for_y(&self, y: Pixels) -> Option<&TerminalSelectionLine> {
        let index = self.lines.partition_point(|line| y >= line.bottom());
        self.lines
            .get(index)
            .filter(|line| y >= line.y && y < line.bottom())
    }

    fn nearest_line_for_y(&self, y: Pixels) -> Option<&TerminalSelectionLine> {
        if let Some(line) = self.line_for_y(y) {
            return Some(line);
        }

        let first = self.lines.first()?;
        if y < first.y {
            return Some(first);
        }
        self.lines.last()
    }
}

impl TerminalSelectionCell {
    fn index_for_x(&self, x: Pixels) -> usize {
        let midpoint = self.bounds.origin.x + self.bounds.size.width * 0.5;
        if x < midpoint {
            self.start_utf16
        } else {
            self.end_utf16
        }
    }
}

impl TerminalSelectionLine {
    fn bottom(&self) -> Pixels {
        self.y + self.line_height
    }
}

fn cell_for_x(line: &TerminalSelectionLine, x: Pixels) -> Option<&TerminalSelectionCell> {
    let index = line
        .cells
        .partition_point(|cell| x >= cell.bounds.origin.x + cell.bounds.size.width);
    line.cells.get(index)
}

fn is_selectable_cell(cell: &IndexedCell) -> bool {
    !cell
        .cell
        .flags
        .intersects(CellFlags::WIDE_CHAR_SPACER | CellFlags::LEADING_WIDE_CHAR_SPACER)
}

fn selectable_nonblank_cell(cell: &IndexedCell) -> bool {
    is_selectable_cell(cell) && !is_blank(cell)
}

fn visible_cell_char(cell: &IndexedCell) -> char {
    match cell.cell.c {
        '\0' | '\t' => ' ',
        c => c,
    }
}

fn row_soft_wraps(row: &[&IndexedCell], grid_cols: usize) -> bool {
    let Some(last_col) = grid_cols.checked_sub(1) else {
        return false;
    };
    row.iter().any(|cell| {
        cell.point.column.0 == last_col && cell.cell.flags.contains(CellFlags::WRAPLINE)
    })
}

#[cfg(test)]
mod tests {
    use gpui::{point, px};

    use super::*;
    use crate::Terminal;

    fn selection_text(output: &[u8], cols: usize, rows: usize) -> String {
        let mut terminal = Terminal::new(cols, rows, px(10.0), px(20.0));
        terminal.advance_bytes(output);
        let content = terminal.content();
        let document =
            TerminalSelectionDocument::new(&content, point(px(0.0), px(0.0)), px(10.0), px(20.0));
        document.text_for_range(0..document.len_utf16()).1
    }

    #[test]
    fn extracts_hard_newline_text() {
        assert_eq!(selection_text(b"hello\r\nworld", 20, 4), "hello\nworld");
    }

    #[test]
    fn omits_newline_for_soft_wraps() {
        assert_eq!(selection_text(b"helloworld", 5, 4), "helloworld");
    }

    #[test]
    fn preserves_wide_glyphs_and_utf16_text() {
        let text = selection_text("a🙂界b".as_bytes(), 20, 4);

        assert_eq!(text, "a🙂界b");
        assert_eq!(text.encode_utf16().count(), 5);
    }

    #[test]
    fn trims_trailing_blank_cells() {
        assert_eq!(selection_text(b"abc   \r\n", 20, 4), "abc");
    }

    #[test]
    fn empty_selection_document_returns_empty_text() {
        let terminal = Terminal::new(20, 4, px(10.0), px(20.0));
        let content = terminal.content();
        let document =
            TerminalSelectionDocument::new(&content, point(px(0.0), px(0.0)), px(10.0), px(20.0));

        assert_eq!(document.len_utf16(), 0);
        assert_eq!(document.text_for_range(0..1), (0..0, String::new()));
    }

    #[test]
    fn selectable_text_flag_is_false_for_empty_or_blank_output() {
        let mut terminal = Terminal::new(20, 4, px(10.0), px(20.0));
        assert!(!TerminalSelectionDocument::has_selectable_text(
            &terminal.content()
        ));

        terminal.advance_bytes(b"     ");
        assert!(!TerminalSelectionDocument::has_selectable_text(
            &terminal.content()
        ));
    }

    #[test]
    fn selectable_text_flag_tracks_visible_terminal_output() {
        let mut terminal = Terminal::new(20, 4, px(10.0), px(20.0));
        terminal.advance_bytes("🙂 hello\r\n".as_bytes());
        assert!(TerminalSelectionDocument::has_selectable_text(
            &terminal.content()
        ));

        for line in 0..20 {
            terminal.advance_bytes(format!("line {line}\r\n").as_bytes());
        }
        terminal.scroll(20);
        assert!(TerminalSelectionDocument::has_selectable_text(
            &terminal.content()
        ));
    }

    #[test]
    fn text_for_range_uses_utf16_offsets() {
        let mut terminal = Terminal::new(20, 4, px(10.0), px(20.0));
        terminal.advance_bytes("a🙂b".as_bytes());
        let content = terminal.content();
        let document =
            TerminalSelectionDocument::new(&content, point(px(0.0), px(0.0)), px(10.0), px(20.0));

        assert_eq!(document.text_for_range(1..3), (1..3, "🙂".to_string()));
    }

    #[test]
    fn word_range_uses_non_whitespace_terminal_words() {
        let mut terminal = Terminal::new(40, 4, px(10.0), px(20.0));
        terminal.advance_bytes("open crates/foo.rs now".as_bytes());
        let content = terminal.content();
        let document =
            TerminalSelectionDocument::new(&content, point(px(0.0), px(0.0)), px(10.0), px(20.0));

        assert_eq!(
            document.text_for_range(document.word_range_at(8)).1,
            "crates/foo.rs"
        );
    }

    #[test]
    fn includes_scrolled_scrollback_viewport() {
        let mut terminal = Terminal::new(80, 4, px(10.0), px(20.0));
        terminal.advance_bytes(b"Read(crates/foo/file_a.rs)\r\n");
        for line in 0..20 {
            terminal.advance_bytes(format!("filler line {line}\r\n").as_bytes());
        }
        terminal.scroll(20);
        let content = terminal.content();
        let document =
            TerminalSelectionDocument::new(&content, point(px(0.0), px(0.0)), px(10.0), px(20.0));

        assert!(
            document
                .text_for_range(0..document.len_utf16())
                .1
                .contains("crates/foo/file_a.rs")
        );
    }
}
