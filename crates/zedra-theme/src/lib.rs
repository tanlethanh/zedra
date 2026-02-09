//! Theme and configuration system for Zedra.
//!
//! This crate provides centralized theming and configuration for all Zedra components,
//! making it easy to customize the appearance and behavior of the editor.

mod colors;
mod config;
mod syntax;
mod theme;

pub use colors::{Color, ColorPalette, LanguageColors};
pub use config::EditorConfig;
pub use syntax::SyntaxTheme;
pub use theme::Theme;
