# Theming

Zedra supports **dark** and **light** appearance. New product UI must use the shared theme tokens so colors stay consistent and update when the user toggles appearance or when the app follows the system theme on launch.

Read `docs/DESIGN.md` for visual tone (density, typography, when to use accent color). This doc covers **where tokens live** and **how to wire them in code**.

## Source Of Truth

| Layer | Location | Role |
|-------|----------|------|
| App settings + appearance | `crates/zedra/src/settings.rs` | Loads/saves app settings; `ThemeState` manages `ThemePreference`, builds `ThemeBundle`, emits `ThemeStateEvent::Changed` |
| Token definitions | `crates/zedra/src/theme.rs` | `ThemePalette`, `EditorTheme`, `ThemeBundle`, layout constants, `theme::palette(cx)` accessors |
| Terminal palette | `crates/zedra-terminal/src/theme.rs` | `TerminalTheme`, ANSI + xterm-256 table, truecolor rules for light mode |
| User control | Settings → Appearance | Calls `ThemeState::set_preference` |

`ThemeState` is registered as a GPUI global at app startup (`ZedraApp` in `crates/zedra/src/app.rs`). It is the appearance-specific state inside the settings module. Views read the active bundle through `theme::palette(cx)` / `theme::bundle(cx)`, which delegate to the global entity.

```
Settings / system on launch
        ↓
   ThemeState (preference + ThemeBundle)
        ↓
   ┌────┴────┬──────────────┐
   │         │              │
GPUI views  Editor        Terminal (zedra-terminal)
theme::*    EditorTheme   TerminalTheme
```

## Rules For New UI

**Required**

- Read **UI colors** from `theme::palette(cx)` or the `theme::bg_primary(cx)`-style accessors in `crates/zedra/src/theme.rs`. Wrap hex tokens with `rgb(...)` (or `Hsla` fields on the palette) in `render()`.
- Use **layout and typography constants** from `theme.rs` (`SPACING_*`, `SUBSCREEN_PADDING_X`, `FONT_*`, `ICON_*`, `DRAWER_*`, etc.). Those are appearance-independent; do not duplicate magic numbers in views.
- Subscreen pages (manage agents, agent history): horizontal gutter `SUBSCREEN_PADDING_X` on header and `subscreen_padded_body`; `SPACING_MD` between back and title. Manage detail metadata row spacing: `AGENT_METADATA_ROW_PY`.
- Use **semantic accents** only for meaning (connected, warning, destructive, focus)—see `docs/DESIGN.md`.
- If a surface must react to a theme toggle, subscribe to `ThemeStateEvent::Changed` and call `cx.notify()` on the owning entity (see `ZedraApp::on_theme_changed` and `WorkspaceTerminal`).

**Forbidden in view `render()` and leaf components**

- Hardcoded `0xRRGGBB` / `rgb(0x...)` for product chrome, text, borders, or backgrounds.
- Per-component light/dark branches (`if light { ... } else { ... }`) unless you are implementing theme infrastructure itself.
- Render-time hacks that only fix contrast for one screen (tint overlays, one-off HSLA nudges). Add or adjust a token in `theme.rs` (or terminal `theme.rs`) instead.

**Exceptions**

- Illustrations, third-party assets, or platform-provided colors outside GPUI control.
- Editor syntax highlighting: use `EditorTheme` / `SyntaxTheme` from `theme::bundle(cx).editor`, not `ThemePalette`.
- Terminal cell colors: use `TerminalTheme` via `TerminalView::set_terminal_theme`, not `ThemePalette`. The emulator answers OSC color queries from `TerminalTheme` and paints through `convert_color`.

## Workspace UI (GPUI)

During `render`, pass `cx` and use accessors:

```rust
use crate::theme;

div()
    .bg(rgb(theme::bg_primary(cx)))
    .child(
        Label::new("Title")
            .text_color(rgb(theme::text_primary(cx)))
            .text_size(px(theme::FONT_BODY)),
    )
```

When several fields are needed together (e.g. badges), take `let palette = theme::palette(cx);` once.

`ThemePalette` fields:

| Field | Typical use |
|-------|-------------|
| `bg_primary`, `bg_surface` | Full-height shells, workspace background |
| `bg_card`, `bg_overlay` | Raised panels, terminal cards, sheets |
| `bg_card_dim` | Lower-contrast card fill (closer to `bg_primary` than `bg_card`) |
| `text_primary`, `text_secondary`, `text_muted` | Labels, body, metadata |
| `border_subtle`, `border_default`, `border_active`, `border_highlight` | 1px separators and control edges |
| `accent_green`, `accent_blue`, `accent_yellow`, `accent_red`, `accent_dim` | Status and semantic emphasis only |
| `git_added`, `git_removed` | Git diff marks |
| `row_pressed_bg`, `overlay_backdrop` | Pressed rows, modal backdrops (`Hsla`) |

Adding a new UI color: extend `ThemePalette` in `theme.rs` for both `dark()` and `light()`, add an accessor if it will be used widely, then use it from views. Do not add parallel constants outside `theme.rs`.

## Editor

Editor views take `EditorTheme` from `theme::bundle(cx).editor` and push it into the editor entity:

- `CodeEditor::sync_editor_theme`
- `GitDiffView::sync_editor_theme`

Subscribe to `ThemeStateEvent::Changed` on the same entity that owns the editor (or a parent that can reach it) and re-sync after `theme::bundle(cx)` updates.

Syntax colors live in `crates/zedra/src/editor/syntax_theme.rs` and are selected inside `EditorTheme::dark()` / `light()`.

## Terminal

Terminal colors are **not** `ThemePalette`. They flow through `ThemeBundle::terminal` (`TerminalTheme` in `crates/zedra-terminal/src/theme.rs`).

- `WorkspaceTerminal::sync_terminal_theme` calls `TerminalView::set_terminal_theme`.
- GPUI painting uses `TerminalTheme::convert_color` in `zedra-terminal`’s element layer.
- OSC 10/11/12 and palette queries are answered from `TerminalTheme` via `ColorRequest` (see `docs/MANUAL_TEST.md` §22).

To tune light terminal contrast, edit tokens and derived tables in `crates/zedra-terminal/src/theme.rs` only—not in `element.rs` or `terminal.rs` render paths.

## Subscribing To Theme Changes

**App shell** — `ZedraApp` subscribes to `ThemeState` and calls `cx.notify()` so top-level screens re-render.

**Feature-owned surfaces** — subscribe where the entity owns the subtree that must update:

```rust
cx.subscribe(&theme_state, |this, _, _: &ThemeStateEvent, cx| {
    this.sync_terminal_theme(cx); // or sync_editor_theme, or cx.notify()
});
```

Call the sync helper once after subscribe setup so the initial preference is applied.

Changing preference:

```rust
theme_state.update(cx, |state, cx| {
    state.set_preference(ThemePreference::Light, cx);
});
```

Persistence is handled inside `ThemeState`. `None` in settings means “follow system on next launch.”

## Checklist For New Screens

1. All backgrounds, text, and borders use `theme::` accessors (or `theme::palette(cx)` fields).
2. Spacing and font sizes use `theme::` layout constants.
3. Parent entity subscribes to `ThemeStateEvent::Changed` if the screen is not under `ZedraApp`’s notify path.
4. Editor or terminal child, if any, has a `sync_*_theme` on create and on theme change.
5. Manual steps added to `docs/MANUAL_TEST.md` if the change is visible on device (toggle light/dark).

## Related Docs

- `docs/DESIGN.md` — visual tone and component patterns
- `docs/CONVENTIONS.md` — GPUI `render()` purity and UI design pointer
- `docs/MANUAL_TEST.md` §22 — terminal appearance verification
- `crates/zedra/AGENTS.md` — crate-level UI rules
