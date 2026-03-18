use std::borrow::Cow;
use std::sync::Once;

static FONTS: &[&[u8]] = &[
    include_bytes!("../assets/fonts/Lora-VariableFont_wght.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono/JetBrainsMonoNLNerdFontMono-Regular.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono/JetBrainsMonoNLNerdFontMono-Bold.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono/JetBrainsMonoNLNerdFontMono-Italic.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono/JetBrainsMonoNLNerdFontMono-BoldItalic.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono/JetBrainsMonoNLNerdFontMono-Medium.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono/JetBrainsMonoNLNerdFontMono-MediumItalic.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono/JetBrainsMonoNLNerdFontMono-SemiBold.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono/JetBrainsMonoNLNerdFontMono-SemiBoldItalic.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono/JetBrainsMonoNLNerdFontMono-Light.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono/JetBrainsMonoNLNerdFontMono-LightItalic.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono/JetBrainsMonoNLNerdFontMono-ExtraBold.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono/JetBrainsMonoNLNerdFontMono-ExtraBoldItalic.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono/JetBrainsMonoNLNerdFontMono-ExtraLight.ttf"),
    include_bytes!(
        "../assets/fonts/JetBrainsMono/JetBrainsMonoNLNerdFontMono-ExtraLightItalic.ttf"
    ),
    include_bytes!("../assets/fonts/JetBrainsMono/JetBrainsMonoNLNerdFontMono-Thin.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono/JetBrainsMonoNLNerdFontMono-ThinItalic.ttf"),
    // Monochrome symbol fallback for ⏺ ⏹ ⏸ ✔ ✘ ★ ⚠ etc.
    include_bytes!("../assets/fonts/NotoSansSymbols2-Regular.ttf"),
];

/// The font family name for app headings (Lora variable serif)
pub const HEADING_FONT_FAMILY: &str = "Lora";

/// The font family name for the embedded monospace font
pub const MONO_FONT_FAMILY: &str = "JetBrainsMonoNL Nerd Font Mono";

/// The font family name for the symbol fallback font
pub const SYMBOL_FONT_FAMILY: &str = "Noto Sans Symbols 2";

static FONT_LOADED: Once = Once::new();

/// Load all embedded fonts into GPUI's text system.
/// This should be called once during app initialization.
pub fn load_fonts(window: &mut gpui::Window) {
    FONT_LOADED.call_once(|| {
        let text_system = window.text_system();
        let fonts: Vec<Cow<'static, [u8]>> = FONTS.iter().map(|&b| Cow::Borrowed(b)).collect();
        let count = fonts.len();
        if let Err(e) = text_system.add_fonts(fonts) {
            log::error!("Failed to load fonts: {:?}", e);
        } else {
            log::info!("Loaded {} font files", count);
        }
    });
}
