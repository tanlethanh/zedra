// Editor: text buffer, syntax highlighting, code editor, git diff

pub mod code_editor;
pub mod combined_diff_view;
pub mod comment_composer;
pub mod git_diff_view;
pub mod git_sidebar;
pub mod markdown;
pub mod mermaid;
pub mod syntax_highlighter;
pub mod syntax_theme;
pub mod text_buffer;

pub use syntax_highlighter::Language;

/// Horizontal-scroll gesture state shared by virtualized-line views
/// (`code_editor::EditorView`, `combined_diff_view::CombinedDiffView`) whose
/// rows scroll vertically via `uniform_list` but need a separate horizontal
/// pan for lines wider than the viewport.
#[derive(Default)]
pub struct HScrollState {
    /// Horizontal scroll offset in logical pixels.
    pub offset: f32,
    /// True once a gesture has been committed to horizontal scroll. Stays
    /// true until a clearly vertical event overrides it.
    pub active: bool,
}

impl HScrollState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a wheel event in; updates `offset`/`active` and, while active,
    /// re-locks the list's vertical offset to `scroll_y_lock` so a drifting
    /// finger doesn't also scroll the list vertically mid-gesture. Returns
    /// `true` if the horizontal offset changed and the caller should notify.
    pub fn handle_wheel(
        &mut self,
        event: &gpui::ScrollWheelEvent,
        max_line_chars: usize,
        font_size: f32,
        scroll_handle: &gpui::UniformListScrollHandle,
        scroll_y_lock: gpui::Pixels,
    ) -> bool {
        let (delta_x, delta_y) = match event.delta {
            gpui::ScrollDelta::Pixels(p) => (f32::from(p.x), f32::from(p.y)),
            gpui::ScrollDelta::Lines(l) => (l.x * 20.0, l.y * 20.0),
        };
        // Enter H-scroll mode: strict threshold (2.5x, 5px min) to commit.
        // Exit H-scroll mode: a strongly vertical event (3x vertical) overrides.
        // While locked, accept any event with non-zero horizontal delta so a
        // drifting finger doesn't break the scroll mid-gesture.
        if delta_y.abs() > delta_x.abs() * 3.0 {
            self.active = false;
        } else if delta_x.abs() > delta_y.abs() * 2.5 && delta_x.abs() > 5.0 {
            self.active = true;
        }
        if !self.active || delta_x.abs() <= 0.1 {
            return false;
        }

        let char_width = font_size * 0.6;
        let max_offset = (max_line_chars as f32 * char_width).max(0.0);
        self.offset = (self.offset - delta_x).clamp(0.0, max_offset);
        // Undo any vertical drift: the uniform_list overflow scroll already fired
        // (bubble phase, inner first) and may have nudged y. Restore it to the
        // value captured at the start of this render so vertical position is
        // locked for the duration of the horizontal gesture.
        scroll_handle
            .0
            .borrow()
            .base_handle
            .set_offset(gpui::point(gpui::px(0.0), scroll_y_lock));
        true
    }
}

/// Sort highlight ranges by start position, then by specificity (shorter wins),
/// and remove overlapping spans. Required by GPUI's `compute_runs`.
pub fn merge_highlights(
    mut raw: Vec<(std::ops::Range<usize>, gpui::HighlightStyle)>,
) -> Vec<(std::ops::Range<usize>, gpui::HighlightStyle)> {
    raw.sort_by(|a, b| a.0.start.cmp(&b.0.start).then(a.0.len().cmp(&b.0.len())));
    let mut merged = Vec::new();
    let mut cursor = 0usize;
    for (range, style) in raw {
        if range.start >= cursor {
            cursor = range.end;
            merged.push((range, style));
        } else if range.end > cursor {
            merged.push((cursor..range.end, style));
            cursor = range.end;
        }
    }
    merged
}
