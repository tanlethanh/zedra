// Terminal element for GPUI rendering
// Adapted from vendor/zed/crates/terminal_view/src/terminal_element.rs

use alacritty_terminal::term::cell::Flags as CellFlags;
use alacritty_terminal::vte::ansi::{Color as AlacColor, CursorShape, NamedColor};
use gpui::*;

use crate::{CursorState, TerminalContent, TerminalSize};

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
            _ => Self::FOREGROUND,
        };
        rgb(hex).into()
    }

    fn alac_color_to_hsla(color: &AlacColor) -> Hsla {
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

/// Data needed to paint the terminal element
pub struct TerminalElementLayout {
    content: TerminalContent,
    size: TerminalSize,
    font: Font,
    font_size: Pixels,
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
        let width = self.size.cell_width * self.size.columns as f32;
        let height = self.size.line_height * self.size.rows as f32;
        let style = Style {
            size: gpui::Size {
                width: width.into(),
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
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        let font = Font {
            family: "monospace".into(),
            features: FontFeatures::default(),
            fallbacks: None,
            weight: FontWeight::NORMAL,
            style: FontStyle::Normal,
        };
        let font_size = self.size.line_height * 0.75;

        TerminalElementLayout {
            content: self.content.clone(),
            size: self.size,
            font,
            font_size,
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
        let origin = bounds.origin;
        let cell_width = layout.size.cell_width;
        let line_height = layout.size.line_height;

        // Draw background
        window.paint_quad(fill(bounds, rgb(TermColors::BACKGROUND)));

        // Group cells by line for batch rendering
        let mut current_line: Option<i32> = None;
        let mut line_text = String::new();
        let mut line_start_col: usize = 0;
        let mut line_fg_color: Hsla = rgb(TermColors::FOREGROUND).into();

        for cell in &layout.content.cells {
            let line = cell.point.line.0;
            let col = cell.point.column.0;

            let fg = TermColors::alac_color_to_hsla(&cell.cell.fg);
            let bg = TermColors::alac_color_to_hsla(&cell.cell.bg);

            // Draw cell background if not default
            let is_default_bg = matches!(cell.cell.bg, AlacColor::Named(NamedColor::Background));
            if !is_default_bg {
                let cell_origin = point(
                    origin.x + cell_width * col as f32,
                    origin.y + line_height * (line + layout.content.display_offset as i32) as f32,
                );
                let cell_bounds = Bounds {
                    origin: cell_origin,
                    size: gpui::Size {
                        width: cell_width,
                        height: line_height,
                    },
                };
                window.paint_quad(fill(cell_bounds, bg));
            }

            // Skip spacers for wide characters
            if cell.cell.flags.contains(CellFlags::WIDE_CHAR_SPACER) {
                continue;
            }

            let ch = cell.cell.c;
            if ch == ' ' || ch == '\0' {
                // Flush current text batch if style changed
                if !line_text.is_empty() && (Some(line) != current_line || fg != line_fg_color) {
                    paint_text_run(
                        window,
                        cx,
                        &line_text,
                        origin,
                        line_start_col,
                        current_line.unwrap_or(0),
                        layout.content.display_offset as i32,
                        cell_width,
                        line_height,
                        layout.font_size,
                        &layout.font,
                        line_fg_color,
                    );
                    line_text.clear();
                }
                current_line = Some(line);
                line_fg_color = fg;
                continue;
            }

            // Start new batch or continue existing
            if Some(line) != current_line || fg != line_fg_color || line_text.is_empty() {
                // Flush previous batch
                if !line_text.is_empty() {
                    paint_text_run(
                        window,
                        cx,
                        &line_text,
                        origin,
                        line_start_col,
                        current_line.unwrap_or(0),
                        layout.content.display_offset as i32,
                        cell_width,
                        line_height,
                        layout.font_size,
                        &layout.font,
                        line_fg_color,
                    );
                    line_text.clear();
                }
                current_line = Some(line);
                line_start_col = col;
                line_fg_color = fg;
            }

            line_text.push(ch);
        }

        // Flush remaining text
        if !line_text.is_empty() {
            paint_text_run(
                window,
                cx,
                &line_text,
                origin,
                line_start_col,
                current_line.unwrap_or(0),
                layout.content.display_offset as i32,
                cell_width,
                line_height,
                layout.font_size,
                &layout.font,
                line_fg_color,
            );
        }

        // Draw cursor
        paint_cursor(
            window,
            &layout.content.cursor,
            origin,
            layout.content.display_offset as i32,
            cell_width,
            line_height,
        );
    }
}

fn paint_text_run(
    window: &mut Window,
    cx: &mut App,
    text: &str,
    origin: Point<Pixels>,
    start_col: usize,
    line: i32,
    display_offset: i32,
    cell_width: Pixels,
    line_height: Pixels,
    font_size: Pixels,
    font: &Font,
    color: Hsla,
) {
    let text_origin = point(
        origin.x + cell_width * start_col as f32,
        origin.y + line_height * (line + display_offset) as f32,
    );

    // Use TextRun (public API) for shape_line
    let runs = vec![TextRun {
        len: text.len(),
        font: font.clone(),
        color,
        ..Default::default()
    }];

    let text_system = window.text_system();
    let shared_text: SharedString = text.to_string().into();
    let shaped = text_system.shape_line(
        shared_text,
        font_size,
        &runs,
        None,
    );

    // Calculate vertical centering within line height
    let text_height = shaped.ascent + shaped.descent;
    let y_offset = (line_height - text_height) / 2.0 + shaped.ascent;

    let paint_origin = point(text_origin.x, text_origin.y + y_offset);
    let _ = shaped.paint(
        paint_origin,
        line_height,
        TextAlign::Left,
        None,
        window,
        cx,
    );
}

fn paint_cursor(
    window: &mut Window,
    cursor: &CursorState,
    origin: Point<Pixels>,
    display_offset: i32,
    cell_width: Pixels,
    line_height: Pixels,
) {
    let col = cursor.point.column.0;
    let line = cursor.point.line.0;

    let cursor_origin = point(
        origin.x + cell_width * col as f32,
        origin.y + line_height * (line + display_offset) as f32,
    );

    let cursor_color: Hsla = rgb(TermColors::CURSOR).into();

    match cursor.shape {
        CursorShape::Block => {
            let bounds = Bounds {
                origin: cursor_origin,
                size: gpui::Size {
                    width: cell_width,
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
                    width: cell_width,
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
                    width: cell_width,
                    height: line_height,
                },
            };
            window.paint_quad(fill(bounds, cursor_color));
        }
    }
}
