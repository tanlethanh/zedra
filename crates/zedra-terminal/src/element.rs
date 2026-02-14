// Terminal element for GPUI rendering
// Adapted from vendor/zed/crates/terminal_view/src/terminal_element.rs

use std::mem;

use alacritty_terminal::term::cell::Flags as CellFlags;
use alacritty_terminal::vte::ansi::{Color as AlacColor, CursorShape, NamedColor};
use gpui::*;
use itertools::Itertools;

use crate::{CursorState, IndexedCell, TERMINAL_FONT_FAMILY, TerminalContent, TerminalSize};

/// Colors for the terminal (One Dark theme)
struct TermColors;

impl TermColors {
    const BACKGROUND: u32 = 0x1e1e1e;
    const FOREGROUND: u32 = 0xabb2bf;
    const CURSOR: u32 = 0x528bff;

    // ANSI standard colors
    const BLACK: u32 = 0x282c34;
    const RED: u32 = 0xe06c75;
    const GREEN: u32 = 0x98c379;
    const YELLOW: u32 = 0xe5c07b;
    const BLUE: u32 = 0x61afef;
    const MAGENTA: u32 = 0xc678dd;
    const CYAN: u32 = 0x56b6c2;
    const WHITE: u32 = 0xabb2bf;

    // Bright variants
    const BRIGHT_BLACK: u32 = 0x5c6370;
    const BRIGHT_RED: u32 = 0xe06c75;
    const BRIGHT_GREEN: u32 = 0x98c379;
    const BRIGHT_YELLOW: u32 = 0xe5c07b;
    const BRIGHT_BLUE: u32 = 0x61afef;
    const BRIGHT_MAGENTA: u32 = 0xc678dd;
    const BRIGHT_CYAN: u32 = 0x56b6c2;
    const BRIGHT_WHITE: u32 = 0xffffff;

    fn named_color(color: NamedColor) -> Hsla {
        let hex = match color {
            NamedColor::Black => Self::BLACK,
            NamedColor::Red => Self::RED,
            NamedColor::Green => Self::GREEN,
            NamedColor::Yellow => Self::YELLOW,
            NamedColor::Blue => Self::BLUE,
            NamedColor::Magenta => Self::MAGENTA,
            NamedColor::Cyan => Self::CYAN,
            NamedColor::White => Self::WHITE,
            NamedColor::BrightBlack => Self::BRIGHT_BLACK,
            NamedColor::BrightRed => Self::BRIGHT_RED,
            NamedColor::BrightGreen => Self::BRIGHT_GREEN,
            NamedColor::BrightYellow => Self::BRIGHT_YELLOW,
            NamedColor::BrightBlue => Self::BRIGHT_BLUE,
            NamedColor::BrightMagenta => Self::BRIGHT_MAGENTA,
            NamedColor::BrightCyan => Self::BRIGHT_CYAN,
            NamedColor::BrightWhite => Self::BRIGHT_WHITE,
            NamedColor::Foreground => Self::FOREGROUND,
            NamedColor::Background => Self::BACKGROUND,
            _ => Self::FOREGROUND,
        };
        rgb(hex).into()
    }

    fn convert_color(color: &AlacColor) -> Hsla {
        match color {
            AlacColor::Named(named) => Self::named_color(*named),
            AlacColor::Spec(rgb_color) => {
                let r = rgb_color.r as u32;
                let g = rgb_color.g as u32;
                let b = rgb_color.b as u32;
                rgb((r << 16) | (g << 8) | b).into()
            }
            AlacColor::Indexed(index) => {
                if *index < 16 {
                    let named = match index {
                        0 => NamedColor::Black,
                        1 => NamedColor::Red,
                        2 => NamedColor::Green,
                        3 => NamedColor::Yellow,
                        4 => NamedColor::Blue,
                        5 => NamedColor::Magenta,
                        6 => NamedColor::Cyan,
                        7 => NamedColor::White,
                        8 => NamedColor::BrightBlack,
                        9 => NamedColor::BrightRed,
                        10 => NamedColor::BrightGreen,
                        11 => NamedColor::BrightYellow,
                        12 => NamedColor::BrightBlue,
                        13 => NamedColor::BrightMagenta,
                        14 => NamedColor::BrightCyan,
                        15 => NamedColor::BrightWhite,
                        _ => NamedColor::Foreground,
                    };
                    Self::named_color(named)
                } else if *index < 232 {
                    // 216-color cube (indices 16-231)
                    let idx = *index as u32 - 16;
                    let r = (idx / 36) * 51;
                    let g = ((idx / 6) % 6) * 51;
                    let b = (idx % 6) * 51;
                    rgb((r << 16) | (g << 8) | b).into()
                } else {
                    // Grayscale (indices 232-255)
                    let level = (*index as u32 - 232) * 10 + 8;
                    rgb((level << 16) | (level << 8) | level).into()
                }
            }
        }
    }
}

/// A batched text run that combines multiple adjacent cells with the same style
/// Following Zed's BatchedTextRun implementation
#[derive(Debug)]
struct BatchedTextRun {
    /// Starting grid position (line, column)
    start_line: i32,
    start_col: i32,
    /// The accumulated text
    text: String,
    /// Number of cells this run covers (may differ from text.len() for wide chars)
    cell_count: usize,
    /// Text color
    color: Hsla,
}

impl BatchedTextRun {
    fn new(line: i32, col: i32, c: char, color: Hsla) -> Self {
        let mut text = String::with_capacity(100);
        text.push(c);
        BatchedTextRun {
            start_line: line,
            start_col: col,
            text,
            cell_count: 1,
            color,
        }
    }

    fn can_append(&self, line: i32, col: i32, color: Hsla) -> bool {
        self.start_line == line
            && self.start_col + self.cell_count as i32 == col
            && self.color == color
    }

    fn append_char(&mut self, c: char) {
        self.text.push(c);
        self.cell_count += 1;
    }

    fn paint(
        &self,
        origin: Point<Pixels>,
        cell_width: Pixels,
        line_height: Pixels,
        font_size: Pixels,
        font: &Font,
        window: &mut Window,
        cx: &mut App,
    ) {
        // Position: no floor/ceil for text (unlike background rects)
        let pos = point(
            origin.x + self.start_col as f32 * cell_width,
            origin.y + self.start_line as f32 * line_height,
        );

        let runs = vec![TextRun {
            len: self.text.len(),
            font: font.clone(),
            color: self.color,
            ..Default::default()
        }];

        let text_system = window.text_system();
        let shared_text: SharedString = self.text.clone().into();

        // Use force_width to ensure monospace grid alignment
        let shaped = text_system.shape_line(shared_text, font_size, &runs, Some(cell_width));

        let _ = shaped.paint(pos, line_height, TextAlign::Left, None, window, cx);
    }
}

/// A background rectangle
#[derive(Debug, Clone)]
struct LayoutRect {
    line: i32,
    col: i32,
    num_cells: usize,
    color: Hsla,
}

impl LayoutRect {
    fn new(line: i32, col: i32, num_cells: usize, color: Hsla) -> Self {
        LayoutRect {
            line,
            col,
            num_cells,
            color,
        }
    }

    fn paint(
        &self,
        origin: Point<Pixels>,
        cell_width: Pixels,
        line_height: Pixels,
        window: &mut Window,
    ) {
        // Background rects use floor for position and ceil for width to prevent gaps
        let position = point(
            (origin.x + self.col as f32 * cell_width).floor(),
            origin.y + self.line as f32 * line_height,
        );
        let size = gpui::Size {
            width: (cell_width * self.num_cells as f32).ceil(),
            height: line_height,
        };
        window.paint_quad(fill(Bounds::new(position, size), self.color));
    }
}

/// Check if a cell is blank (following Zed's is_blank function)
fn is_blank(cell: &IndexedCell) -> bool {
    if cell.cell.c != ' ' {
        return false;
    }
    if !matches!(cell.cell.bg, AlacColor::Named(NamedColor::Background)) {
        return false;
    }
    if cell
        .cell
        .flags
        .intersects(CellFlags::ALL_UNDERLINES | CellFlags::INVERSE | CellFlags::STRIKEOUT)
    {
        return false;
    }
    true
}

/// Data needed to paint the terminal element
pub struct TerminalElementLayout {
    content: TerminalContent,
    font: Font,
    font_size: Pixels,
    cell_width: Pixels,
    line_height: Pixels,
}

/// GPUI element that renders a terminal grid
pub struct TerminalElement {
    content: TerminalContent,
    size: TerminalSize,
}

impl TerminalElement {
    pub fn new(content: TerminalContent, size: TerminalSize) -> Self {
        Self { content, size }
    }

    /// Layout the grid following Zed's layout_grid approach.
    /// Groups cells by line and batches adjacent cells with the same style.
    /// Only cells within [0, grid_rows) after display_offset adjustment are rendered.
    fn layout_grid(
        cells: &[IndexedCell],
        display_offset: i32,
        grid_rows: usize,
    ) -> (Vec<LayoutRect>, Vec<BatchedTextRun>) {
        let mut batched_runs: Vec<BatchedTextRun> = Vec::new();
        let mut rects: Vec<LayoutRect> = Vec::new();
        let mut current_batch: Option<BatchedTextRun> = None;

        // Group cells by line (following Zed's chunk_by approach)
        let line_groups = cells.iter().chunk_by(|cell| cell.point.line.0);

        for (_line_key, line_cells) in &line_groups {
            // Flush batch at line boundaries
            if let Some(batch) = current_batch.take() {
                batched_runs.push(batch);
            }

            for cell in line_cells {
                let line = cell.point.line.0 + display_offset;
                let col = cell.point.column.0 as i32;

                // Skip cells outside the visible grid (stale circular buffer data)
                if line < 0 || line >= grid_rows as i32 {
                    continue;
                }

                // Handle INVERSE flag
                let mut fg = cell.cell.fg;
                let mut bg = cell.cell.bg;
                if cell.cell.flags.contains(CellFlags::INVERSE) {
                    mem::swap(&mut fg, &mut bg);
                }

                let fg_color = TermColors::convert_color(&fg);
                let bg_color = TermColors::convert_color(&bg);

                // Collect background rectangles (skip default background)
                if !matches!(bg, AlacColor::Named(NamedColor::Background)) {
                    // Try to extend the last rect if it's adjacent and same color
                    if let Some(last_rect) = rects.last_mut() {
                        if last_rect.line == line
                            && last_rect.col + last_rect.num_cells as i32 == col
                            && last_rect.color == bg_color
                        {
                            last_rect.num_cells += 1;
                        } else {
                            rects.push(LayoutRect::new(line, col, 1, bg_color));
                        }
                    } else {
                        rects.push(LayoutRect::new(line, col, 1, bg_color));
                    }
                }

                // Skip wide character spacers
                if cell.cell.flags.contains(CellFlags::WIDE_CHAR_SPACER) {
                    continue;
                }

                // Skip blank cells (but they break the current batch)
                if is_blank(cell) {
                    if let Some(batch) = current_batch.take() {
                        batched_runs.push(batch);
                    }
                    continue;
                }

                let c = cell.cell.c;

                // Try to batch with existing run
                if let Some(ref mut batch) = current_batch {
                    if batch.can_append(line, col, fg_color) {
                        batch.append_char(c);
                    } else {
                        // Flush current batch and start new one
                        batched_runs.push(current_batch.take().unwrap());
                        current_batch = Some(BatchedTextRun::new(line, col, c, fg_color));
                    }
                } else {
                    current_batch = Some(BatchedTextRun::new(line, col, c, fg_color));
                }
            }
        }

        // Flush any remaining batch
        if let Some(batch) = current_batch {
            batched_runs.push(batch);
        }

        (rects, batched_runs)
    }
}

impl IntoElement for TerminalElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TerminalElement {
    type RequestLayoutState = ();
    type PrepaintState = TerminalElementLayout;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let height = self.size.line_height * self.size.rows as f32;
        let style = Style {
            size: gpui::Size {
                width: relative(1.).into(), // fill parent, center grid in paint
                height: height.into(),
            },
            ..Default::default()
        };
        let layout_id = window.request_layout(style, [], cx);
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        // Use JetBrains Mono NL - embedded monospace font (loaded once at app init)
        let font = Font {
            family: TERMINAL_FONT_FAMILY.into(),
            features: FontFeatures::default(),
            fallbacks: Some(FontFallbacks::from_fonts(vec![
                "Droid Sans Mono".to_string(),
                "monospace".to_string(),
            ])),
            weight: FontWeight::NORMAL,
            style: FontStyle::Normal,
        };

        // Use configured line height
        let line_height = self.size.line_height;
        // Font size scaled to fit within line height
        let font_size = line_height * 0.75;

        // Get exact cell width from font metrics using 'm' as reference (following Zed)
        let text_system = window.text_system();
        let font_id = text_system.resolve_font(&font);
        let cell_width = text_system
            .advance(font_id, font_size, 'm')
            .map(|size| size.width)
            .unwrap_or(self.size.cell_width);

        TerminalElementLayout {
            content: self.content.clone(),
            font,
            font_size,
            cell_width,
            line_height,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        layout: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let cell_width = layout.cell_width;
        let line_height = layout.line_height;

        // Center the grid horizontally when it's narrower than the allocated bounds
        let grid_width = cell_width * layout.content.grid_cols as f32;
        let x_offset = ((bounds.size.width - grid_width) * 0.5).max(px(0.0));
        let origin = point(bounds.origin.x + x_offset, bounds.origin.y);

        // Draw terminal background
        window.paint_quad(fill(bounds, rgb(TermColors::BACKGROUND)));

        // Layout the grid (batch text runs, collect background rects)
        let (rects, batched_runs) = Self::layout_grid(
            &layout.content.cells,
            layout.content.display_offset as i32,
            layout.content.grid_rows,
        );

        // Paint background rectangles first
        for rect in &rects {
            rect.paint(origin, cell_width, line_height, window);
        }

        // Paint text runs
        for batch in &batched_runs {
            batch.paint(
                origin,
                cell_width,
                line_height,
                layout.font_size,
                &layout.font,
                window,
                cx,
            );
        }

        // Paint cursor (following Zed's cursor positioning)
        paint_cursor(
            window,
            &layout.content.cursor,
            origin,
            layout.content.display_offset as i32,
            layout.content.grid_rows,
            layout.content.grid_cols,
            cell_width,
            line_height,
        );
    }
}

fn paint_cursor(
    window: &mut Window,
    cursor: &CursorState,
    origin: Point<Pixels>,
    display_offset: i32,
    grid_rows: usize,
    grid_cols: usize,
    cell_width: Pixels,
    line_height: Pixels,
) {
    let col = cursor.point.column.0 as i32;
    let line = cursor.point.line.0 + display_offset;

    // Don't paint cursor when hidden (TUI apps manage their own virtual cursor)
    if matches!(cursor.shape, CursorShape::Hidden) {
        return;
    }

    // Only paint cursor if it's within the visible grid area
    if line < 0 || line >= grid_rows as i32 || col < 0 || col >= grid_cols as i32 {
        return;
    }

    // Cursor uses floor for position (following Zed's shape_cursor)
    let cursor_origin = point(
        (origin.x + col as f32 * cell_width).floor(),
        (origin.y + line as f32 * line_height).floor(),
    );

    let cursor_color: Hsla = rgb(TermColors::CURSOR).into();

    // Cursor width uses ceil (following Zed)
    let cursor_width = cell_width.ceil();

    match cursor.shape {
        CursorShape::Block => {
            let bounds = Bounds {
                origin: cursor_origin,
                size: gpui::Size {
                    width: cursor_width,
                    height: line_height,
                },
            };
            window.paint_quad(fill(bounds, cursor_color));
        }
        CursorShape::Underline => {
            let underline_height = px(2.0);
            let bounds = Bounds {
                origin: point(
                    cursor_origin.x,
                    cursor_origin.y + line_height - underline_height,
                ),
                size: gpui::Size {
                    width: cursor_width,
                    height: underline_height,
                },
            };
            window.paint_quad(fill(bounds, cursor_color));
        }
        CursorShape::Beam => {
            let beam_width = px(2.0);
            let bounds = Bounds {
                origin: cursor_origin,
                size: gpui::Size {
                    width: beam_width,
                    height: line_height,
                },
            };
            window.paint_quad(fill(bounds, cursor_color));
        }
        _ => {
            let bounds = Bounds {
                origin: cursor_origin,
                size: gpui::Size {
                    width: cursor_width,
                    height: line_height,
                },
            };
            window.paint_quad(fill(bounds, cursor_color));
        }
    }
}
