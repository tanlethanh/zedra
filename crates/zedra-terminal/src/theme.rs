//! Terminal color tokens and xterm-256 tables for light/dark appearance.
//!
//! Product UI chrome uses `zedra::theme::ThemePalette`; terminal ANSI/truecolor uses this module only.
//! See `docs/THEMING.md`.

use alacritty_terminal::vte::ansi::{Color as AlacColor, NamedColor, Rgb as AlacRgb};
use gpui::{Hsla, rgb};

/// ANSI terminal colors from the theme. The 256-color cube/ramp is derived from xterm.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnsiPalette {
    pub black: u32,
    pub red: u32,
    pub green: u32,
    pub yellow: u32,
    pub blue: u32,
    pub magenta: u32,
    pub cyan: u32,
    pub white: u32,
    pub bright_black: u32,
    pub bright_red: u32,
    pub bright_green: u32,
    pub bright_yellow: u32,
    pub bright_blue: u32,
    pub bright_magenta: u32,
    pub bright_cyan: u32,
    pub bright_white: u32,
    pub dim_black: u32,
    pub dim_red: u32,
    pub dim_green: u32,
    pub dim_yellow: u32,
    pub dim_blue: u32,
    pub dim_magenta: u32,
    pub dim_cyan: u32,
    pub dim_white: u32,
}

impl AnsiPalette {
    pub fn color(self, index: u8) -> u32 {
        match index {
            0 => self.black,
            1 => self.red,
            2 => self.green,
            3 => self.yellow,
            4 => self.blue,
            5 => self.magenta,
            6 => self.cyan,
            7 => self.white,
            8 => self.bright_black,
            9 => self.bright_red,
            10 => self.bright_green,
            11 => self.bright_yellow,
            12 => self.bright_blue,
            13 => self.bright_magenta,
            14 => self.bright_cyan,
            15 => self.bright_white,
            _ => self.black,
        }
    }

    pub fn dim_color(self, index: u8) -> u32 {
        match index {
            0 => self.dim_black,
            1 => self.dim_red,
            2 => self.dim_green,
            3 => self.dim_yellow,
            4 => self.dim_blue,
            5 => self.dim_magenta,
            6 => self.dim_cyan,
            7 => self.dim_white,
            _ => self.dim_black,
        }
    }
}

/// Terminal theme: tokens + precomputed xterm-256 table (built once per light/dark).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerminalTheme {
    pub background: u32,
    pub foreground: u32,
    pub bright_foreground: u32,
    pub dim_foreground: u32,
    pub cursor: u32,
    pub ansi: AnsiPalette,
    pub dim_lightness_factor: f32,
    pub dim_alpha_factor: f32,
    indexed: [u32; 256],
}

impl TerminalTheme {
    pub fn dark() -> Self {
        let background = 0x0e0c0c;
        let foreground = 0xabb2bf;
        let bright_foreground = 0xdce0e5;
        let dim_foreground = 0x636d83;
        let cursor = 0x528bff;
        let ansi = AnsiPalette {
            black: 0x282c34,
            red: 0xe06c75,
            green: 0x98c379,
            yellow: 0xe5c07b,
            blue: 0x61afef,
            magenta: 0xc678dd,
            cyan: 0x56b6c2,
            white: 0xabb2bf,
            bright_black: 0x5c6370,
            bright_red: 0xe06c75,
            bright_green: 0x98c379,
            bright_yellow: 0xe5c07b,
            bright_blue: 0x61afef,
            bright_magenta: 0xc678dd,
            bright_cyan: 0x56b6c2,
            bright_white: 0xffffff,
            dim_black: 0x3b3f4a,
            dim_red: 0xa7545a,
            dim_green: 0x6d8f59,
            dim_yellow: 0xb8985b,
            dim_blue: 0x457cad,
            dim_magenta: 0x8d54a0,
            dim_cyan: 0x3c818a,
            dim_white: 0x8f969b,
        };
        Self::from_parts(
            background,
            foreground,
            bright_foreground,
            dim_foreground,
            cursor,
            ansi,
            1.0,
            0.7,
        )
    }

    pub fn light() -> Self {
        let background = 0xfafafa;
        let foreground = 0x2a2c33;
        let bright_foreground = 0x2a2c33;
        let dim_foreground = 0xbbbbbb;
        let cursor = 0x2f5af3;
        let ansi = AnsiPalette {
            black: 0x000000,
            red: 0xde3e35,
            green: 0x3f953a,
            yellow: 0xd2b67c,
            blue: 0x2f5af3,
            magenta: 0x950095,
            cyan: 0x0997b3,
            white: 0xbbbbbb,
            bright_black: 0x000000,
            bright_red: 0xde3e35,
            bright_green: 0x3f953a,
            bright_yellow: 0xd2b67c,
            bright_blue: 0x2f5af3,
            bright_magenta: 0xa00095,
            bright_cyan: 0x0bbcd6,
            bright_white: 0xffffff,
            dim_black: 0x555555,
            dim_red: 0x9c2b26,
            dim_green: 0x2b6927,
            dim_yellow: 0xa48c5a,
            dim_blue: 0x2140ab,
            dim_magenta: 0x6a006a,
            dim_cyan: 0x0a7b92,
            dim_white: 0x888888,
        };
        Self::from_parts(
            background,
            foreground,
            bright_foreground,
            dim_foreground,
            cursor,
            ansi,
            0.92,
            0.85,
        )
    }

    pub fn one_dark() -> Self {
        Self::dark()
    }

    fn from_parts(
        background: u32,
        foreground: u32,
        bright_foreground: u32,
        dim_foreground: u32,
        cursor: u32,
        ansi: AnsiPalette,
        dim_lightness_factor: f32,
        dim_alpha_factor: f32,
    ) -> Self {
        Self {
            background,
            foreground,
            bright_foreground,
            dim_foreground,
            cursor,
            ansi,
            dim_lightness_factor,
            dim_alpha_factor,
            indexed: build_indexed_table(ansi),
        }
    }

    pub fn is_light(&self) -> bool {
        relative_luminance(self.background) >= 0.5
    }

    /// Focused GPUI cursor alpha; on light backgrounds, high-luminance cursors need less opacity.
    pub fn cursor_focused_alpha(&self) -> f32 {
        if !self.is_light() {
            return 1.0;
        }
        let contrast =
            (relative_luminance(self.cursor) - relative_luminance(self.background)).abs();
        (0.38 + contrast * 0.5).clamp(0.35, 0.65)
    }

    pub fn color_at_index(&self, index: usize) -> AlacRgb {
        match index {
            0..=255 => rgb_from_hex(self.indexed[index]),
            256 => rgb_from_hex(self.foreground),
            257 => rgb_from_hex(self.background),
            258 => rgb_from_hex(self.cursor),
            259..=266 => rgb_from_hex(self.ansi.dim_color((index - 259) as u8)),
            267 => rgb_from_hex(self.bright_foreground),
            268 => rgb_from_hex(self.dim_foreground),
            _ => rgb_from_hex(self.foreground),
        }
    }

    pub fn osc_color_setup_sequence(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(512);
        append_dynamic_color(&mut buf, b"10", self.foreground);
        append_dynamic_color(&mut buf, b"11", self.background);
        append_dynamic_color(&mut buf, b"12", self.cursor);
        for index in 0..16u8 {
            append_palette_color(&mut buf, index, self.indexed[index as usize]);
        }
        buf
    }

    pub fn apply_dim(&self, color: Hsla, dim: bool) -> Hsla {
        if !dim {
            return color;
        }
        gpui::hsla(
            color.h,
            color.s,
            color.l * self.dim_lightness_factor,
            color.a * self.dim_alpha_factor,
        )
    }

    pub fn convert_color(&self, color: &AlacColor) -> Hsla {
        rgb(self.color_hex(color)).into()
    }

    fn color_hex(&self, color: &AlacColor) -> u32 {
        let hex = match color {
            AlacColor::Named(named) => self.named_hex(*named),
            AlacColor::Spec(rgb_color) => {
                pack_rgb(rgb_color.r as u32, rgb_color.g as u32, rgb_color.b as u32)
            }
            AlacColor::Indexed(index) => self.indexed[usize::from(*index)],
        };
        hex
    }

    fn named_hex(self, color: NamedColor) -> u32 {
        let rgb = self.color_at_index(color as usize);
        pack_rgb(rgb.r as u32, rgb.g as u32, rgb.b as u32)
    }
}

fn build_indexed_table(ansi: AnsiPalette) -> [u32; 256] {
    let mut table = [0u32; 256];
    for index in 0..16 {
        table[index] = ansi.color(index as u8);
    }
    for index in 16..232 {
        let idx = index - 16;
        let hex = pack_rgb(
            xterm_cube_channel(idx / 36),
            xterm_cube_channel((idx / 6) % 6),
            xterm_cube_channel(idx % 6),
        );
        table[index] = hex;
    }
    for index in 232..256 {
        let step = (index - 232) as u32;
        let level = step * 10 + 8;
        table[index] = pack_rgb(level, level, level);
    }
    table
}

fn relative_luminance(hex: u32) -> f32 {
    let channel = |c: u32| {
        let x = c as f32 / 255.0;
        if x <= 0.03928 {
            x / 12.92
        } else {
            ((x + 0.055) / 1.055).powf(2.4)
        }
    };
    let r = channel((hex >> 16) & 0xff);
    let g = channel((hex >> 8) & 0xff);
    let b = channel(hex & 0xff);
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

fn append_dynamic_color(buf: &mut Vec<u8>, kind: &[u8], hex: u32) {
    let rgb = rgb_from_hex(hex);
    buf.extend_from_slice(b"\x1b]");
    buf.extend_from_slice(kind);
    buf.extend_from_slice(b";rgb:");
    append_rgb_duplicated(buf, &rgb);
    buf.push(0x07);
}

fn append_palette_color(buf: &mut Vec<u8>, index: u8, hex: u32) {
    let rgb = rgb_from_hex(hex);
    buf.extend_from_slice(b"\x1b]4;");
    append_decimal_u8(buf, index);
    buf.extend_from_slice(b";rgb:");
    append_rgb_duplicated(buf, &rgb);
    buf.push(0x07);
}

fn append_decimal_u8(buf: &mut Vec<u8>, value: u8) {
    if value >= 10 {
        buf.push(b'1');
        buf.push(b'0' + (value - 10));
    } else {
        buf.push(b'0' + value);
    }
}

fn append_rgb_duplicated(buf: &mut Vec<u8>, rgb: &AlacRgb) {
    use std::fmt::Write as _;
    let mut channel = String::with_capacity(4);
    for byte in [rgb.r, rgb.g, rgb.b] {
        channel.clear();
        write!(&mut channel, "{byte:02x}{byte:02x}").expect("fmt");
        buf.extend_from_slice(channel.as_bytes());
        buf.push(b'/');
    }
    buf.pop();
}

pub(crate) fn rgb_from_hex(hex: u32) -> AlacRgb {
    AlacRgb {
        r: ((hex >> 16) & 0xff) as u8,
        g: ((hex >> 8) & 0xff) as u8,
        b: (hex & 0xff) as u8,
    }
}

fn xterm_cube_channel(component: usize) -> u32 {
    match component {
        0 => 0,
        1 => 95,
        2 => 135,
        3 => 175,
        4 => 215,
        _ => 255,
    }
}

fn pack_rgb(r: u32, g: u32, b: u32) -> u32 {
    (r << 16) | (g << 8) | b
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::vte::ansi::Color as AlacColor;

    #[test]
    fn light_named_blue_matches_one_light() {
        let theme = TerminalTheme::light();
        assert_eq!(theme.background, 0xfafafa);
        assert_eq!(theme.foreground, 0x2a2c33);
        assert_eq!(theme.bright_foreground, 0x2a2c33);
        assert_eq!(theme.dim_foreground, 0xbbbbbb);
        assert_eq!(theme.ansi.blue, 0x2f5af3);
        assert_eq!(theme.ansi.dim_blue, 0x2140ab);
        assert_eq!(
            theme.convert_color(&AlacColor::Named(NamedColor::Blue)),
            rgb(0x2f5af3).into()
        );
    }

    #[test]
    fn light_special_color_indexes_match_theme_tokens() {
        let theme = TerminalTheme::light();
        assert_eq!(theme.color_at_index(259), rgb_from_hex(0x555555));
        assert_eq!(theme.color_at_index(263), rgb_from_hex(0x2140ab));
        assert_eq!(theme.color_at_index(267), rgb_from_hex(0x2a2c33));
        assert_eq!(theme.color_at_index(268), rgb_from_hex(0x000000));
    }

    #[test]
    fn light_indexed_cube_uses_standard_xterm_values() {
        let theme = TerminalTheme::light();
        let indexed: Hsla = theme.convert_color(&AlacColor::Indexed(17));
        let expected: Hsla = rgb(pack_rgb(0, 0, 95)).into();
        assert_eq!(indexed, expected);
    }

    #[test]
    fn dark_indexed_cube_uses_standard_xterm_values() {
        let theme = TerminalTheme::dark();
        let indexed: Hsla = theme.convert_color(&AlacColor::Indexed(17));
        let expected: Hsla = rgb(pack_rgb(0, 0, 95)).into();
        assert_eq!(indexed, expected);
    }

    #[test]
    fn light_truecolor_foreground_is_unchanged() {
        let theme = TerminalTheme::light();
        let color = theme.convert_color(&AlacColor::Spec(alacritty_terminal::vte::ansi::Rgb {
            r: 0x9c,
            g: 0xb8,
            b: 0xff,
        }));
        let expected: Hsla = rgb(pack_rgb(0x9c, 0xb8, 0xff)).into();
        assert_eq!(color, expected);
    }

    #[test]
    fn light_truecolor_background_is_unchanged() {
        let theme = TerminalTheme::light();
        let background =
            theme.convert_color(&AlacColor::Spec(alacritty_terminal::vte::ansi::Rgb {
                r: 0xf0,
                g: 0xf0,
                b: 0xf0,
            }));
        let expected: Hsla = rgb(pack_rgb(0xf0, 0xf0, 0xf0)).into();
        assert_eq!(background, expected);
    }

    #[test]
    fn dark_truecolor_is_unchanged() {
        let theme = TerminalTheme::dark();
        let color = theme.convert_color(&AlacColor::Spec(alacritty_terminal::vte::ansi::Rgb {
            r: 0x9c,
            g: 0xb8,
            b: 0xff,
        }));
        let expected: Hsla = rgb(pack_rgb(0x9c, 0xb8, 0xff)).into();
        assert_eq!(color, expected);
    }

    #[test]
    fn light_indexed_grayscale_uses_standard_xterm_ramp() {
        let theme = TerminalTheme::light();
        let indexed = theme.color_at_index(255);
        assert_eq!(indexed, rgb_from_hex(pack_rgb(238, 238, 238)));
    }

    #[test]
    fn light_cursor_alpha_scales_with_cursor_luminance() {
        let light = TerminalTheme::light().cursor_focused_alpha();
        assert!((0.35..=0.65).contains(&light));
        assert_eq!(TerminalTheme::dark().cursor_focused_alpha(), 1.0);
    }
}
