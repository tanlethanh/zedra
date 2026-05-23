//! Terminal color tokens and xterm-256 tables for light/dark appearance.
//!
//! Product UI chrome uses `zedra::theme::ThemePalette`; terminal ANSI/truecolor uses this module only.
//! See `docs/THEMING.md`.

use alacritty_terminal::vte::ansi::{Color as AlacColor, NamedColor, Rgb as AlacRgb};
use gpui::{Hsla, rgb};

/// Minimum relative-luminance gap between a foreground and the light background.
const LIGHT_FG_LUMINANCE_DELTA: f32 = 0.35;

/// Standard 16 ANSI terminal colors — edit these for contrast; the 256 table is derived.
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
}

/// Terminal theme: tokens + precomputed xterm-256 table (built once per light/dark).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerminalTheme {
    pub background: u32,
    pub foreground: u32,
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
        };
        Self::from_parts(background, foreground, cursor, ansi, 1.0, 0.7, false)
    }

    pub fn light() -> Self {
        let background = 0xfafafa;
        let foreground = 0x1f2328;
        let cursor = 0x0969da;
        let ansi = AnsiPalette {
            black: 0x1f2328,
            red: 0xb62324,
            green: 0x116329,
            yellow: 0x633c01,
            blue: 0x012f7a,
            magenta: 0x512a93,
            cyan: 0x024a6b,
            white: 0x32383f,
            bright_black: 0x32383f,
            bright_red: 0x8c1921,
            bright_green: 0x0a4d22,
            bright_yellow: 0x573400,
            bright_blue: 0x012152,
            bright_magenta: 0x3f1f6e,
            bright_cyan: 0x023b5a,
            bright_white: 0x1f2328,
        };
        Self::from_parts(background, foreground, cursor, ansi, 0.92, 0.85, true)
    }

    pub fn one_dark() -> Self {
        Self::dark()
    }

    fn from_parts(
        background: u32,
        foreground: u32,
        cursor: u32,
        ansi: AnsiPalette,
        dim_lightness_factor: f32,
        dim_alpha_factor: f32,
        light: bool,
    ) -> Self {
        Self {
            background,
            foreground,
            cursor,
            ansi,
            dim_lightness_factor,
            dim_alpha_factor,
            indexed: build_indexed_table(background, ansi, light),
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
            259..=266 => dim_rgb(rgb_from_hex(self.ansi.color((index - 259) as u8))),
            267 => rgb_from_hex(self.foreground),
            268 => dim_rgb(rgb_from_hex(self.foreground)),
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
        let hex = match color {
            AlacColor::Named(named) => self.named_hex(*named),
            AlacColor::Spec(rgb_color) => {
                pack_rgb(rgb_color.r as u32, rgb_color.g as u32, rgb_color.b as u32)
            }
            AlacColor::Indexed(index) => self.indexed[usize::from(*index)],
        };
        let mut hsla: Hsla = rgb(hex).into();
        if matches!(color, AlacColor::Spec(_)) && self.is_light() {
            hsla = mute_truecolor_on_light(hsla, self.foreground);
        }
        hsla
    }

    fn named_hex(self, color: NamedColor) -> u32 {
        let rgb = self.color_at_index(color as usize);
        pack_rgb(rgb.r as u32, rgb.g as u32, rgb.b as u32)
    }
}

fn build_indexed_table(background: u32, ansi: AnsiPalette, light: bool) -> [u32; 256] {
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
        table[index] = if light {
            ensure_readable_on_light(hex, background)
        } else {
            hex
        };
    }
    for index in 232..256 {
        let step = (index - 232) as u32;
        let level = if light {
            (96 + step * 4).min(176)
        } else {
            step * 10 + 8
        };
        table[index] = pack_rgb(level, level, level);
    }
    table
}

fn ensure_readable_on_light(fg: u32, bg: u32) -> u32 {
    const MAX_PASSES: u8 = 8;
    let bg_lum = relative_luminance(bg);
    let mut current = fg;
    for _ in 0..MAX_PASSES {
        // On light backgrounds, readable indexed colors should be darker than the background.
        if bg_lum - relative_luminance(current) >= LIGHT_FG_LUMINANCE_DELTA {
            return current;
        }
        current = scale_rgb(current, 0.55);
    }
    current
}

fn scale_rgb(hex: u32, scale: f32) -> u32 {
    let r = ((hex >> 16) as f32 * scale) as u32;
    let g = (((hex >> 8) & 0xff) as f32 * scale) as u32;
    let b = ((hex & 0xff) as f32 * scale) as u32;
    pack_rgb(r, g, b)
}

/// Claude-style 24-bit pastels: softer than neon, separate from indexed-token contrast rules.
fn mute_truecolor_on_light(color: Hsla, foreground: u32) -> Hsla {
    let fg: Hsla = rgb(foreground).into();
    let l = color.l;
    let s = color.s;
    if l <= 0.48 && s < 0.45 {
        return gpui::hsla(color.h, s * 0.9, l, color.a);
    }
    let new_l = (l * 0.5 + 0.43).clamp(0.38, 0.48);
    let new_s = (s * 0.68).clamp(0.22, 0.58);
    let softened = gpui::hsla(color.h, new_s, new_l, color.a);
    blend_rgb(softened, fg, 0.1)
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

fn blend_rgb(a: Hsla, b: Hsla, t: f32) -> Hsla {
    let t = t.clamp(0.0, 1.0);
    let a: gpui::Rgba = a.into();
    let b: gpui::Rgba = b.into();
    let channel = |x: f32, y: f32| ((x + (y - x) * t) * 255.0).round().clamp(0.0, 255.0) as u32;
    rgb(pack_rgb(
        channel(a.r, b.r),
        channel(a.g, b.g),
        channel(a.b, b.b),
    ))
    .into()
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

fn dim_rgb(color: AlacRgb) -> AlacRgb {
    AlacRgb {
        r: color.r / 2,
        g: color.g / 2,
        b: color.b / 2,
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
    fn light_named_blue_uses_palette_token() {
        let theme = TerminalTheme::light();
        let blue: Hsla = theme.convert_color(&AlacColor::Named(NamedColor::Blue));
        assert_eq!(theme.ansi.blue, 0x012f7a);
        assert!(
            blue.l < 0.45,
            "light blue token should be readable, got {}",
            blue.l
        );
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
    fn light_truecolor_is_softened_not_neon() {
        let theme = TerminalTheme::light();
        let raw: Hsla = rgb(pack_rgb(0x9c, 0xb8, 0xff)).into();
        let color = theme.convert_color(&AlacColor::Spec(alacritty_terminal::vte::ansi::Rgb {
            r: 0x9c,
            g: 0xb8,
            b: 0xff,
        }));
        assert!(color.l < raw.l);
        assert!(color.l >= 0.38 && color.l <= 0.48);
        assert!(color.s < raw.s);
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
    fn light_indexed_cube_meets_luminance_delta() {
        let theme = TerminalTheme::light();
        let bg_lum = relative_luminance(theme.background);
        for index in 16..232 {
            let hex = theme.color_at_index(index);
            let packed = pack_rgb(hex.r as u32, hex.g as u32, hex.b as u32);
            let fg_lum = relative_luminance(packed);
            let delta = if fg_lum > bg_lum {
                fg_lum - bg_lum
            } else {
                bg_lum - fg_lum
            };
            assert!(
                delta >= LIGHT_FG_LUMINANCE_DELTA,
                "indexed {index} too low contrast on light background"
            );
        }
    }

    #[test]
    fn light_cursor_alpha_scales_with_cursor_luminance() {
        let light = TerminalTheme::light().cursor_focused_alpha();
        assert!((0.35..=0.65).contains(&light));
        assert_eq!(TerminalTheme::dark().cursor_focused_alpha(), 1.0);
    }
}
