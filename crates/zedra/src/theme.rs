use gpui::{rgb, HighlightStyle, Hsla};

// ============================================================================
// App-wide color constants
// ============================================================================

// Background colors
pub const BG_PRIMARY: u32 = 0x0e0c0c;
pub const BG_CARD: u32 = 0x131313;
pub const BG_OVERLAY: u32 = 0x131313;
pub const BG_SURFACE: u32 = 0x0e0c0c; // Terminal / input field background (matches BG_PRIMARY)

// Text colors
pub const TEXT_PRIMARY: u32 = 0xffffff;
pub const TEXT_SECONDARY: u32 = 0xcacaca;
pub const TEXT_MUTED: u32 = 0x505050;

// Border colors
pub const BORDER_DEFAULT: u32 = 0x505050;
pub const BORDER_SUBTLE: u32 = 0x1a1a1a;

// Accent / button colors
pub const ACCENT_GREEN: u32 = 0x98c379;
pub const ACCENT_BLUE: u32 = 0x61afef;
pub const ACCENT_YELLOW: u32 = 0xe5c07b;
pub const ACCENT_RED: u32 = 0xe06c75;

// Font sizes (pixels) — change these to scale all UI text
pub const FONT_TITLE: f32 = 22.0; // App main title ("Zedra")
pub const FONT_HEADING: f32 = 14.0; // Section headings, dialog titles
pub const FONT_BODY: f32 = 12.0; // Standard UI text: labels, buttons, file names, values
pub const FONT_DETAIL: f32 = 10.0; // Small metadata, code previews, badges

// Icon sizes (pixels)
pub const ICON_NAV: f32 = 18.0; // Drawer nav bar icons
pub const ICON_HEADER: f32 = 20.0; // Header logo / action icons
pub const ICON_FILE: f32 = 12.0; // File tree icons
pub const ICON_FILE_DIR: f32 = 14.0; // Directory icons (slightly larger than file)
pub const ICON_STATUS: f32 = 6.0; // Status dots (connected/disconnected)

// Editor / diff code view constants
pub const EDITOR_FONT_SIZE: f32 = 12.0;
pub const EDITOR_GUTTER_FONT_SIZE: f32 = 11.0;
pub const EDITOR_LINE_HEIGHT: f32 = 15.0;
pub const EDITOR_GUTTER_WIDTH: f32 = 36.0;

// Line number color (white at 30% opacity)
pub fn line_number_color() -> Hsla {
    gpui::hsla(0.0, 0.0, 0.83, 0.3)
}

// Backdrop overlay
pub fn backdrop_color() -> Hsla {
    gpui::hsla(0.0, 0.0, 0.075, 0.6)
}

// Hover highlight (white at 5% opacity)
pub fn hover_bg() -> Hsla {
    gpui::hsla(0.0, 0.0, 1.0, 0.05)
}

// Transport badge background
pub fn badge_bg() -> Hsla {
    gpui::hsla(0.0, 0.0, 0.08, 0.8)
}

// ============================================================================
// Editor colors (One Dark palette)
// ============================================================================

/// One Dark colors used by the code editor UI (backgrounds, gutter, cursor).
pub struct EditorColors {
    pub bg: Hsla,
    pub bg_gutter: Hsla,
    pub bg_current_line: Hsla,
    pub border_subtle: Hsla,
    pub text_primary: Hsla,
    pub text_gutter: Hsla,
    pub text_gutter_active: Hsla,
    pub cursor: Hsla,
    pub status_bar_bg: Hsla,
    pub status_bar_text: Hsla,
}

impl Default for EditorColors {
    fn default() -> Self {
        Self {
            bg: rgb(0x282c34).into(),
            bg_gutter: rgb(0x21252b).into(),
            bg_current_line: rgb(0x2c313c).into(),
            border_subtle: rgb(0x3e4451).into(),
            text_primary: rgb(0xabb2bf).into(),
            text_gutter: rgb(0x4b5263).into(),
            text_gutter_active: rgb(0x737984).into(),
            cursor: rgb(0x528bff).into(),
            status_bar_bg: rgb(0x21252b).into(),
            status_bar_text: rgb(0x5c6370).into(),
        }
    }
}

// ============================================================================
// Language badge colors
// ============================================================================

/// Per-language badge colors shown in the editor status bar.
pub struct LanguageColors;

impl LanguageColors {
    pub fn for_language(name: &str) -> Hsla {
        match name.to_lowercase().as_str() {
            "rust" => rgb(0xdea584).into(),
            "python" => rgb(0x3572a5).into(),
            "go" => rgb(0x00add8).into(),
            "javascript" | "js" => rgb(0xc8a816).into(), // darkened for white text readability
            "typescript" | "ts" | "tsx" => rgb(0x3178c6).into(),
            "c" => rgb(0x555555).into(),
            "c++" | "cpp" => rgb(0xf34b7d).into(),
            "css" => rgb(0x563d7c).into(),
            "json" => rgb(0x3e4451).into(),
            "yaml" => rgb(0xcb171e).into(),
            "bash" | "sh" | "shell" => rgb(0x4a8c2a).into(), // darkened for white text
            "markdown" | "md" => rgb(0x083fa1).into(),
            _ => rgb(0x4b5263).into(),
        }
    }
}

// ============================================================================
// Syntax theme
// ============================================================================

/// Maps tree-sitter capture names to highlight styles.
/// Lookup uses longest prefix match so e.g. `"function.method"` matches `"function"`.
pub struct SyntaxTheme {
    styles: Vec<(String, HighlightStyle)>,
}

impl SyntaxTheme {
    /// One Dark syntax theme (default).
    pub fn one_dark() -> Self {
        let entries: &[(&str, u32)] = &[
            ("keyword", 0xc678dd),
            ("function", 0x61afef),
            ("type", 0xe5c07b),
            ("string", 0x98c379),
            ("comment", 0x5c6370),
            ("number", 0xd19a66),
            ("constant", 0xd19a66),
            ("property", 0x56b6c2),
            ("operator", 0xc678dd),
            ("variable", 0xabb2bf),
            ("punctuation", 0x636d83),
            ("attribute", 0xd19a66),
            ("label", 0xe06c75),
            ("constructor", 0x61afef),
            ("tag", 0xe06c75),
            ("embedded", 0x98c379),
            ("link", 0x61afef),
            ("emphasis", 0xc678dd),
            ("strong", 0xd19a66),
        ];
        Self::from_entries(entries)
    }

    /// Alias for backwards compatibility.
    pub fn default_dark() -> Self {
        Self::one_dark()
    }

    fn from_entries(entries: &[(&str, u32)]) -> Self {
        let styles = entries
            .iter()
            .map(|(name, hex)| {
                (
                    name.to_string(),
                    HighlightStyle {
                        color: Some(rgb(*hex).into()),
                        ..Default::default()
                    },
                )
            })
            .collect();
        Self { styles }
    }

    /// Look up the style for a capture name using longest prefix match.
    pub fn get(&self, capture_name: &str) -> Option<HighlightStyle> {
        let mut name = capture_name;
        loop {
            for (prefix, style) in &self.styles {
                if name == prefix {
                    return Some(*style);
                }
            }
            match name.rfind('.') {
                Some(pos) => name = &name[..pos],
                None => return None,
            }
        }
    }
}

impl Default for SyntaxTheme {
    fn default() -> Self {
        Self::one_dark()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prefix_match() {
        let theme = SyntaxTheme::default();
        assert!(theme.get("keyword").is_some());
        assert!(theme.get("function").is_some());
        assert!(theme.get("function.method").is_some());
        assert_eq!(
            theme.get("function.method").unwrap().color,
            theme.get("function").unwrap().color
        );
        assert!(theme.get("nonexistent").is_none());
    }
}
