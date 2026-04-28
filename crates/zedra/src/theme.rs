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
pub const BORDER_DEFAULT: u32 = 0x2c2c2c;
pub const BORDER_ACTIVE: u32 = 0x505050;
pub const BORDER_SUBTLE: u32 = 0x1a1a1a;

// Accent / button colors
pub const ACCENT_GREEN: u32 = 0x98c379;
pub const ACCENT_BLUE: u32 = 0x61afef;
pub const ACCENT_YELLOW: u32 = 0xe5c07b;
pub const ACCENT_RED: u32 = 0xe06c75;
pub const ACCENT_DIM: u32 = 0x505050;

// Spacing
pub const DRAWER_PADDING: f32 = 12.0; // Horizontal padding for drawer tab content
pub const SPACING_SM: f32 = 8.0;
pub const SPACING_MD: f32 = 12.0;
pub const SPACING_LG: f32 = 16.0;

// Layout dimensions
pub const HEADER_HEIGHT: f32 = 48.0;
pub const HOME_CARD_WIDTH: f32 = 300.0;
pub const HOME_GUIDE_WIDTH: f32 = 300.0;
pub const CONNECT_DETAIL_WIDTH: f32 = 300.0;
pub const HEADER_BUTTON_SIZE: f32 = 42.0;
pub const DRAWER_ICON_ZONE: f32 = 38.0;
pub const TERMINAL_LINE_HEIGHT: f32 = 16.0;
pub const PANEL_ITEM_HEIGHT: f32 = 28.0;

// Drawer gesture thresholds
pub const DRAWER_EDGE_ZONE: f32 = 56.0;
pub const DRAWER_VELOCITY_THRESHOLD: f32 = 12.0;
pub const DRAWER_BACKDROP_OPACITY: f32 = 0.4;
pub const DRAWER_DEFAULT_WIDTH: f32 = 295.0;
pub const DRAWER_OPEN_ANIMATION_DURATION_MS: u64 = 160;
pub const DRAWER_CLOSE_ANIMATION_DURATION_MS: u64 = 100;

// Font sizes (pixels) — change these to scale all UI text
pub const FONT_APP_TITLE: f32 = 28.0; // App main title ("Zedra")
pub const FONT_TITLE: f32 = 20.0; // Screen title
pub const FONT_HEADING: f32 = 13.0; // Section headings, dialog titles
pub const FONT_BODY: f32 = 12.0; // Standard UI text: labels, buttons, file names, values
pub const FONT_DETAIL: f32 = 12.0; // Small metadata, code previews, badges

// Icon sizes (pixels)
pub const ICON_LOGO: f32 = 20.0;
pub const ICON_LG: f32 = 24.0;
pub const ICON_MD: f32 = 18.0;
pub const ICON_SM: f32 = 16.0;
pub const ICON_FILE: f32 = 12.0; // File tree icons
pub const ICON_FILE_DIR: f32 = 14.0; // Directory icons (slightly larger than file)
pub const ICON_STATUS: f32 = 6.0; // Status dots (connected/disconnected)
pub const ICON_TERMINAL: f32 = 16.0; // Terminal icon

// Editor / diff code view constants
pub const EDITOR_FONT_SIZE: f32 = 12.0; // Code text in editor and diff views
pub const EDITOR_GUTTER_FONT_SIZE: f32 = 11.0; // Line numbers in gutter
pub const EDITOR_LINE_HEIGHT: f32 = 15.0; // Row height for code lines
pub const EDITOR_GUTTER_WIDTH: f32 = 36.0; // Gutter column width

// Line number color (white at 30% opacity)
pub fn line_number_color() -> Hsla {
    gpui::hsla(0.0, 0.0, 0.83, 0.3)
}

// Backdrop overlay
pub fn backdrop_color() -> Hsla {
    gpui::hsla(0.0, 0.0, 0.075, 0.6)
}

// Transport badge background
pub fn badge_bg() -> Hsla {
    gpui::hsla(0.0, 0.0, 0.08, 0.8)
}
