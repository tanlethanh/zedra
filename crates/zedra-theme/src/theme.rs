//! Complete theme definition combining all theme components.

use serde::{Deserialize, Serialize};

use crate::colors::{ColorPalette, LanguageColors};
use crate::config::{EditorConfig, FilePreviewConfig, StatusBarConfig};
use crate::syntax::{SyntaxColors, SyntaxTheme};

/// Complete theme definition for Zedra.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theme {
    /// Theme name.
    pub name: String,

    /// Color palette for UI elements.
    pub colors: ColorPalette,

    /// Syntax highlighting colors.
    pub syntax: SyntaxColors,

    /// Language-specific badge colors.
    pub language_colors: LanguageColors,

    /// Editor configuration.
    #[serde(default)]
    pub editor: EditorConfig,

    /// Status bar configuration.
    #[serde(default)]
    pub status_bar: StatusBarConfig,

    /// File preview configuration.
    #[serde(default)]
    pub file_preview: FilePreviewConfig,
}

impl Default for Theme {
    fn default() -> Self {
        Self::one_dark()
    }
}

impl Theme {
    /// One Dark theme (default).
    pub fn one_dark() -> Self {
        Self {
            name: "One Dark".to_string(),
            colors: ColorPalette::one_dark(),
            syntax: SyntaxColors::one_dark(),
            language_colors: LanguageColors::default(),
            editor: EditorConfig::default(),
            status_bar: StatusBarConfig::default(),
            file_preview: FilePreviewConfig::default(),
        }
    }

    /// One Darker theme variant.
    pub fn one_darker() -> Self {
        Self {
            name: "One Darker".to_string(),
            colors: ColorPalette::one_darker(),
            ..Self::one_dark()
        }
    }

    /// GitHub Dark theme.
    pub fn github_dark() -> Self {
        Self {
            name: "GitHub Dark".to_string(),
            colors: ColorPalette::github_dark(),
            syntax: SyntaxColors::github_dark(),
            language_colors: LanguageColors::default(),
            editor: EditorConfig::default(),
            status_bar: StatusBarConfig::default(),
            file_preview: FilePreviewConfig::default(),
        }
    }

    /// Get the syntax theme for this theme.
    pub fn syntax_theme(&self) -> SyntaxTheme {
        SyntaxTheme::from_colors(&self.syntax)
    }

    /// Load theme from JSON string.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Serialize theme to JSON string.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Serialize theme to JSON string (compact).
    pub fn to_json_compact(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

/// Global theme provider.
///
/// In a real application, this would be stored in GPUI's global state.
/// For now, it provides a simple way to access the current theme.
#[derive(Debug, Clone)]
pub struct ThemeProvider {
    theme: Theme,
}

impl Default for ThemeProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl ThemeProvider {
    /// Create a new theme provider with the default theme.
    pub fn new() -> Self {
        Self {
            theme: Theme::default(),
        }
    }

    /// Create a theme provider with a specific theme.
    pub fn with_theme(theme: Theme) -> Self {
        Self { theme }
    }

    /// Get a reference to the current theme.
    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    /// Set the current theme.
    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
    }

    /// Get the color palette.
    pub fn colors(&self) -> &ColorPalette {
        &self.theme.colors
    }

    /// Get the syntax theme.
    pub fn syntax_theme(&self) -> SyntaxTheme {
        self.theme.syntax_theme()
    }

    /// Get the editor configuration.
    pub fn editor_config(&self) -> &EditorConfig {
        &self.theme.editor
    }

    /// Get the language colors.
    pub fn language_colors(&self) -> &LanguageColors {
        &self.theme.language_colors
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_theme_serialization() {
        let theme = Theme::one_dark();
        let json = theme.to_json().unwrap();
        let loaded = Theme::from_json(&json).unwrap();
        assert_eq!(loaded.name, theme.name);
    }

    #[test]
    fn test_theme_variants() {
        let _one_dark = Theme::one_dark();
        let _one_darker = Theme::one_darker();
        let _github_dark = Theme::github_dark();
    }
}
