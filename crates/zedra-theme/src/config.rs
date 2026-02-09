//! Editor configuration settings.

use serde::{Deserialize, Serialize};

/// Configuration for the code editor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditorConfig {
    /// Font size in pixels.
    pub font_size: f32,

    /// Line height in pixels.
    pub line_height: f32,

    /// Width of the line number gutter in pixels.
    pub gutter_width: f32,

    /// Horizontal padding for code content in pixels.
    pub horizontal_padding: f32,

    /// Vertical padding for the editor in pixels.
    pub vertical_padding: f32,

    /// Tab size in spaces.
    pub tab_size: u32,

    /// Whether to show line numbers.
    pub show_line_numbers: bool,

    /// Whether to highlight the current line.
    pub highlight_current_line: bool,

    /// Whether to show the minimap.
    pub show_minimap: bool,

    /// Cursor style.
    pub cursor_style: CursorStyle,

    /// Cursor blink rate in milliseconds (0 = no blink).
    pub cursor_blink_ms: u32,

    /// Cursor width in pixels.
    pub cursor_width: f32,

    /// Whether word wrap is enabled.
    pub word_wrap: bool,

    /// Scroll speed multiplier.
    pub scroll_speed: f32,
}

impl Default for EditorConfig {
    fn default() -> Self {
        Self {
            font_size: 13.0,
            line_height: 22.0,
            gutter_width: 52.0,
            horizontal_padding: 12.0,
            vertical_padding: 8.0,
            tab_size: 4,
            show_line_numbers: true,
            highlight_current_line: true,
            show_minimap: false,
            cursor_style: CursorStyle::Line,
            cursor_blink_ms: 500,
            cursor_width: 2.0,
            word_wrap: false,
            scroll_speed: 1.0,
        }
    }
}

impl EditorConfig {
    /// Compact configuration for mobile devices.
    pub fn mobile() -> Self {
        Self {
            font_size: 12.0,
            line_height: 20.0,
            gutter_width: 44.0,
            horizontal_padding: 8.0,
            vertical_padding: 4.0,
            show_minimap: false,
            ..Default::default()
        }
    }

    /// Configuration optimized for larger screens.
    pub fn desktop() -> Self {
        Self {
            font_size: 14.0,
            line_height: 24.0,
            gutter_width: 56.0,
            horizontal_padding: 16.0,
            vertical_padding: 12.0,
            show_minimap: true,
            ..Default::default()
        }
    }

    /// Calculate the approximate character width based on font size.
    /// Uses a multiplier typical for monospace fonts.
    pub fn char_width(&self) -> f32 {
        self.font_size * 0.602
    }

    /// Calculate cursor X position for a given column.
    pub fn cursor_x(&self, column: usize) -> f32 {
        self.horizontal_padding + (column as f32 * self.char_width())
    }
}

/// Cursor display style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CursorStyle {
    /// Thin vertical line (default).
    Line,
    /// Block cursor.
    Block,
    /// Underline cursor.
    Underline,
}

impl Default for CursorStyle {
    fn default() -> Self {
        CursorStyle::Line
    }
}

/// Configuration for the status bar.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusBarConfig {
    /// Height of the status bar in pixels.
    pub height: f32,

    /// Whether to show the language badge.
    pub show_language: bool,

    /// Whether to show the cursor position.
    pub show_position: bool,

    /// Whether to show the line count.
    pub show_line_count: bool,

    /// Whether to show the encoding.
    pub show_encoding: bool,

    /// Whether to show the line ending type.
    pub show_line_ending: bool,
}

impl Default for StatusBarConfig {
    fn default() -> Self {
        Self {
            height: 28.0,
            show_language: true,
            show_position: true,
            show_line_count: true,
            show_encoding: false,
            show_line_ending: false,
        }
    }
}

/// Configuration for file preview cards.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilePreviewConfig {
    /// Width of preview cards in pixels.
    pub card_width: f32,

    /// Height of preview cards in pixels.
    pub card_height: f32,

    /// Number of preview lines to show.
    pub preview_lines: usize,

    /// Maximum characters per preview line.
    pub max_line_chars: usize,

    /// Gap between cards in pixels.
    pub gap: f32,

    /// Card corner radius in pixels.
    pub border_radius: f32,
}

impl Default for FilePreviewConfig {
    fn default() -> Self {
        Self {
            card_width: 155.0,
            card_height: 180.0,
            preview_lines: 6,
            max_line_chars: 24,
            gap: 12.0,
            border_radius: 8.0,
        }
    }
}
