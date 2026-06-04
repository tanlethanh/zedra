use gpui::HighlightStyle;

/// Maps tree-sitter capture names to highlight styles.
/// Lookup uses longest prefix match so e.g. `"function.method"` matches `"function"`.
#[derive(Clone, Debug, PartialEq)]
pub struct SyntaxTheme {
    styles: Vec<(String, HighlightStyle)>,
}

impl SyntaxTheme {
    pub fn dark() -> Self {
        use gpui::rgb;

        let entries = vec![
            ("keyword", rgb(0xc678dd)),
            ("function", rgb(0x61afef)),
            ("type", rgb(0xe5c07b)),
            ("string", rgb(0x98c379)),
            ("comment", rgb(0x5c6370)),
            ("number", rgb(0xd19a66)),
            ("constant", rgb(0xd19a66)),
            ("property", rgb(0x56b6c2)),
            ("operator", rgb(0xc678dd)),
            ("variable", rgb(0xabb2bf)),
            ("punctuation", rgb(0x636d83)),
            ("attribute", rgb(0xd19a66)),
            ("label", rgb(0xe06c75)),
            ("constructor", rgb(0x61afef)),
            ("tag", rgb(0xe06c75)),
        ];

        Self::from_entries(entries)
    }

    pub fn light() -> Self {
        use gpui::rgb;

        let entries = vec![
            ("keyword", rgb(0xcf222e)),
            ("function", rgb(0x8250df)),
            ("type", rgb(0x953800)),
            ("string", rgb(0x0a3069)),
            ("comment", rgb(0x6e7781)),
            ("number", rgb(0x0550ae)),
            ("constant", rgb(0x0550ae)),
            ("property", rgb(0x116329)),
            ("operator", rgb(0xcf222e)),
            ("variable", rgb(0x24292f)),
            ("punctuation", rgb(0x57606a)),
            ("attribute", rgb(0x0550ae)),
            ("label", rgb(0xcf222e)),
            ("constructor", rgb(0x8250df)),
            ("tag", rgb(0xcf222e)),
        ];

        Self::from_entries(entries)
    }

    fn from_entries(entries: Vec<(&str, gpui::Rgba)>) -> Self {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prefix_match() {
        let theme = SyntaxTheme::dark();
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
