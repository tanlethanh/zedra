# Zedra Design Notes

## Core Taste

Zedra should feel like a serious tool, not a decorative app.

- Dark, flat, and quiet
- Monotone-first, with color reserved for meaning
- Dense but readable
- Minimal chrome, minimal borders, minimal interruption
- Touch-friendly without looking oversized

The UI should read like a native code workspace on mobile: calm surfaces, compact rows, restrained icons, and subtle separators.

## Color System

Canonical tokens and wiring live in `docs/THEMING.md`. Definitions are in `crates/zedra/src/theme.rs` (`ThemePalette`, dark/light pairs) and `crates/zedra-terminal/src/theme.rs` for the embedded terminal.

New UI must read colors from `theme::palette(cx)` or the `theme::bg_primary(cx)`-style accessors at render time. Do not hardcode hex values in views.

| Role | Token (via `theme::`) |
|------|------------------------|
| Base background | `bg_primary`, `bg_surface` |
| Raised surface | `bg_card`, `bg_overlay` |
| Quiet structure | `border_subtle` |
| Control edge | `border_default`, `border_active` |
| Primary text | `text_primary` |
| Standard text | `text_secondary` |
| Muted metadata | `text_muted` |

Accent colors are semantic only.

- Green: healthy / connected / success (`accent_green`)
- Yellow: connecting / warning (`accent_yellow`)
- Red: failure / destructive (`accent_red`)
- Blue: focus and active input state (`accent_blue`)

Do not use accent color as decoration. Light and dark palettes are paired in `ThemePalette::dark()` / `light()`; keep new tokens in sync in both.

## Layout And Spacing

The spacing rhythm is compact and consistent.

- Small: `8`
- Standard: `12`
- Large: `16`

Preferred visual behavior:

- Favor fixed, stable proportions over airy layouts
- Keep headers and action zones compact
- Use padding for breathing room, not extra framing
- Let scrollable content stay dense and scan-friendly

## Typography

The default voice is monospace-first.

- Most product UI should use the existing mono system
- Standard sizes are small and information-dense
- Prefer weight and color for emphasis over bigger text
- Use truncation before wrapping in tight panels

One-off branding surfaces can break this rule, but workspace UI should not.

## Borders, Radius, And Effects

- Prefer 1px separators over heavy containers
- Use radius sparingly, usually `6` or `8`
- Avoid shadows and depth effects
- Avoid thick outlines
- Avoid stacked borders inside already framed layouts

If a component feels busy, remove a line before adding a new one.

## Interaction Language

- Use native alerts for confirmation and failure
- Use tiny status signals instead of banners when possible
- Keep primary actions obvious but not loud
- Prefer icon-only actions when the meaning is clear
- Keep touch targets forgiving via hit slop, not visual bulk
- Hover is not a current mobile design state; controls must read correctly without hover-only reveals, fills, or tooltips

## Component Rules

### Panels

- Full-height dark surfaces
- Header and footer separated with subtle lines
- No card-within-card feeling unless content truly benefits from grouping

### Cards

- Use `bg_card` from the active palette
- Use a subtle 1px border
- Keep padding tight
- Avoid decorative fills and badges unless they carry state

### Inputs

- Prefer the shared GPUI input component
- Dark surface, subtle border, blue focus
- Placeholder should be muted
- Keep copy short

### Buttons

- Default to chromeless or near-chromeless actions
- Prefer monochrome icons and muted labels
- Only increase contrast when the action is ready or focused
- Avoid heavy filled buttons in workspace views
- Express button state with pressed, selected, active, disabled, text, icon, and border styling rather than hover styling

## Git View Guidance

For git UI specifically:

- Keep the file list dense and flat
- Keep git status marks textual or monochrome
- Put commit controls in a fixed bottom composer
- Use one shared input plus one compact icon button
- Commit actions should feel quiet and precise, not promotional

Recommended direction for commit UI:

- One-line message input
- Small monotone commit icon
- Subtle top separator for the composer area
- No extra card frame around the whole git view

## Do

- Use neutral surfaces from the active palette (dark or light)
- Reuse `theme.rs` tokens; follow `docs/THEMING.md` for editor and terminal
- Reuse existing GPUI controls
- Prefer subtraction over embellishment
- Make state visible with text, opacity, and spacing

## Avoid

- Bright accent-heavy layouts
- Thick borders
- Large floating buttons
- Multiple nested containers
- Decorative gradients, glows, or shadows
- Mixed visual languages inside one panel
