//! Syntax highlighting theme definitions.

use gpui::{rgb, HighlightStyle};
use serde::{Deserialize, Serialize};

use crate::colors::Color;

/// Syntax highlighting colors.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyntaxColors {
    pub keyword: Color,
    pub function: Color,
    pub r#type: Color,
    pub string: Color,
    pub comment: Color,
    pub number: Color,
    pub constant: Color,
    pub property: Color,
    pub operator: Color,
    pub variable: Color,
    pub punctuation: Color,
    pub attribute: Color,
    pub label: Color,
    pub constructor: Color,
    pub tag: Color,
    pub embedded: Color,
    pub link: Color,
    pub emphasis: Color,
    pub strong: Color,
}

impl Default for SyntaxColors {
    fn default() -> Self {
        Self::one_dark()
    }
}

impl SyntaxColors {
    /// One Dark syntax colors.
    pub fn one_dark() -> Self {
        Self {
            keyword: Color::hex(0xc678dd),     // purple
            function: Color::hex(0x61afef),    // blue
            r#type: Color::hex(0xe5c07b),      // yellow
            string: Color::hex(0x98c379),      // green
            comment: Color::hex(0x5c6370),     // gray
            number: Color::hex(0xd19a66),      // orange
            constant: Color::hex(0xd19a66),    // orange
            property: Color::hex(0x56b6c2),    // cyan
            operator: Color::hex(0xc678dd),    // purple
            variable: Color::hex(0xabb2bf),    // foreground
            punctuation: Color::hex(0x636d83), // dim gray
            attribute: Color::hex(0xd19a66),   // orange
            label: Color::hex(0xe06c75),       // red
            constructor: Color::hex(0x61afef), // blue
            tag: Color::hex(0xe06c75),         // red
            embedded: Color::hex(0x98c379),    // green
            link: Color::hex(0x61afef),        // blue
            emphasis: Color::hex(0xc678dd),    // purple (italic)
            strong: Color::hex(0xd19a66),      // orange (bold)
        }
    }

    /// GitHub Dark syntax colors.
    pub fn github_dark() -> Self {
        Self {
            keyword: Color::hex(0xff7b72),
            function: Color::hex(0xd2a8ff),
            r#type: Color::hex(0x79c0ff),
            string: Color::hex(0xa5d6ff),
            comment: Color::hex(0x8b949e),
            number: Color::hex(0x79c0ff),
            constant: Color::hex(0x79c0ff),
            property: Color::hex(0x79c0ff),
            operator: Color::hex(0xff7b72),
            variable: Color::hex(0xc9d1d9),
            punctuation: Color::hex(0x8b949e),
            attribute: Color::hex(0x79c0ff),
            label: Color::hex(0x7ee787),
            constructor: Color::hex(0xd2a8ff),
            tag: Color::hex(0x7ee787),
            embedded: Color::hex(0xa5d6ff),
            link: Color::hex(0x58a6ff),
            emphasis: Color::hex(0xc9d1d9),
            strong: Color::hex(0xc9d1d9),
        }
    }

    /// Dracula syntax colors.
    pub fn dracula() -> Self {
        Self {
            keyword: Color::hex(0xff79c6),     // pink
            function: Color::hex(0x50fa7b),    // green
            r#type: Color::hex(0x8be9fd),      // cyan
            string: Color::hex(0xf1fa8c),      // yellow
            comment: Color::hex(0x6272a4),     // comment
            number: Color::hex(0xbd93f9),      // purple
            constant: Color::hex(0xbd93f9),    // purple
            property: Color::hex(0x8be9fd),    // cyan
            operator: Color::hex(0xff79c6),    // pink
            variable: Color::hex(0xf8f8f2),    // foreground
            punctuation: Color::hex(0x6272a4), // comment
            attribute: Color::hex(0x50fa7b),   // green
            label: Color::hex(0x8be9fd),       // cyan
            constructor: Color::hex(0x8be9fd), // cyan
            tag: Color::hex(0xff79c6),         // pink
            embedded: Color::hex(0xf1fa8c),    // yellow
            link: Color::hex(0x8be9fd),        // cyan
            emphasis: Color::hex(0xf1fa8c),    // yellow
            strong: Color::hex(0xffb86c),      // orange
        }
    }
}

/// Maps tree-sitter capture names to highlight styles.
#[derive(Debug, Clone)]
pub struct SyntaxTheme {
    /// Sorted list of `(capture_prefix, style)` pairs.
    styles: Vec<(String, HighlightStyle)>,
}

impl SyntaxTheme {
    /// Create a syntax theme from syntax colors.
    pub fn from_colors(colors: &SyntaxColors) -> Self {
        let entries = vec![
            ("keyword", colors.keyword),
            ("function", colors.function),
            ("type", colors.r#type),
            ("string", colors.string),
            ("comment", colors.comment),
            ("number", colors.number),
            ("constant", colors.constant),
            ("property", colors.property),
            ("operator", colors.operator),
            ("variable", colors.variable),
            ("punctuation", colors.punctuation),
            ("attribute", colors.attribute),
            ("label", colors.label),
            ("constructor", colors.constructor),
            ("tag", colors.tag),
            ("embedded", colors.embedded),
            ("link", colors.link),
            ("emphasis", colors.emphasis),
            ("strong", colors.strong),
        ];

        let styles = entries
            .into_iter()
            .map(|(name, color)| {
                (
                    name.to_string(),
                    HighlightStyle {
                        color: Some(color.to_hsla()),
                        ..Default::default()
                    },
                )
            })
            .collect();

        Self { styles }
    }

    /// One Dark theme (default).
    pub fn one_dark() -> Self {
        Self::from_colors(&SyntaxColors::one_dark())
    }

    /// GitHub Dark theme.
    pub fn github_dark() -> Self {
        Self::from_colors(&SyntaxColors::github_dark())
    }

    /// Dracula theme.
    pub fn dracula() -> Self {
        Self::from_colors(&SyntaxColors::dracula())
    }

    /// Alias for one_dark() for backwards compatibility.
    pub fn default_dark() -> Self {
        Self::one_dark()
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
        // Dotted captures should fall back to the prefix
        assert!(theme.get("function.method").is_some());
        assert_eq!(
            theme.get("function.method").unwrap().color,
            theme.get("function").unwrap().color
        );
        // Unknown capture
        assert!(theme.get("nonexistent").is_none());
    }
}
