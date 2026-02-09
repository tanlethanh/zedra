//! Color definitions and palettes.

use gpui::{rgb, Hsla, Rgba};
use serde::{Deserialize, Serialize};

/// A color that can be serialized and converted to GPUI color types.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Color {
    /// Hex color value (e.g., 0xRRGGBB)
    Hex(u32),
    /// RGBA color with values 0.0-1.0
    Rgba { r: f32, g: f32, b: f32, a: f32 },
}

impl Color {
    /// Create a color from a hex value (0xRRGGBB).
    pub const fn hex(value: u32) -> Self {
        Color::Hex(value)
    }

    /// Create an RGBA color.
    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Color::Rgba { r, g, b, a }
    }

    /// Convert to GPUI Hsla color.
    pub fn to_hsla(&self) -> Hsla {
        match self {
            Color::Hex(value) => rgb(*value).into(),
            Color::Rgba { r, g, b, a } => Rgba {
                r: *r,
                g: *g,
                b: *b,
                a: *a,
            }
            .into(),
        }
    }
}

impl From<u32> for Color {
    fn from(value: u32) -> Self {
        Color::Hex(value)
    }
}

impl From<Color> for Hsla {
    fn from(color: Color) -> Self {
        color.to_hsla()
    }
}

/// Color palette for UI elements.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorPalette {
    // Backgrounds
    pub bg_primary: Color,
    pub bg_secondary: Color,
    pub bg_editor: Color,
    pub bg_gutter: Color,
    pub bg_current_line: Color,
    pub bg_selection: Color,
    pub bg_status_bar: Color,
    pub bg_panel: Color,
    pub bg_hover: Color,
    pub bg_active: Color,

    // Borders
    pub border_subtle: Color,
    pub border_default: Color,
    pub border_focused: Color,

    // Text
    pub text_primary: Color,
    pub text_secondary: Color,
    pub text_muted: Color,
    pub text_gutter: Color,
    pub text_gutter_active: Color,

    // Accents
    pub accent_primary: Color,
    pub accent_secondary: Color,
    pub cursor: Color,

    // Semantic
    pub success: Color,
    pub warning: Color,
    pub error: Color,
    pub info: Color,
}

impl Default for ColorPalette {
    fn default() -> Self {
        Self::one_dark()
    }
}

impl ColorPalette {
    /// One Dark color palette (default).
    pub fn one_dark() -> Self {
        Self {
            // Backgrounds
            bg_primary: Color::hex(0x1e2127),
            bg_secondary: Color::hex(0x21252b),
            bg_editor: Color::hex(0x282c34),
            bg_gutter: Color::hex(0x21252b),
            bg_current_line: Color::hex(0x2c313c),
            bg_selection: Color::hex(0x3e4451),
            bg_status_bar: Color::hex(0x21252b),
            bg_panel: Color::hex(0x21252b),
            bg_hover: Color::hex(0x2c313a),
            bg_active: Color::hex(0x3e4451),

            // Borders
            border_subtle: Color::hex(0x3e4451),
            border_default: Color::hex(0x4b5263),
            border_focused: Color::hex(0x61afef),

            // Text
            text_primary: Color::hex(0xabb2bf),
            text_secondary: Color::hex(0x5c6370),
            text_muted: Color::hex(0x4b5263),
            text_gutter: Color::hex(0x4b5263),
            text_gutter_active: Color::hex(0x737984),

            // Accents
            accent_primary: Color::hex(0x61afef),
            accent_secondary: Color::hex(0x56b6c2),
            cursor: Color::hex(0x528bff),

            // Semantic
            success: Color::hex(0x98c379),
            warning: Color::hex(0xe5c07b),
            error: Color::hex(0xe06c75),
            info: Color::hex(0x61afef),
        }
    }

    /// Darker variant of One Dark.
    pub fn one_darker() -> Self {
        Self {
            bg_primary: Color::hex(0x1a1d23),
            bg_secondary: Color::hex(0x1e2127),
            bg_editor: Color::hex(0x21252b),
            bg_gutter: Color::hex(0x1e2127),
            bg_current_line: Color::hex(0x282c34),
            ..Self::one_dark()
        }
    }

    /// GitHub Dark theme.
    pub fn github_dark() -> Self {
        Self {
            // Backgrounds
            bg_primary: Color::hex(0x0d1117),
            bg_secondary: Color::hex(0x161b22),
            bg_editor: Color::hex(0x0d1117),
            bg_gutter: Color::hex(0x0d1117),
            bg_current_line: Color::hex(0x161b22),
            bg_selection: Color::hex(0x264f78),
            bg_status_bar: Color::hex(0x161b22),
            bg_panel: Color::hex(0x161b22),
            bg_hover: Color::hex(0x1f2428),
            bg_active: Color::hex(0x2d333b),

            // Borders
            border_subtle: Color::hex(0x30363d),
            border_default: Color::hex(0x484f58),
            border_focused: Color::hex(0x58a6ff),

            // Text
            text_primary: Color::hex(0xc9d1d9),
            text_secondary: Color::hex(0x8b949e),
            text_muted: Color::hex(0x6e7681),
            text_gutter: Color::hex(0x6e7681),
            text_gutter_active: Color::hex(0x8b949e),

            // Accents
            accent_primary: Color::hex(0x58a6ff),
            accent_secondary: Color::hex(0x56d4dd),
            cursor: Color::hex(0x58a6ff),

            // Semantic
            success: Color::hex(0x3fb950),
            warning: Color::hex(0xd29922),
            error: Color::hex(0xf85149),
            info: Color::hex(0x58a6ff),
        }
    }
}

/// Language-specific colors for badges and icons.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanguageColors {
    pub rust: Color,
    pub python: Color,
    pub go: Color,
    pub javascript: Color,
    pub typescript: Color,
    pub c: Color,
    pub cpp: Color,
    pub css: Color,
    pub json: Color,
    pub yaml: Color,
    pub bash: Color,
    pub markdown: Color,
    pub plain_text: Color,
}

impl Default for LanguageColors {
    fn default() -> Self {
        Self {
            rust: Color::hex(0xdea584),
            python: Color::hex(0x3572a5),
            go: Color::hex(0x00add8),
            javascript: Color::hex(0xf1e05a),
            typescript: Color::hex(0x3178c6),
            c: Color::hex(0x555555),
            cpp: Color::hex(0xf34b7d),
            css: Color::hex(0x563d7c),
            json: Color::hex(0x292929),
            yaml: Color::hex(0xcb171e),
            bash: Color::hex(0x89e051),
            markdown: Color::hex(0x083fa1),
            plain_text: Color::hex(0x5c6370),
        }
    }
}

impl LanguageColors {
    /// Get color for a language by name.
    pub fn for_language(&self, language: &str) -> Color {
        match language.to_lowercase().as_str() {
            "rust" => self.rust,
            "python" => self.python,
            "go" => self.go,
            "javascript" | "js" => self.javascript,
            "typescript" | "ts" | "tsx" => self.typescript,
            "c" => self.c,
            "c++" | "cpp" => self.cpp,
            "css" => self.css,
            "json" => self.json,
            "yaml" | "yml" => self.yaml,
            "bash" | "sh" | "shell" => self.bash,
            "markdown" | "md" => self.markdown,
            _ => self.plain_text,
        }
    }
}
