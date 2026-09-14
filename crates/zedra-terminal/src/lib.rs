pub mod element;
pub mod input;
pub mod keyboard_accessory;
pub mod keys;
mod selection;
pub mod terminal;
pub mod theme;
pub mod view;
pub mod zoom_ratchet;

pub use element::{TerminalElement, TerminalElementLayout};
pub use input::*;
pub use keyboard_accessory::*;
pub use keys::*;
pub use terminal::*;
pub use theme::{AnsiPalette, TerminalTheme};
pub use view::*;
pub use zoom_ratchet::{TerminalFontSize, TerminalZoomRatchet, ZoomStep};

use gpui::*;

/// The font family name for the embedded terminal font.
/// The font bytes and loader live in the `zedra` crate (`fonts` module).
pub const MONO_FONT_FAMILY: &str = "JetBrainsMonoNL Nerd Font Mono";

/// Shared terminal font descriptor used by both grid measurement and paint.
pub fn terminal_font() -> gpui::Font {
    gpui::Font {
        family: MONO_FONT_FAMILY.into(),
        features: gpui::FontFeatures::default(),
        fallbacks: Some(gpui::FontFallbacks::from_fonts(vec![
            "Noto Sans Symbols 2".to_string(),
            "Apple Symbols".to_string(),
            "Menlo".to_string(),
            "Droid Sans Mono".to_string(),
            "monospace".to_string(),
        ])),
        weight: gpui::FontWeight::NORMAL,
        style: gpui::FontStyle::Normal,
    }
}

/// The font size for the embedded terminal font.
pub const TERMINAL_FONT_SIZE: Pixels = px(12.0);
