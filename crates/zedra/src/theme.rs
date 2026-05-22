use gpui::Hsla;
use zedra_terminal::element::TerminalTheme;

use crate::editor::syntax_theme::SyntaxTheme;

// ---------------------------------------------------------------------------
// Layout and typography (theme-independent)
// ---------------------------------------------------------------------------

pub const DRAWER_PADDING: f32 = 12.0;
pub const SPACING_XS: f32 = 4.0;
pub const SPACING_SM: f32 = 8.0;
pub const SPACING_MD: f32 = 12.0;
pub const SPACING_LG: f32 = 16.0;

pub const HEADER_HEIGHT: f32 = 48.0;
pub const HOME_CARD_WIDTH: f32 = 300.0;
pub const HOME_GUIDE_WIDTH: f32 = 300.0;
pub const CONNECT_DETAIL_WIDTH: f32 = 300.0;
pub const HEADER_BUTTON_SIZE: f32 = 42.0;
pub const DRAWER_ICON_ZONE: f32 = 38.0;
pub const TERMINAL_LINE_HEIGHT: f32 = 16.0;
pub const PANEL_ITEM_HEIGHT: f32 = 28.0;

#[cfg(target_os = "android")]
pub const DRAWER_EDGE_ZONE: f32 = 72.0;
#[cfg(not(target_os = "android"))]
pub const DRAWER_EDGE_ZONE: f32 = 56.0;
pub const DRAWER_VELOCITY_THRESHOLD: f32 = 12.0;
pub const DRAWER_BACKDROP_OPACITY: f32 = 0.4;
pub const DRAWER_DEFAULT_WIDTH: f32 = 295.0;
pub const DRAWER_OPEN_ANIMATION_DURATION_MS: u64 = 160;
pub const DRAWER_CLOSE_ANIMATION_DURATION_MS: u64 = 100;

pub const FONT_APP_TITLE: f32 = 28.0;
pub const FONT_TITLE: f32 = 20.0;
pub const FONT_HEADING: f32 = 13.0;
pub const FONT_BODY: f32 = 12.0;
pub const FONT_DETAIL: f32 = 12.0;

pub const ICON_LOGO: f32 = 20.0;
pub const ICON_LG: f32 = 24.0;
pub const ICON_MD: f32 = 18.0;
pub const ICON_SM: f32 = 16.0;
pub const ICON_XS: f32 = 14.0;
pub const ICON_FILE: f32 = 12.0;
pub const ICON_FILE_DIR: f32 = 14.0;
pub const ICON_STATUS: f32 = 6.0;
pub const ICON_TERMINAL: f32 = 16.0;

pub const EDITOR_FONT_SIZE: f32 = 12.0;
pub const EDITOR_GUTTER_FONT_SIZE: f32 = 11.0;
pub const EDITOR_LINE_HEIGHT: f32 = 15.0;
pub const EDITOR_GUTTER_WIDTH: f32 = 36.0;

// ---------------------------------------------------------------------------
// Theme preference
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemePreference {
    #[default]
    Dark,
    Light,
}

impl ThemePreference {
    pub fn label(self) -> &'static str {
        match self {
            Self::Dark => "Dark",
            Self::Light => "Light",
        }
    }
}

// ---------------------------------------------------------------------------
// UI palette
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct ThemePalette {
    pub bg_primary: u32,
    pub bg_card: u32,
    pub bg_overlay: u32,
    pub bg_surface: u32,
    pub text_primary: u32,
    pub text_secondary: u32,
    pub text_muted: u32,
    pub border_highlight: u32,
    pub border_default: u32,
    pub border_active: u32,
    pub border_subtle: u32,
    pub accent_green: u32,
    pub accent_blue: u32,
    pub accent_yellow: u32,
    pub accent_red: u32,
    pub accent_dim: u32,
    pub git_added: u32,
    pub git_removed: u32,
    pub row_pressed_bg: Hsla,
    pub overlay_backdrop: Hsla,
}

impl ThemePalette {
    pub fn dark() -> Self {
        Self {
            bg_primary: 0x0e0c0c,
            bg_card: 0x131313,
            bg_overlay: 0x131313,
            bg_surface: 0x0e0c0c,
            text_primary: 0xffffff,
            text_secondary: 0xcacaca,
            text_muted: 0x505050,
            border_highlight: 0xcacaca,
            border_default: 0x2c2c2c,
            border_active: 0x505050,
            border_subtle: 0x1a1a1a,
            accent_green: 0x98c379,
            accent_blue: 0x61afef,
            accent_yellow: 0xe5c07b,
            accent_red: 0xe06c75,
            accent_dim: 0x505050,
            git_added: 0x6fc17a,
            git_removed: 0xd57a7a,
            row_pressed_bg: gpui::hsla(0.0, 0.0, 1.0, 0.10),
            overlay_backdrop: gpui::hsla(0.0, 0.0, 0.0, 1.0),
        }
    }

    pub fn light() -> Self {
        Self {
            bg_primary: 0xf5f5f5,
            bg_card: 0xffffff,
            bg_overlay: 0xffffff,
            bg_surface: 0xffffff,
            text_primary: 0x1a1a1a,
            text_secondary: 0x4a4a4a,
            text_muted: 0x8a8a8a,
            border_highlight: 0x4a4a4a,
            border_default: 0xd8d8d8,
            border_active: 0x8a8a8a,
            border_subtle: 0xe8e8e8,
            accent_green: 0x1a7f37,
            accent_blue: 0x0969da,
            accent_yellow: 0x9a6700,
            accent_red: 0xcf222e,
            accent_dim: 0x8a8a8a,
            git_added: 0x1a7f37,
            git_removed: 0xcf222e,
            row_pressed_bg: gpui::hsla(0.0, 0.0, 0.0, 0.06),
            overlay_backdrop: gpui::hsla(0.0, 0.0, 0.0, 1.0),
        }
    }
}

// ---------------------------------------------------------------------------
// Editor + diff
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct DiffTheme {
    pub header_bg: u32,
    pub added_bg: u32,
    pub removed_bg: u32,
    pub header_text: u32,
    pub gutter_text: u32,
    pub body_text: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EditorTheme {
    pub background: u32,
    pub foreground: u32,
    pub pending_syntax: u32,
    pub gutter: Hsla,
    pub syntax: SyntaxTheme,
    pub diff: DiffTheme,
}

impl EditorTheme {
    pub fn dark() -> Self {
        Self {
            background: 0x0e0c0c,
            foreground: 0xabb2bf,
            pending_syntax: 0x7b8494,
            gutter: gpui::hsla(0.0, 0.0, 0.83, 0.3),
            syntax: SyntaxTheme::dark(),
            diff: DiffTheme {
                header_bg: 0x131313,
                added_bg: 0x162016,
                removed_bg: 0x201616,
                header_text: 0x61afef,
                gutter_text: 0x404040,
                body_text: 0xcacaca,
            },
        }
    }

    pub fn light() -> Self {
        Self {
            background: 0xfafafa,
            foreground: 0x24292f,
            pending_syntax: 0x8b949e,
            gutter: gpui::hsla(0.0, 0.0, 0.45, 0.6),
            syntax: SyntaxTheme::light(),
            diff: DiffTheme {
                header_bg: 0xf6f8fa,
                added_bg: 0xdafbe1,
                removed_bg: 0xffebe9,
                header_text: 0x0969da,
                gutter_text: 0x8b949e,
                body_text: 0x24292f,
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Full bundle
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct ThemeBundle {
    pub ui: ThemePalette,
    pub editor: EditorTheme,
    pub terminal: TerminalTheme,
}

impl ThemeBundle {
    pub fn dark() -> Self {
        Self {
            ui: ThemePalette::dark(),
            editor: EditorTheme::dark(),
            terminal: TerminalTheme::dark(),
        }
    }

    pub fn light() -> Self {
        Self {
            ui: ThemePalette::light(),
            editor: EditorTheme::light(),
            terminal: TerminalTheme::light(),
        }
    }

    pub fn for_preference(preference: ThemePreference) -> Self {
        match preference {
            ThemePreference::Dark => Self::dark(),
            ThemePreference::Light => Self::light(),
        }
    }
}

pub fn palette(cx: &gpui::App) -> ThemePalette {
    crate::theme_state::palette(cx)
}

pub fn bundle(cx: &gpui::App) -> ThemeBundle {
    crate::theme_state::bundle(cx)
}

macro_rules! palette_accessor {
    ($name:ident, $field:ident) => {
        pub fn $name(cx: &gpui::App) -> u32 {
            palette(cx).$field
        }
    };
}

palette_accessor!(bg_primary, bg_primary);
palette_accessor!(bg_card, bg_card);
palette_accessor!(bg_overlay, bg_overlay);
palette_accessor!(bg_surface, bg_surface);
palette_accessor!(text_primary, text_primary);
palette_accessor!(text_secondary, text_secondary);
palette_accessor!(text_muted, text_muted);
palette_accessor!(border_highlight, border_highlight);
palette_accessor!(border_default, border_default);
palette_accessor!(border_active, border_active);
palette_accessor!(border_subtle, border_subtle);
palette_accessor!(accent_green, accent_green);
palette_accessor!(accent_blue, accent_blue);
palette_accessor!(accent_yellow, accent_yellow);
palette_accessor!(accent_red, accent_red);
palette_accessor!(accent_dim, accent_dim);
palette_accessor!(git_added, git_added);
palette_accessor!(git_removed, git_removed);

pub fn row_pressed_bg(cx: &gpui::App) -> Hsla {
    palette(cx).row_pressed_bg
}

pub fn overlay_backdrop(cx: &gpui::App) -> Hsla {
    palette(cx).overlay_backdrop
}

pub fn overlay_backdrop_with_opacity(base: Hsla, opacity: f32) -> Hsla {
    gpui::hsla(base.h, base.s, base.l, opacity)
}
