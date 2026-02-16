use gpui::Hsla;

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
