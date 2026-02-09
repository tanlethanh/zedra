mod buffer;
mod editor_view;
mod highlighter;

pub use buffer::Buffer;
pub use editor_view::EditorView;
pub use highlighter::{Highlighter, Language};

// Re-export theme types from zedra-theme
pub use zedra_theme::{
    Color, ColorPalette, EditorConfig, LanguageColors, SyntaxTheme, Theme, ThemeProvider,
};
