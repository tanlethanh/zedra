// Editor: text buffer, syntax highlighting, code editor, git diff

pub mod code_editor;
pub mod git_diff_view;
pub mod git_sidebar;
pub mod syntax_highlighter;
pub mod syntax_theme;
pub mod text_buffer;

pub use syntax_highlighter::Language;

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
