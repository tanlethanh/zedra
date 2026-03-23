pub mod element;
pub mod keys;
pub mod view;

use std::borrow::Cow;
use std::sync::atomic::{AtomicU32, Ordering};

// Global keyboard height in physical pixels, set by the JNI layer.
// 0 means the keyboard is hidden.
static KEYBOARD_HEIGHT_PX: AtomicU32 = AtomicU32::new(0);

// Global display density (scale factor × 100, stored as integer).
// Default 300 = 3.0× scale. Set by the JNI layer.
static DISPLAY_DENSITY_X100: AtomicU32 = AtomicU32::new(300);

/// Set the current soft keyboard height (physical pixels). Called from JNI layer.
pub fn set_keyboard_height(px: u32) {
    KEYBOARD_HEIGHT_PX.store(px, Ordering::Relaxed);
}

/// Get the current soft keyboard height in physical pixels (0 = hidden).
pub fn get_keyboard_height() -> u32 {
    KEYBOARD_HEIGHT_PX.load(Ordering::Relaxed)
}

/// Set the display density (scale factor). Called from JNI layer.
pub fn set_display_density(density: f32) {
    DISPLAY_DENSITY_X100.store((density * 100.0) as u32, Ordering::Relaxed);
}

/// Get the display density (scale factor).
pub fn get_display_density() -> f32 {
    DISPLAY_DENSITY_X100.load(Ordering::Relaxed) as f32 / 100.0
}

/// The font family name for the embedded terminal font.
/// The font bytes and loader live in the `zedra` crate (`fonts` module).
pub const MONO_FONT_FAMILY: &str = "JetBrainsMonoNL Nerd Font Mono";

use alacritty_terminal::event::{Event as AlacTermEvent, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Direction, Line, Point as AlacPoint};
use alacritty_terminal::term::Config;

/// A detected hyperlink span in the terminal grid.
#[derive(Clone, Debug)]
pub struct LinkMatch {
    pub url: String,
    /// Absolute alacritty grid line (before display_offset adjustment).
    pub start_line: i32,
    pub start_col: usize,
    pub end_line: i32,
    pub end_col: usize,
}
use alacritty_terminal::term::cell::Cell;
use alacritty_terminal::term::search::{RegexIter, RegexSearch};
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::{CursorShape, Processor};
use gpui::Pixels;

// Same URL regex used by Zed's terminal_hyperlinks module.
const URL_REGEX: &str = r#"(ipfs:|ipns:|magnet:|mailto:|gemini://|gopher://|https://|http://|news:|file://|git://|ssh:|ftp://)[^\u{0000}-\u{001F}\u{007F}-\u{009F}<>"\s{-}\^⟨⟩`']+"#;

use crate::keys::to_esc_str;

/// Event listener that collects terminal events
#[derive(Clone)]
pub struct ZedraListener;

impl EventListener for ZedraListener {
    fn send_event(&self, _event: AlacTermEvent) {
        // Events like title change, bell, etc. are ignored for now
    }
}

/// Snapshot of terminal grid content for rendering
#[derive(Clone)]
pub struct TerminalContent {
    pub cells: Vec<IndexedCell>,
    pub mode: TermMode,
    pub display_offset: usize,
    pub cursor: CursorState,
    pub cursor_char: char,
    pub grid_rows: usize,
    pub grid_cols: usize,
}

/// A terminal cell with its grid position
#[derive(Clone, Debug)]
pub struct IndexedCell {
    pub point: alacritty_terminal::index::Point,
    pub cell: Cell,
}

/// Cursor rendering state
#[derive(Clone, Debug)]
pub struct CursorState {
    pub point: alacritty_terminal::index::Point,
    pub shape: CursorShape,
}

/// Terminal size in cells and pixels
#[derive(Clone, Copy, Debug)]
pub struct TerminalSize {
    pub cell_width: Pixels,
    pub line_height: Pixels,
    pub columns: usize,
    pub rows: usize,
}

/// Simple Dimensions implementation for terminal sizing
struct SimpleDimensions {
    columns: usize,
    screen_lines: usize,
}

impl Dimensions for SimpleDimensions {
    fn total_lines(&self) -> usize {
        self.screen_lines
    }

    fn screen_lines(&self) -> usize {
        self.screen_lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

/// Minimal terminal state wrapping alacritty_terminal::Term
pub struct TerminalState {
    term: Term<ZedraListener>,
    /// VTE processor — persisted across advance_bytes calls so that
    /// escape sequences split across network packets are parsed correctly.
    processor: Processor,
    mode: TermMode,
    size: TerminalSize,
    /// Compiled URL regex for tap-to-open link detection (fallback when no OSC 8 hyperlink).
    url_regex: Option<RegexSearch>,
}

impl TerminalState {
    /// Create a new terminal with the given grid dimensions
    pub fn new(columns: usize, rows: usize, cell_width: Pixels, line_height: Pixels) -> Self {
        let config = Config::default();
        let term_size = SimpleDimensions {
            columns,
            screen_lines: rows,
        };
        let term = Term::new(config, &term_size, ZedraListener);

        let url_regex = RegexSearch::new(URL_REGEX)
            .map_err(|e| tracing::warn!("terminal URL regex init failed: {e}"))
            .ok();

        Self {
            term,
            processor: Processor::new(),
            mode: TermMode::empty(),
            size: TerminalSize {
                cell_width,
                line_height,
                columns,
                rows,
            },
            url_regex,
        }
    }

    /// Feed bytes from SSH output into the terminal emulator
    pub fn advance_bytes(&mut self, bytes: &[u8]) {
        self.processor.advance(&mut self.term, bytes);
        self.mode = *self.term.mode();
    }

    /// Get a snapshot of the terminal content for rendering
    pub fn content(&self) -> TerminalContent {
        let content = self.term.renderable_content();
        let mut cells = Vec::new();

        for ic in content.display_iter {
            cells.push(IndexedCell {
                point: ic.point,
                cell: ic.cell.clone(),
            });
        }

        let cursor_point = content.cursor.point;
        let cursor_char = self.term.grid()[cursor_point].c;

        TerminalContent {
            cells,
            mode: content.mode,
            display_offset: content.display_offset,
            cursor: CursorState {
                point: cursor_point,
                shape: content.cursor.shape,
            },
            cursor_char,
            grid_rows: self.size.rows,
            grid_cols: self.size.columns,
        }
    }

    /// Convert a GPUI keystroke to terminal escape sequence bytes
    pub fn try_keystroke(&self, keystroke: &gpui::Keystroke) -> Option<Vec<u8>> {
        let esc = to_esc_str(keystroke, &self.mode, false);
        esc.map(|s| match s {
            Cow::Borrowed(string) => string.as_bytes().to_vec(),
            Cow::Owned(string) => string.into_bytes(),
        })
    }

    /// Resize the terminal grid
    pub fn resize(&mut self, columns: usize, rows: usize, cell_width: Pixels, line_height: Pixels) {
        self.size = TerminalSize {
            cell_width,
            line_height,
            columns,
            rows,
        };
        let term_size = SimpleDimensions {
            columns,
            screen_lines: rows,
        };
        self.term.resize(term_size);
    }

    /// Get current terminal size
    pub fn size(&self) -> TerminalSize {
        self.size
    }

    /// Get current terminal mode
    pub fn mode(&self) -> TermMode {
        self.mode
    }

    /// Scroll the terminal by a number of lines (positive = up)
    pub fn scroll(&mut self, lines: i32) {
        let scroll = alacritty_terminal::grid::Scroll::Delta(lines);
        self.term.scroll_display(scroll);
    }

    /// Current display offset (0 = bottom, history_size = top)
    pub fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    /// Get total history size
    pub fn history_size(&self) -> usize {
        self.term.grid().history_size()
    }

    pub fn link_at(&mut self, col: usize, screen_row: usize) -> Option<LinkMatch> {
        if col >= self.size.columns || screen_row >= self.size.rows {
            return None;
        }
        let display_offset = self.term.grid().display_offset() as i32;
        let grid_row = screen_row as i32 - display_offset;
        let point = AlacPoint::new(Line(grid_row), Column(col));

        // OSC 8 hyperlink takes priority — find the full span of adjacent cells.
        if let Some(uri) = self.term.grid()[point]
            .hyperlink()
            .map(|h| h.uri().to_owned())
        {
            let (start, end) = self.osc_link_span(point, &uri);
            return Some(LinkMatch {
                url: uri,
                start_line: start.line.0,
                start_col: start.column.0,
                end_line: end.line.0,
                end_col: end.column.0,
            });
        }

        // Regex scan the line for bare URLs (https://, file://, etc.).
        if let Some(ref mut url_regex) = self.url_regex {
            let line_start = self.term.line_search_left(point);
            let line_end = self.term.line_search_right(point);
            return RegexIter::new(
                line_start,
                line_end,
                Direction::Right,
                &self.term,
                url_regex,
            )
            .find(|m| m.contains(&point))
            .map(|m| {
                let url = self.term.bounds_to_string(*m.start(), *m.end());
                LinkMatch {
                    url,
                    start_line: m.start().line.0,
                    start_col: m.start().column.0,
                    end_line: m.end().line.0,
                    end_col: m.end().column.0,
                }
            });
        }

        None
    }

    /// Find the full horizontal span of an OSC 8 hyperlink by scanning adjacent cells
    /// on the same line that share the same URI.
    fn osc_link_span(&self, point: AlacPoint, uri: &str) -> (AlacPoint, AlacPoint) {
        let line = point.line;
        let max_col = self.size.columns.saturating_sub(1);

        let mut start = point;
        loop {
            if start.column.0 == 0 {
                break;
            }
            let prev = AlacPoint::new(line, Column(start.column.0 - 1));
            let matches = self.term.grid()[prev]
                .hyperlink()
                .map_or(false, |h| h.uri() == uri);
            if matches {
                start = prev;
            } else {
                break;
            }
        }

        let mut end = point;
        loop {
            if end.column.0 >= max_col {
                break;
            }
            let next = AlacPoint::new(line, Column(end.column.0 + 1));
            let matches = self.term.grid()[next]
                .hyperlink()
                .map_or(false, |h| h.uri() == uri);
            if matches {
                end = next;
            } else {
                break;
            }
        }

        (start, end)
    }
}
