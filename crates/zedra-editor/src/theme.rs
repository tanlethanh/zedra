use gpui::HighlightStyle;

/// Maps tree-sitter capture names to highlight styles.
pub struct SyntaxTheme {
    /// Sorted list of `(capture_prefix, style)` pairs. Lookup uses longest
    /// prefix match so that e.g. `"function.method"` matches `"function"`.
    styles: Vec<(String, HighlightStyle)>,
}

impl SyntaxTheme {
    /// A dark theme inspired by One Dark.
    pub fn default_dark() -> Self {
        use gpui::rgb;

        let entries = vec![
            ("keyword", rgb(0xc678dd)),       // purple
            ("function", rgb(0x61afef)),       // blue
            ("type", rgb(0xe5c07b)),           // yellow
            ("string", rgb(0x98c379)),         // green
            ("comment", rgb(0x5c6370)),        // gray
            ("number", rgb(0xd19a66)),         // orange
            ("constant", rgb(0xd19a66)),       // orange
            ("property", rgb(0x56b6c2)),       // cyan
            ("operator", rgb(0xc678dd)),       // purple
            ("variable", rgb(0xabb2bf)),       // foreground
            ("punctuation", rgb(0x636d83)),    // dim gray
            ("attribute", rgb(0xd19a66)),      // orange
            ("label", rgb(0xe06c75)),          // red
            ("constructor", rgb(0x61afef)),    // blue
            ("tag", rgb(0xe06c75)),            // red
        ];

        let styles = entries
            .into_iter()
            .map(|(name, color)| {
                (
                    name.to_string(),
                    HighlightStyle {
                        color: Some(color.into()),
                        ..Default::default()
                    },
                )
            })
            .collect();

        Self { styles }
    }

    /// Look up the style for a capture name using longest prefix match.
    ///
    /// For example, `"function.method"` will match `"function"` if there is no
    /// exact `"function.method"` entry.
    pub fn get(&self, capture_name: &str) -> Option<HighlightStyle> {
        // Try exact match first, then progressively shorter prefixes
        let mut name = capture_name;
        loop {
            for (prefix, style) in &self.styles {
                if name == prefix {
                    return Some(*style);
                }
            }
            // Strip the last `.component` and try again
            match name.rfind('.') {
                Some(pos) => name = &name[..pos],
                None => return None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prefix_match() {
        let theme = SyntaxTheme::default_dark();
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
