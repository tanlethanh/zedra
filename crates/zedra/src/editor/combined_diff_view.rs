//! CombinedDiffView - Single scrolling buffer across all changed files.
//!
//! Replaces the old single-file GitDiffView. Renders Staged/Unstaged/Untracked
//! FileDiffs as one virtualized list with inline file-title separators, and
//! exposes the native selection area used by the Mention/Comment actions.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;

use gpui::{prelude::FluentBuilder as _, *};
use tracing::info;
use zedra_session::SessionHandle;

use super::code_editor::line_range_for_selection_lines;
use super::git_diff_view::{DiffLine, DiffLineKind, FileDiff, parse_unified_diff};
use super::git_sidebar::GitFileSection;
use super::syntax_highlighter::Highlighter;
use crate::platform_bridge;
use crate::theme::{self, EditorTheme};
use crate::workspace_action::{AddSelectionComment, AddSelectionMention};
use crate::workspace_editor::EditorSelection;

/// Native selection area id for the combined diff view. Only Mention/Comment
/// are registered here — "Add to Chat" stays scoped to code_editor/markdown.
pub const DIFF_SELECTION_AREA_ID: &str = "combined-diff-view-selection";

/// Skip files whose diff content exceeds this size (matches the old
/// single-file GitDiffView's guard).
const MAX_DIFF_BYTES: usize = 200 * 1024;

/// How many files on each side of the active (scrolled-to) file are
/// prefetched eagerly, so a small scroll doesn't have to wait on an RPC.
const PREFETCH_WINDOW: usize = 2;

/// One changed file, tagged with the sidebar section it came from. `file` is
/// a placeholder (empty hunks, `old_path`/`new_path` both set to the file's
/// path) until `loaded` — content is fetched lazily, only for files near
/// where the user is actually looking, instead of the whole workspace diff
/// upfront (a session with many changed files would otherwise pay for N
/// RPC round trips it may never scroll to).
#[derive(Clone, Debug)]
pub struct DiffFileEntry {
    pub file: FileDiff,
    pub section: GitFileSection,
    pub loaded: bool,
}

#[derive(Clone)]
enum RowKind {
    /// Blank breathing room before a file block, so a new file is obvious at
    /// a glance while scrolling. Omitted before the very first file.
    Spacer,
    /// First row-slot of a file header block — blank but for background.
    /// `uniform_list` requires uniform row heights, so a taller header is
    /// built from two stacked same-height rows; the visible label lives in
    /// `FileHeaderTail`, absolutely positioned to overlap both rows and
    /// center within their combined height (see that variant).
    FileHeader,
    FileHeaderTail {
        added: usize,
        removed: usize,
    },
    /// Body of a file not yet fetched — see `DiffFileEntry::loaded`.
    LoadingPlaceholder,
    Line(DiffLine),
}

#[derive(Clone)]
struct CachedRow {
    file_index: usize,
    kind: RowKind,
    highlights: Vec<(Range<usize>, HighlightStyle)>,
    content: String,
    status: FileStatus,
    section: GitFileSection,
}

/// A file's change kind, independent of which section (Staged/Unstaged/
/// Untracked) it's in — drives the header's accent color so added/removed/
/// modified files are visually distinct regardless of section.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FileStatus {
    /// New file (untracked, or `old_path` empty/`/dev/null`).
    Added,
    /// Deleted file (`new_path` empty/`/dev/null`).
    Removed,
    Modified,
}

fn file_status(file: &FileDiff) -> FileStatus {
    let old_empty = file.old_path.is_empty() || file.old_path == "/dev/null";
    let new_empty = file.new_path.is_empty() || file.new_path == "/dev/null";
    if old_empty {
        FileStatus::Added
    } else if new_empty {
        FileStatus::Removed
    } else {
        FileStatus::Modified
    }
}

/// Accent color for a file's full-height header stripe and title text:
/// added = green, removed = red, modified = blue (staged) or yellow
/// (unstaged) — independent of section for Added/Removed since those read
/// the same regardless of which section the file is in.
fn status_accent(status: FileStatus, section: GitFileSection, cx: &App) -> Rgba {
    match status {
        FileStatus::Added => rgb(theme::git_added(cx)),
        FileStatus::Removed => rgb(theme::git_removed(cx)),
        FileStatus::Modified => match section {
            GitFileSection::Staged => rgb(theme::accent_blue(cx)),
            GitFileSection::Unstaged | GitFileSection::Untracked => rgb(theme::accent_yellow(cx)),
        },
    }
}

// ── CombinedDiffView ─────────────────────────────────────────────────────────

const LINE_HEIGHT: f32 = theme::EDITOR_LINE_HEIGHT;
const GUTTER_WIDTH: f32 = theme::EDITOR_GUTTER_WIDTH;
const FONT_SIZE: f32 = theme::EDITOR_FONT_SIZE;
const GUTTER_FONT_SIZE: f32 = theme::EDITOR_GUTTER_FONT_SIZE;
const BOTTOM_INSET_MIN: f32 = 100.0;
/// A file header block is two stacked `uniform_list` rows tall (see
/// `RowKind::FileHeader`/`FileHeaderTail`).
const HEADER_BLOCK_HEIGHT: f32 = LINE_HEIGHT * 2.0;

pub struct CombinedDiffView {
    files: Vec<DiffFileEntry>,
    /// Row index of each file's header row, parallel to `files`.
    file_row_start: Vec<usize>,
    editor_theme: EditorTheme,
    scroll_handle: UniformListScrollHandle,
    focus_handle: FocusHandle,
    cached_rows: Rc<Vec<CachedRow>>,
    rows_dirty: bool,
    h_scroll: super::HScrollState,
    max_line_chars: usize,
    pending_scroll_to: Option<String>,
    session_handle: SessionHandle,
    /// Indices into `files` with an in-flight `git_diff` fetch.
    loading: HashSet<usize>,
    /// Bumped by `set_files`; lets a stale in-flight fetch from a prior file
    /// list recognize it no longer applies and skip writing to `files`.
    generation: u64,
    /// Indices into `files` whose diff body is collapsed (header stays,
    /// lines hidden) via the chevron in the file header.
    collapsed: HashSet<usize>,
    /// Per-file syntax-highlighted line rows, keyed by file index — building
    /// these re-parses and re-highlights every line, so this cache lets
    /// `rebuild_row_cache` reuse it across collapse toggles and other
    /// `rows_dirty` triggers that don't touch a file's content. Cleared on
    /// `set_files` and on editor-theme change (highlights bake in colors).
    line_cache: HashMap<usize, Rc<(Vec<CachedRow>, usize)>>,
}

impl CombinedDiffView {
    pub fn new(session_handle: SessionHandle, cx: &mut App) -> Self {
        Self {
            files: Vec::new(),
            file_row_start: Vec::new(),
            editor_theme: EditorTheme::dark(),
            scroll_handle: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            cached_rows: Rc::new(Vec::new()),
            rows_dirty: true,
            h_scroll: super::HScrollState::new(),
            max_line_chars: 0,
            pending_scroll_to: None,
            session_handle,
            loading: HashSet::new(),
            generation: 0,
            collapsed: HashSet::new(),
            line_cache: HashMap::new(),
        }
    }

    /// Toggle whether `file_index`'s diff body is shown or just its header.
    fn toggle_collapsed(&mut self, file_index: usize, cx: &mut Context<Self>) {
        if !self.collapsed.remove(&file_index) {
            self.collapsed.insert(file_index);
        }
        self.rows_dirty = true;
        // Collapsing/expanding removes or restores rows below this file's
        // header, shifting every later row's position — without re-pinning,
        // the scroll offset would keep pointing at the same *pixel* it did
        // before, landing on whatever row now happens to sit there instead
        // of keeping this file's title stuck at the top where it was.
        if let Some(entry) = self.files.get(file_index) {
            let path = if !entry.file.new_path.is_empty() {
                entry.file.new_path.clone()
            } else {
                entry.file.old_path.clone()
            };
            self.pending_scroll_to = Some(path);
        }
        cx.notify();
    }

    /// Replace the full set of file diffs (Staged -> Unstaged -> Untracked
    /// order). Entries may be unloaded placeholders — see `DiffFileEntry`.
    pub fn set_files(&mut self, files: Vec<DiffFileEntry>, cx: &mut Context<Self>) {
        self.files = files;
        self.rows_dirty = true;
        self.loading.clear();
        self.collapsed.clear();
        self.line_cache.clear();
        self.generation += 1;
        cx.notify();
    }

    /// Total added/removed across whatever's loaded so far — used for the
    /// workspace header summary, which updates incrementally as files load.
    pub fn total_change_counts(&self) -> (usize, usize) {
        self.files.iter().fold((0, 0), |(a, r), entry| {
            let (fa, fr) = entry.file.change_counts();
            (a + fa, r + fr)
        })
    }

    /// Kick off a `git_diff` fetch for `index` if it isn't already loaded or
    /// loading. No-op otherwise — safe to call repeatedly (e.g. every render).
    fn request_load(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(entry) = self.files.get(index) else {
            return;
        };
        if entry.loaded || self.loading.contains(&index) {
            return;
        }
        let path = if !entry.file.new_path.is_empty() {
            entry.file.new_path.clone()
        } else {
            entry.file.old_path.clone()
        };
        let staged = matches!(entry.section, GitFileSection::Staged);
        self.loading.insert(index);

        let handle = self.session_handle.clone();
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            let result = handle.git_diff(Some(&path), staged).await;
            let _ = this.update(cx, |this, cx| {
                this.loading.remove(&index);
                if this.generation != generation {
                    // `set_files` replaced the list while this was in flight —
                    // `index` no longer refers to this file.
                    return;
                }
                match result {
                    Ok(text) if text.len() <= MAX_DIFF_BYTES => {
                        let file = parse_unified_diff(&text)
                            .into_iter()
                            .find(|f| f.new_path == path || f.old_path == path);
                        if let (Some(file), Some(entry)) = (file, this.files.get_mut(index)) {
                            entry.file = file;
                            entry.loaded = true;
                            this.rows_dirty = true;
                            // Let `WorkspaceGitdiff` know it can recompute the
                            // header's running total now that this file loaded.
                            cx.emit(());
                        }
                    }
                    Ok(_) => {
                        tracing::warn!("[debug:diffload] diff too large for {path}, skipping");
                    }
                    Err(e) => {
                        tracing::warn!("git_diff RPC failed for {path}: {e}");
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Load the active (scrolled-to) file and a small window of its
    /// neighbors, so scrolling a little doesn't have to wait on an RPC.
    fn ensure_window_loaded(&mut self, cx: &mut Context<Self>) {
        let Some((active, _)) = self.sticky_header_geometry() else {
            return;
        };
        self.request_load_window(active, cx);
    }

    /// Request `center` and `PREFETCH_WINDOW` files on each side of it.
    fn request_load_window(&mut self, center: usize, cx: &mut Context<Self>) {
        let lo = center.saturating_sub(PREFETCH_WINDOW);
        let hi = (center + PREFETCH_WINDOW).min(self.files.len().saturating_sub(1));
        for index in lo..=hi {
            self.request_load(index, cx);
        }
    }

    /// Scroll so the given file's header row is at the top. If the view hasn't
    /// finished loading yet, the request is remembered and applied once ready.
    pub fn scroll_to(&mut self, path: &str, cx: &mut Context<Self>) {
        if self.rows_dirty {
            self.pending_scroll_to = Some(path.to_string());
            cx.notify();
            return;
        }
        self.scroll_to_now(path, cx);
    }

    fn scroll_to_now(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(index) = self
            .files
            .iter()
            .position(|entry| entry.file.new_path == path || entry.file.old_path == path)
        else {
            return;
        };
        let Some(&row) = self.file_row_start.get(index) else {
            return;
        };
        // `scroll_to_item` on the outer `UniformListScrollHandle` — NOT
        // `.0.borrow().base_handle.scroll_to_item(..)`, which is a different
        // (single-arg) method on the plain `div.rs` `ScrollHandle` that only
        // consults per-child bounds tracking `uniform_list` never populates,
        // making it a silent no-op here.
        self.scroll_handle.scroll_to_item(row, ScrollStrategy::Top);
        // Load the tapped file and its window immediately by known index —
        // right after a programmatic jump, the scroll offset hasn't visually
        // settled yet, so `ensure_window_loaded`'s offset-derived active file
        // (read on the next render) would still point at wherever we were
        // scrolled *before* this jump.
        self.request_load_window(index, cx);
        cx.notify();
    }

    /// Resolve a native selection (row range in UTF-16 units) against the
    /// rendered rows. Clamped to the first file's rows if the selection spans
    /// more than one file.
    pub fn line_range_for_selection(&self, range_utf16: Range<usize>) -> Option<EditorSelection> {
        resolve_selection(&self.files, &self.cached_rows, range_utf16)
    }

    fn sync_editor_theme(&mut self, editor_theme: &EditorTheme) {
        if self.editor_theme == *editor_theme {
            return;
        }
        self.editor_theme = editor_theme.clone();
        self.line_cache.clear();
        self.rows_dirty = true;
    }

    /// Flatten `files` into virtualized rows (spacers, file headers, diff
    /// lines), reusing `line_cache` for any file whose highlighted lines are
    /// already built. Cheap to call on every `rows_dirty` (e.g. a collapse
    /// toggle) as long as no file's content or the theme actually changed.
    fn rebuild_row_cache(&mut self) {
        let mut rows: Vec<CachedRow> = Vec::new();
        let mut file_row_start = Vec::with_capacity(self.files.len());
        let mut max_line_chars = 0usize;

        for (file_index, entry) in self.files.iter().enumerate() {
            let status = file_status(&entry.file);
            if file_index > 0 {
                rows.push(CachedRow {
                    file_index,
                    kind: RowKind::Spacer,
                    highlights: Vec::new(),
                    content: String::new(),
                    status,
                    section: entry.section,
                });
            }

            let (added, removed) = entry.file.change_counts();
            file_row_start.push(rows.len());
            rows.push(CachedRow {
                file_index,
                kind: RowKind::FileHeader,
                highlights: Vec::new(),
                content: String::new(),
                status,
                section: entry.section,
            });
            rows.push(CachedRow {
                file_index,
                kind: RowKind::FileHeaderTail { added, removed },
                highlights: Vec::new(),
                content: entry.file.display_path(),
                status,
                section: entry.section,
            });

            if !entry.loaded {
                rows.push(CachedRow {
                    file_index,
                    kind: RowKind::LoadingPlaceholder,
                    highlights: Vec::new(),
                    content: String::new(),
                    status,
                    section: entry.section,
                });
                continue;
            }

            if self.collapsed.contains(&file_index) {
                continue;
            }

            let editor_theme = &self.editor_theme;
            let cached = self
                .line_cache
                .entry(file_index)
                .or_insert_with(|| Rc::new(build_file_lines(file_index, entry, editor_theme)));
            max_line_chars = max_line_chars.max(cached.1);
            rows.extend(cached.0.iter().cloned());
        }

        self.max_line_chars = max_line_chars;
        self.file_row_start = file_row_start;
        self.cached_rows = Rc::new(rows);
        self.rows_dirty = false;
    }

    /// Which file's header should be pinned at the top right now, and how far
    /// to shift it (0 = fully stuck, negative = being pushed off by the next
    /// file's real header approaching from below) — classic sticky-section-
    /// header behavior. Row positions are uniform (`LINE_HEIGHT` each), so
    /// this is plain arithmetic against the current scroll offset rather than
    /// needing per-row measurement.
    fn sticky_header_geometry(&self) -> Option<(usize, f32)> {
        if self.files.is_empty() || self.file_row_start.is_empty() {
            return None;
        }
        let offset_y: f32 = self.scroll_handle.0.borrow().base_handle.offset().y.into();
        let viewport_top = (-offset_y).max(0.0);

        let mut active = 0usize;
        for (i, &row) in self.file_row_start.iter().enumerate() {
            let top = row as f32 * LINE_HEIGHT;
            if top <= viewport_top {
                active = i;
            } else {
                break;
            }
        }

        let push = self
            .file_row_start
            .get(active + 1)
            .map(|&next_row| {
                let next_top = next_row as f32 * LINE_HEIGHT;
                let distance = next_top - viewport_top;
                if distance < HEADER_BLOCK_HEIGHT {
                    -(HEADER_BLOCK_HEIGHT - distance)
                } else {
                    0.0
                }
            })
            .unwrap_or(0.0);

        Some((active, push))
    }

    fn render_sticky_header(&self, weak: WeakEntity<Self>, cx: &App) -> Option<AnyElement> {
        let (active, push) = self.sticky_header_geometry()?;
        let entry = self.files.get(active)?;
        // Tail row already carries the counts computed once in `build_rows` —
        // reuse it instead of re-scanning every hunk/line on every render.
        let tail_row = self.cached_rows.get(self.file_row_start[active] + 1)?;
        let RowKind::FileHeaderTail { added, removed } = &tail_row.kind else {
            return None;
        };
        let (added, removed) = (*added, *removed);
        let accent = status_accent(file_status(&entry.file), entry.section, cx);
        let collapsed = self.collapsed.contains(&active);
        let chevron = render_collapse_chevron(active, collapsed, weak, cx);
        Some(
            div()
                .absolute()
                .top(px(push))
                .left(px(0.0))
                .w_full()
                .h(px(HEADER_BLOCK_HEIGHT))
                .child(render_file_header_chrome(
                    entry.file.display_path(),
                    added,
                    removed,
                    accent,
                    chevron,
                    cx,
                ))
                .into_any_element(),
        )
    }
}

/// Build one file's syntax-highlighted `Line` rows (the expensive part of row
/// flattening — parses and highlights every line). Cached by
/// `CombinedDiffView::rebuild_row_cache` per file index so collapsing a file,
/// or any other `rows_dirty` trigger, doesn't redo this for unaffected files.
fn build_file_lines(
    file_index: usize,
    entry: &DiffFileEntry,
    editor_theme: &EditorTheme,
) -> (Vec<CachedRow>, usize) {
    let mut rows = Vec::new();
    let mut max_line_chars = 0usize;
    let status = file_status(&entry.file);

    let file_path = if entry.file.new_path.is_empty() {
        &entry.file.old_path
    } else {
        &entry.file.new_path
    };
    let mut highlighter = Highlighter::from_filename(file_path);

    for hunk in &entry.file.hunks {
        for line in &hunk.lines {
            let (highlights, char_len) =
                if line.kind != DiffLineKind::Header && !line.content.is_empty() {
                    let content_len = line.content.len();
                    highlighter.parse_fresh(&line.content);
                    let raw = highlighter.highlights(&line.content, 0..content_len);
                    let mut result: Vec<(Range<usize>, HighlightStyle)> = Vec::new();
                    for (span_range, capture_name) in &raw {
                        if let Some(style) = editor_theme.syntax.get(capture_name) {
                            let start = span_range.start.min(content_len);
                            let end = span_range.end.min(content_len);
                            if start < end {
                                result.push((start..end, style));
                            }
                        }
                    }
                    (
                        super::merge_highlights(result),
                        line.content.chars().count(),
                    )
                } else {
                    (Vec::new(), line.content.chars().count())
                };
            max_line_chars = max_line_chars.max(char_len);

            rows.push(CachedRow {
                file_index,
                kind: RowKind::Line(line.clone()),
                highlights,
                content: line.content.clone(),
                status,
                section: entry.section,
            });
        }
    }

    (rows, max_line_chars)
}

/// Resolve a row-range selection (in UTF-16 offsets over the rendered row
/// text) against `cached_rows`, clamping to the first file's rows if the
/// selection spans more than one file.
fn resolve_selection(
    files: &[DiffFileEntry],
    cached_rows: &[CachedRow],
    range_utf16: Range<usize>,
) -> Option<EditorSelection> {
    let contents: Vec<&str> = cached_rows.iter().map(|r| r.content.as_str()).collect();
    let (start_row, end_row) = line_range_for_selection_lines(&contents, range_utf16)?;
    let start_idx = (start_row as usize).saturating_sub(1);
    let end_idx = (end_row as usize)
        .saturating_sub(1)
        .min(cached_rows.len().saturating_sub(1));
    if start_idx >= cached_rows.len() {
        return None;
    }

    let file_index = cached_rows[start_idx].file_index;
    let end_idx = cached_rows
        .get(end_idx)
        .filter(|row| row.file_index == file_index)
        .map(|_| end_idx)
        .unwrap_or_else(|| {
            // Selection spilled into another file — clamp to the last row
            // belonging to `file_index`.
            cached_rows
                .iter()
                .rposition(|row| row.file_index == file_index)
                .unwrap_or(start_idx)
        });

    let mut start_line: Option<u32> = None;
    let mut end_line: Option<u32> = None;
    let mut text_lines: Vec<&str> = Vec::new();
    for row in &cached_rows[start_idx..=end_idx] {
        let RowKind::Line(line) = &row.kind else {
            continue;
        };
        let line_num = line.new_line_num.or(line.old_line_num);
        let Some(line_num) = line_num else { continue };
        if start_line.is_none() {
            start_line = Some(line_num as u32);
        }
        end_line = Some(line_num as u32);
        text_lines.push(&line.content);
    }

    let (start_line, end_line) = (start_line?, end_line?);
    let entry = &files[file_index];
    let path = if entry.file.new_path.is_empty() {
        entry.file.old_path.clone()
    } else {
        entry.file.new_path.clone()
    };

    Some(EditorSelection {
        path,
        start: start_line,
        end: end_line,
        text: text_lines.join("\n"),
    })
}

fn render_spacer_row(i: usize, separator: &'static str, text_style: &TextStyle) -> AnyElement {
    // Empty but still selectable/ordered, so `resolve_selection`'s row<->text
    // offset accounting stays aligned with what's actually painted.
    let blank = StyledText::new(" ")
        .with_default_highlights(text_style, Vec::new())
        .selectable()
        .selection_order(i as u64)
        .selection_separator_after(separator);
    div()
        .w_full()
        .h(px(LINE_HEIGHT))
        .opacity(0.0)
        .child(blank)
        .into_any_element()
}

fn render_loading_placeholder_row(
    i: usize,
    separator: &'static str,
    text_style: &TextStyle,
    cx: &App,
) -> AnyElement {
    let mut small_style = text_style.clone();
    small_style.font_size = px(GUTTER_FONT_SIZE).into();
    small_style.color = rgb(theme::text_muted(cx)).into();
    let label = StyledText::new("Loading diff…")
        .with_default_highlights(&small_style, Vec::new())
        .selectable()
        .selection_order(i as u64)
        .selection_separator_after(separator);
    div()
        .w_full()
        .h(px(LINE_HEIGHT))
        .flex()
        .items_center()
        .justify_center()
        .opacity(0.6)
        .child(label)
        .into_any_element()
}

/// Shared visual chrome for a file header block: a full-height section-color
/// stripe flush against the left edge, then the path label and +/- counts.
/// Used both by the real (selectable) list row and the non-selectable sticky
/// overlay — `label` differs between the two (`StyledText` vs plain string).
fn render_file_header_chrome(
    label: impl IntoElement,
    added: usize,
    removed: usize,
    accent: Rgba,
    trailing: AnyElement,
    cx: &App,
) -> AnyElement {
    div()
        .size_full()
        .flex()
        .flex_row()
        .bg(rgb(theme::bg_card(cx)))
        .child(div().flex_shrink_0().w(px(3.0)).h_full().bg(accent))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_2()
                .pl_2()
                .pr_2()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(accent)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(FONT_SIZE))
                        .child(label),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_shrink_0()
                        .items_center()
                        .gap_2()
                        .child(render_header_counts(added, removed, cx))
                        .child(trailing),
                ),
        )
        .into_any_element()
}

/// Chevron button that collapses/expands `file_index`'s diff body. Down =
/// expanded (matches the sidebar's section-toggle convention), right =
/// collapsed.
fn render_collapse_chevron(
    file_index: usize,
    collapsed: bool,
    weak: WeakEntity<CombinedDiffView>,
    cx: &App,
) -> AnyElement {
    div()
        .id(("diff-collapse-toggle", file_index))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .w(px(30.0))
        .h(px(26.0))
        .cursor_pointer()
        // Near-zero top slop: this sits right below the workspace header's
        // own buttons, whose hit-test slop reaches a few px past their
        // visual bounds — a generous top slop here would extend back up
        // into that zone and race them for the same taps.
        .hit_slop_edges(Edges {
            top: px(2.0),
            bottom: px(10.0),
            left: px(10.0),
            right: px(10.0),
        })
        .on_press(move |_event, _window, cx| {
            let _ = weak.update(cx, |this, cx| this.toggle_collapsed(file_index, cx));
        })
        .child(
            svg()
                .path(if collapsed {
                    "icons/chevron-right.svg"
                } else {
                    "icons/chevron-down.svg"
                })
                .size(px(18.0))
                .text_color(rgb(theme::text_muted(cx))),
        )
        .into_any_element()
}

fn render_header_counts(added: usize, removed: usize, cx: &App) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .flex_shrink_0()
        .items_center()
        .gap_2()
        .text_size(px(GUTTER_FONT_SIZE))
        .when(added > 0, |el| {
            el.child(
                div()
                    .text_color(rgb(theme::git_added(cx)))
                    .child(format!("+{added}")),
            )
        })
        .when(removed > 0, |el| {
            el.child(
                div()
                    .text_color(rgb(theme::git_removed(cx)))
                    .child(format!("-{removed}")),
            )
        })
        .into_any_element()
}

/// Top half of the (visually doubled-height) file header block — blank; the
/// visible label lives in `render_file_header_tail_row`, which overlaps into
/// this row to center across both (see that function).
fn render_file_header_row(
    i: usize,
    separator: &'static str,
    text_style: &TextStyle,
    cx: &App,
) -> AnyElement {
    let blank = StyledText::new(" ")
        .with_default_highlights(text_style, Vec::new())
        .selectable()
        .selection_order(i as u64)
        .selection_separator_after(separator);
    div()
        .w_full()
        .h(px(LINE_HEIGHT))
        .bg(rgb(theme::bg_card(cx)))
        .child(blank)
        .into_any_element()
}

/// Bottom half of the doubled-height file header block. Holds the real label
/// content in an absolutely-positioned overlay shifted up by one row height
/// and sized to two rows, so it paints across both halves (uniform_list
/// paints items in index order with only a viewport-level clip, so this
/// row's overlay — painted after `render_file_header_row`'s blank row above
/// it — correctly shows on top) and centers vertically in the combined span.
#[allow(clippy::too_many_arguments)]
fn render_file_header_tail_row(
    row: &CachedRow,
    added: usize,
    removed: usize,
    i: usize,
    separator: &'static str,
    text_style: &TextStyle,
    collapsed: bool,
    weak: WeakEntity<CombinedDiffView>,
    cx: &App,
) -> AnyElement {
    let accent = status_accent(row.status, row.section, cx);
    // `StyledText` bakes its `TextStyle`'s color into the paint run, so the
    // chrome div's `.text_color(accent)` (which only affects inherited-color
    // children) has no effect here — the color must be set on the style itself.
    let mut label_style = text_style.clone();
    label_style.color = accent.into();
    let label = StyledText::new(row.content.clone())
        .with_default_highlights(&label_style, Vec::new())
        .selectable()
        .selection_order(i as u64)
        .selection_separator_after(separator);
    let chevron = render_collapse_chevron(row.file_index, collapsed, weak, cx);
    div()
        .relative()
        .w_full()
        .h(px(LINE_HEIGHT))
        .child(
            div()
                .absolute()
                .top(px(-LINE_HEIGHT))
                .left(px(0.0))
                .w_full()
                .h(px(HEADER_BLOCK_HEIGHT))
                .child(render_file_header_chrome(
                    label, added, removed, accent, chevron, cx,
                )),
        )
        .into_any_element()
}

fn diff_line_gutter(
    line: &DiffLine,
    diff: &theme::DiffTheme,
    editor_theme: &EditorTheme,
) -> (Rgba, String) {
    match line.kind {
        DiffLineKind::Header => (rgb(diff.header_bg), String::new()),
        DiffLineKind::Added => {
            let num = line
                .new_line_num
                .map(|n| format!("{:>3}", n))
                .unwrap_or_default();
            (rgb(diff.added_bg), num)
        }
        DiffLineKind::Removed => {
            let num = line
                .old_line_num
                .map(|n| format!("{:>3}", n))
                .unwrap_or_default();
            (rgb(diff.removed_bg), num)
        }
        DiffLineKind::Unchanged => {
            let num = line
                .new_line_num
                .map(|n| format!("{:>3}", n))
                .unwrap_or_default();
            (rgb(editor_theme.background), num)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_line_row(
    row: &CachedRow,
    line: &DiffLine,
    i: usize,
    separator: &'static str,
    text_style: &TextStyle,
    diff: &theme::DiffTheme,
    editor_theme: &EditorTheme,
    h_scroll_offset: f32,
) -> AnyElement {
    let (bg_color, gutter_text) = diff_line_gutter(line, diff, editor_theme);

    let content = &line.content;
    let styled_text = if content.is_empty() {
        StyledText::new(" ").with_default_highlights(text_style, Vec::new())
    } else {
        StyledText::new(content.clone()).with_default_highlights(text_style, row.highlights.clone())
    }
    .selectable()
    .selection_order(i as u64)
    .selection_separator_after(separator);

    let gutter = div()
        .w(px(GUTTER_WIDTH))
        .h(px(LINE_HEIGHT))
        .flex()
        .items_center()
        .justify_end()
        .pr_2()
        .text_color(rgb(diff.gutter_text))
        .text_size(px(GUTTER_FONT_SIZE))
        .child(gutter_text);

    let text_slot = div()
        .flex_1()
        .h(px(LINE_HEIGHT))
        .overflow_hidden()
        .relative()
        .child(render_line_text_slot(styled_text, h_scroll_offset));

    div()
        .w_full()
        .flex()
        .flex_row()
        .h(px(LINE_HEIGHT))
        .bg(bg_color)
        .child(gutter)
        .child(text_slot)
        .into_any_element()
}

fn render_line_text_slot(styled_text: StyledText, h_scroll_offset: f32) -> impl IntoElement {
    div()
        .absolute()
        .top(px(0.0))
        .left(px(-h_scroll_offset))
        .h(px(LINE_HEIGHT))
        .flex()
        .items_center()
        .text_size(px(FONT_SIZE))
        .relative()
        .child(styled_text)
}

impl EventEmitter<()> for CombinedDiffView {}

impl Focusable for CombinedDiffView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for CombinedDiffView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_editor_theme(&theme::bundle(cx).editor);

        if self.rows_dirty {
            info!("rebuilding combined diff row cache");
            self.rebuild_row_cache();
            if let Some(path) = self.pending_scroll_to.take() {
                self.scroll_to_now(&path, cx);
            }
        }
        // Cheap no-op once the active file's window is already loaded/loading;
        // only actually spawns fetches for files newly scrolled into range.
        self.ensure_window_loaded(cx);

        let row_count = self.cached_rows.len();
        let cached_rows = self.cached_rows.clone();
        let collapsed = self.collapsed.clone();
        let weak = cx.weak_entity();
        let bottom_inset = f32::max(platform_bridge::home_indicator_inset(), BOTTOM_INSET_MIN);
        let extra_items = (bottom_inset / LINE_HEIGHT).ceil() as usize;
        let h_scroll_offset = self.h_scroll.offset;
        let scroll_y_lock = self.scroll_handle.0.borrow().base_handle.offset().y;

        let editor_theme = self.editor_theme.clone();
        let diff = editor_theme.diff.clone();
        let text_style = {
            let mut style = window.text_style();
            style.color = rgb(diff.body_text).into();
            style.font_size = px(FONT_SIZE).into();
            style
        };
        let sticky_header = self.render_sticky_header(weak.clone(), cx);

        div()
            .relative()
            .overflow_hidden()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(editor_theme.background))
            .track_focus(&self.focus_handle)
            .on_scroll_wheel(
                cx.listener(move |this, event: &ScrollWheelEvent, _window, cx| {
                    if this.h_scroll.handle_wheel(
                        event,
                        this.max_line_chars,
                        FONT_SIZE,
                        &this.scroll_handle,
                        scroll_y_lock,
                    ) {
                        cx.notify();
                    }
                }),
            )
            .child(
                selection_area(
                    uniform_list("combined-diff-view-rows", row_count + extra_items, {
                        let text_style = text_style.clone();
                        let diff = diff.clone();
                        let editor_theme = editor_theme.clone();
                        let collapsed = collapsed.clone();
                        let weak = weak.clone();
                        move |range: Range<usize>, _window: &mut Window, cx: &mut App| {
                            range
                                .map(|i| {
                                    if i >= row_count {
                                        return div().h(px(LINE_HEIGHT)).into_any_element();
                                    }

                                    let row = &cached_rows[i];
                                    let separator = if i + 1 < row_count { "\n" } else { "" };

                                    match &row.kind {
                                        RowKind::Spacer => {
                                            render_spacer_row(i, separator, &text_style)
                                        }
                                        RowKind::FileHeader => {
                                            render_file_header_row(i, separator, &text_style, cx)
                                        }
                                        RowKind::FileHeaderTail { added, removed } => {
                                            render_file_header_tail_row(
                                                row,
                                                *added,
                                                *removed,
                                                i,
                                                separator,
                                                &text_style,
                                                collapsed.contains(&row.file_index),
                                                weak.clone(),
                                                cx,
                                            )
                                        }
                                        RowKind::LoadingPlaceholder => {
                                            render_loading_placeholder_row(
                                                i,
                                                separator,
                                                &text_style,
                                                cx,
                                            )
                                        }
                                        RowKind::Line(line) => render_line_row(
                                            row,
                                            line,
                                            i,
                                            separator,
                                            &text_style,
                                            &diff,
                                            &editor_theme,
                                            h_scroll_offset,
                                        ),
                                    }
                                })
                                .collect()
                        }
                    })
                    .track_scroll(&self.scroll_handle)
                    .w_full()
                    .flex_1(),
                )
                .id(DIFF_SELECTION_AREA_ID)
                .action_with_image("Mention", "zedra", AddSelectionMention)
                .action_with_image("Comment", "zedra", AddSelectionComment),
            )
            .when_some(sticky_header, |el, overlay| el.child(overlay))
    }
}
