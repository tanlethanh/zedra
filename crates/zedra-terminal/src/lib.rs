pub mod element;
pub mod input;
pub mod keys;
pub mod terminal;
pub mod view;

pub use element::*;
pub use input::*;
pub use keys::*;
pub use terminal::*;
pub use view::*;

use gpui::*;

/// The font family name for the embedded terminal font.
/// The font bytes and loader live in the `zedra` crate (`fonts` module).
pub const MONO_FONT_FAMILY: &str = "JetBrainsMonoNL Nerd Font Mono";

/// The font size for the embedded terminal font.
pub const TERMINAL_FONT_SIZE: Pixels = px(12.0);
