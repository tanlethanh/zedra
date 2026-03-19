/// Reusable terminal card UI component.
///
/// Returns a `Stateful<Div>` that the caller can chain event handlers onto:
///
/// ```rust
/// render_terminal_card(props)
///     .on_click(cx.listener(...))
///     .on_long_press(cx.listener(...))
/// ```
///
/// Used in the workspace drawer terminal tab and the quick-action panel.
use gpui::prelude::FluentBuilder;
use gpui::*;

use crate::{fonts, theme};

/// Props that describe how a single terminal card should be rendered.
pub struct TerminalCardProps {
    /// Unique server-assigned terminal ID (used as the GPUI element ID base).
    pub id: String,
    /// 1-based display index: shown as "Terminal N" when no OSC title is available.
    pub index: usize,
    /// Whether this is the currently active / focused terminal.
    pub is_active: bool,
    /// OSC 2 title set by the shell — updates dynamically via PS1 (path) and
    /// preexec hook (running command name when shell integration is active).
    pub title: Option<String>,
    /// OSC 7 working directory (full path; last component shown as subtitle).
    pub cwd: Option<String>,
    /// Shell execution state from OSC 133 marks.
    pub shell_state: zedra_session::ShellState,
    /// Exit code of the last completed command (OSC 133;D).
    pub last_exit_code: Option<i32>,
}

/// Colour of the status dot based on shell state and last exit code.
fn dot_color(shell_state: &zedra_session::ShellState, last_exit_code: Option<i32>) -> u32 {
    use zedra_session::ShellState;
    match shell_state {
        ShellState::Unknown => theme::TEXT_MUTED,
        ShellState::Running => theme::ACCENT_YELLOW,
        ShellState::Idle => match last_exit_code {
            None | Some(0) => theme::ACCENT_GREEN,
            _ => theme::ACCENT_RED,
        },
    }
}

/// Return the last non-empty path component of a CWD string.
fn cwd_last(cwd: &str) -> &str {
    cwd.rfind('/')
        .map(|i| &cwd[i + 1..])
        .filter(|s| !s.is_empty())
        .unwrap_or(cwd)
}

/// Detect a known AI agent from the raw OSC 2 title and return its brand icon path.
/// Returns `None` when the title doesn't match any known agent.
fn agent_icon(title: Option<&str>) -> Option<&'static str> {
    let t = title?.to_ascii_lowercase();
    if t.contains("claude") {
        Some("icons/claude.svg")
    } else if t.contains("opencode") {
        Some("icons/opencode.svg")
    } else if t.contains("codex") || t.contains("openai") {
        Some("icons/openai.svg")
    } else if t.contains("gemini") {
        Some("icons/gemini.svg")
    } else if t.contains("copilot") {
        Some("icons/copilot.svg")
    } else {
        None
    }
}

/// Strip the `user@host:` prefix that default PS1 configs embed in OSC 2 titles.
/// `alice@mybox:~/projects/zedra` → `~/projects/zedra`
/// Returns the original string unchanged if no such prefix is found.
fn strip_ps1_prefix(title: &str) -> &str {
    if let Some(at) = title.find('@') {
        if let Some(colon_offset) = title[at..].find(':') {
            let path = &title[at + colon_offset + 1..];
            if !path.is_empty() {
                return path;
            }
        }
    }
    title
}

/// Render a terminal card element.
///
/// Returns a `Div` — chain `.on_click()` and `.on_long_press()` for tap and
/// long-press actions respectively.
pub fn render_terminal_card(props: TerminalCardProps) -> Stateful<Div> {
    // Primary label: OSC 2 title (stripped of user@host: prefix) — the most
    // dynamic source, updated each prompt and with each command via preexec.
    // Falls back to cwd last component, then to the numbered placeholder.
    let label: SharedString = if let Some(t) = props.title.as_deref() {
        let s = strip_ps1_prefix(t);
        if s.is_empty() {
            SharedString::from(format!("Terminal {}", props.index))
        } else {
            SharedString::from(s.to_owned())
        }
    } else if let Some(cwd) = props.cwd.as_deref() {
        SharedString::from(cwd_last(cwd).to_owned())
    } else {
        SharedString::from(format!("Terminal {}", props.index))
    };

    // Subtitle: cwd last component — stable location anchor shown below the
    // dynamic label.  Always rendered (empty when unavailable) so the card
    // height never changes and the icon stays vertically centred.
    let subtitle: SharedString = props
        .cwd
        .as_deref()
        .map(|p| SharedString::from(cwd_last(p).to_owned()))
        .unwrap_or_default();
    let has_subtitle = !subtitle.is_empty();

    let status_color = dot_color(&props.shell_state, props.last_exit_code);
    let card_id = SharedString::from(format!("term-card-{}", props.id));
    let is_active = props.is_active;
    let icon_path = agent_icon(props.title.as_deref()).unwrap_or("icons/terminal.svg");

    div()
        .id(card_id)
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .mx(px(theme::DRAWER_PADDING))
        .mb(px(6.0))
        .px(px(12.0))
        .py(px(10.0))
        .rounded(px(6.0))
        .bg(rgb(theme::BG_CARD))
        .border_1()
        .border_color(rgb(theme::BORDER_SUBTLE))
        .cursor_pointer()
        // Icon — brand icon for known AI agents, terminal icon otherwise.
        // Colour tied to active state only for visual consistency.
        .child(
            svg()
                .path(icon_path)
                .size(px(theme::ICON_TERMINAL))
                .flex_shrink_0()
                .text_color(if is_active {
                    rgb(theme::TEXT_PRIMARY)
                } else {
                    rgb(theme::TEXT_MUTED)
                }),
        )
        // Text column: always two rows for a fixed card height.
        // min_w_0 lets the flex item shrink below its content width so
        // overflow_hidden + whitespace_nowrap can clip long text.
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.0))
                // Row 1: primary label
                .child(
                    div()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .font_family(fonts::MONO_FONT_FAMILY)
                        .text_color(if is_active {
                            rgb(theme::TEXT_PRIMARY)
                        } else {
                            rgb(theme::TEXT_SECONDARY)
                        })
                        .text_size(px(theme::FONT_BODY))
                        .when(is_active, |s| s.font_weight(FontWeight::MEDIUM))
                        .child(label),
                )
                // Row 2: cwd subtitle — always present to keep card height
                // constant, invisible when no cwd is available.
                .child(
                    div()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .font_family(fonts::MONO_FONT_FAMILY)
                        .text_color(rgb(theme::TEXT_MUTED))
                        .text_size(px(theme::FONT_BODY - 1.0))
                        .when(!has_subtitle, |s| s.invisible())
                        .child(subtitle),
                ),
        )
        // Shell state dot — always shown; colour encodes state.
        .child(
            div()
                .w(px(theme::ICON_STATUS))
                .h(px(theme::ICON_STATUS))
                .flex_shrink_0()
                .rounded(px(3.0))
                .bg(rgb(status_color)),
        )
}
