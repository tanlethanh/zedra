// FileSearchPanel — floating global file search (cmd+P style).
//
// Mounted as a full-screen overlay by `Workspace`. Streams `fs_search`
// results from the host and dispatches `OpenFile` when a row is tapped.

use std::time::Duration;

use gpui::{prelude::FluentBuilder as _, *};
use tracing::*;

use zedra_rpc::proto::{FS_SEARCH_DEFAULT_LIMIT, FsSearchEntry};
use zedra_session::SessionHandle;

use crate::theme;
use crate::ui::InputChanged;
use crate::ui::input::Input;
use crate::workspace_action;

/// Debounce before issuing a remote search after the query changes.
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(180);
/// Height of a single result row (two lines: name + path).
const RESULT_ROW_HEIGHT: f32 = 52.0;
/// Maximum number of result rows shown before the list scrolls.
const MAX_VISIBLE_ROWS: usize = 8;

#[derive(Clone, Debug)]
pub enum FileSearchEvent {
    /// The panel requested to be dismissed.
    Close,
}

impl EventEmitter<FileSearchEvent> for FileSearchPanel {}

pub struct FileSearchPanel {
    session_handle: SessionHandle,
    focus_handle: FocusHandle,
    scroll_handle: UniformListScrollHandle,
    search_input: Entity<Input>,
    query: String,
    results: Vec<FsSearchEntry>,
    loading: bool,
    error: Option<String>,
    truncated: bool,
    /// Monotonic token; stale async responses are ignored.
    epoch: u64,
    _subscription: Subscription,
}

impl FileSearchPanel {
    pub fn new(session_handle: SessionHandle, cx: &mut Context<Self>) -> Self {
        let search_input = cx.new(|cx| {
            Input::new(cx)
                .placeholder("Search files")
                .trailing_gutter(40.0)
                .toggle_keyboard_on_press(true)
                .hide_keyboard_on_submit(true)
        });
        let subscription = cx.subscribe(
            &search_input,
            |this: &mut Self, _input, event: &InputChanged, cx| {
                this.query = event.value.clone();
                this.request_search(cx);
                cx.notify();
            },
        );

        Self {
            session_handle,
            focus_handle: cx.focus_handle(),
            scroll_handle: UniformListScrollHandle::new(),
            search_input,
            query: String::new(),
            results: Vec::new(),
            loading: false,
            error: None,
            truncated: false,
            epoch: 0,
            _subscription: subscription,
        }
    }

    /// Clear all state for a fresh open and focus the input.
    pub fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        debug!(
            "[DEBUG-ANDROID-IME] file_search.open query={:?} results={} keyboard_visible={}",
            self.query,
            self.results.len(),
            window.is_soft_keyboard_visible()
        );
        self.query.clear();
        self.results.clear();
        self.loading = false;
        self.error = None;
        self.truncated = false;
        self.epoch = self.epoch.wrapping_add(1);
        self.scroll_handle.scroll_to_item(0, ScrollStrategy::Top);
        self.search_input
            .update(cx, |input, _cx| input.set_value(""));
        let input_focus = self.search_input.read(cx).focus_handle(cx);
        input_focus.focus(window, cx);
        // Android registers the platform input handler during the next render
        // of this overlay. Requesting IME before that can leave the keyboard
        // attached to no active handler while the caret is already focused.
        #[cfg(not(target_os = "android"))]
        window.show_soft_keyboard();
        debug!(
            "[DEBUG-ANDROID-IME] file_search.open focused input; android_deferred_keyboard={} keyboard_visible={}",
            cfg!(target_os = "android"),
            window.is_soft_keyboard_visible()
        );
        cx.notify();
    }

    fn request_search(&mut self, cx: &mut Context<Self>) {
        let query = self.query.trim().to_string();
        self.epoch = self.epoch.wrapping_add(1);
        let epoch = self.epoch;

        self.results.clear();
        self.error = None;
        self.truncated = false;
        self.scroll_handle.scroll_to_item(0, ScrollStrategy::Top);
        if query.is_empty() {
            self.loading = false;
            return;
        }
        self.loading = true;

        let handle = self.session_handle.clone();
        cx.spawn(async move |this, cx| {
            // Debounce: drop the request if the query changed meanwhile.
            cx.background_executor().timer(SEARCH_DEBOUNCE).await;
            // A matching epoch already guarantees the query is unchanged (and
            // thus still non-empty, since empty queries never reach here).
            let still_current = this
                .update(cx, |this, _cx| this.epoch == epoch)
                .unwrap_or(false);
            if !still_current {
                return;
            }

            let result = handle.fs_search(".", &query, FS_SEARCH_DEFAULT_LIMIT).await;
            let _ = this.update(cx, |this, cx| {
                if this.epoch != epoch {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(result) => {
                        this.results = result.entries;
                        this.truncated = result.truncated;
                        this.error = None;
                    }
                    Err(error) => {
                        error!("file search failed: {error}");
                        this.results.clear();
                        this.truncated = false;
                        this.error = Some(error.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn render_result_row(
        &self,
        index: usize,
        entry: &FsSearchEntry,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let icon = if entry.is_dir {
            "icons/folder.svg"
        } else {
            "icons/file.svg"
        };
        // The filename is the last component of the relative path.
        let name = entry
            .rel_path
            .rsplit('/')
            .next()
            .unwrap_or(&entry.rel_path)
            .to_string();
        let path = entry.path.clone();
        let is_dir = entry.is_dir;

        div()
            .id(("file-search-row", index))
            .w_full()
            .h(px(RESULT_ROW_HEIGHT))
            .px(px(theme::SPACING_MD))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .cursor_pointer()
            .on_press(cx.listener(move |_this, _event, window, cx| {
                window.hide_soft_keyboard();
                window.dispatch_action(
                    workspace_action::RevealInFileExplorer { path: path.clone() }.boxed_clone(),
                    cx,
                );
                if is_dir {
                    window.dispatch_action(workspace_action::OpenDrawer.boxed_clone(), cx);
                } else {
                    window.dispatch_action(
                        workspace_action::OpenFile { path: path.clone() }.boxed_clone(),
                        cx,
                    );
                }
                cx.emit(FileSearchEvent::Close);
            }))
            .child(
                div().flex_shrink_0().child(
                    svg()
                        .path(icon)
                        .size(px(theme::ICON_FILE))
                        .text_color(rgb(theme::text_muted(cx))),
                ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .truncate()
                            .text_size(px(theme::FONT_BODY))
                            .text_color(rgb(theme::text_primary(cx)))
                            .child(name),
                    )
                    // Highlight exactly the characters the host matched.
                    .child(build_highlighted_path(
                        &entry.rel_path,
                        &entry.match_indices,
                        cx,
                    )),
            )
            .into_any_element()
    }

    fn render_message(&self, message: impl Into<SharedString>, cx: &App) -> AnyElement {
        div()
            .w_full()
            .h(px(RESULT_ROW_HEIGHT * 2.0))
            .flex()
            .items_center()
            .justify_center()
            .px(px(theme::SPACING_MD))
            .text_size(px(theme::FONT_BODY))
            .text_color(rgb(theme::text_muted(cx)))
            .child(message.into())
            .into_any_element()
    }

    fn clear_query(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        debug!(
            "[DEBUG-ANDROID-IME] file_search.clear_query keyboard_visible={}",
            window.is_soft_keyboard_visible()
        );
        self.query.clear();
        self.results.clear();
        self.loading = false;
        self.error = None;
        self.truncated = false;
        self.epoch = self.epoch.wrapping_add(1);
        self.scroll_handle.scroll_to_item(0, ScrollStrategy::Top);
        self.search_input
            .update(cx, |input, _cx| input.set_value(""));
        let input_focus = self.search_input.read(cx).focus_handle(cx);
        input_focus.focus(window, cx);
        #[cfg(not(target_os = "android"))]
        window.show_soft_keyboard();
        debug!(
            "[DEBUG-ANDROID-IME] file_search.clear_query focused input; android_deferred_keyboard={} keyboard_visible={}",
            cfg!(target_os = "android"),
            window.is_soft_keyboard_visible()
        );
        cx.notify();
    }
}

impl Focusable for FileSearchPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for FileSearchPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let has_query = !self.query.trim().is_empty();

        let body: AnyElement = if !has_query {
            self.render_message("Type to search files", cx)
        } else if self.loading {
            self.render_message("Searching…", cx)
        } else if let Some(error) = self.error.clone() {
            self.render_message(format!("Search failed: {error}"), cx)
        } else if self.results.is_empty() {
            self.render_message("No matching files", cx)
        } else {
            let len = self.results.len();
            let list_height = (len.min(MAX_VISIBLE_ROWS) as f32) * RESULT_ROW_HEIGHT;
            let list = uniform_list(
                "file-search-results",
                len,
                cx.processor(|this, range: std::ops::Range<usize>, _window, cx| {
                    range
                        .filter_map(|idx| {
                            this.results
                                .get(idx)
                                .map(|entry| this.render_result_row(idx, entry, cx))
                        })
                        .collect::<Vec<_>>()
                }),
            )
            .track_scroll(&self.scroll_handle)
            .h(px(list_height));

            let footer = self.truncated.then(|| {
                div()
                    .w_full()
                    .px(px(theme::SPACING_MD))
                    .py(px(theme::SPACING_SM))
                    .border_t_1()
                    .border_color(rgb(theme::border_subtle(cx)))
                    .text_size(px(theme::FONT_DETAIL))
                    .text_color(rgb(theme::text_muted(cx)))
                    .child("Showing first matches")
            });

            div()
                .w_full()
                .flex()
                .flex_col()
                .child(list)
                .children(footer)
                .into_any_element()
        };

        div()
            .track_focus(&self.focus_handle)
            .w_full()
            .flex()
            .flex_col()
            .bg(rgb(theme::bg_card(cx)))
            .rounded(px(10.0))
            .border_1()
            .border_color(rgb(theme::border_subtle(cx)))
            .overflow_hidden()
            // Stop taps inside the panel from reaching the dismiss backdrop.
            .on_pointer_down(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .relative()
                    .w_full()
                    .child(
                        div()
                            .w_full()
                            .px(px(theme::SPACING_MD))
                            .py(px(theme::SPACING_SM))
                            .border_b_1()
                            .border_color(rgb(theme::border_subtle(cx)))
                            .child(self.search_input.clone()),
                    )
                    .when(!self.query.is_empty(), |container| {
                        container.child(
                            div()
                                .id("file-search-clear")
                                .absolute()
                                .right(px(theme::SPACING_LG))
                                .top_0()
                                .bottom_0()
                                .w(px(32.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .on_pointer_down(|_, _, cx| cx.stop_propagation())
                                .on_press(cx.listener(|this, _event, window, cx| {
                                    this.clear_query(window, cx);
                                }))
                                .child(
                                    div()
                                        .w(px(32.0))
                                        .h(px(32.0))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded(px(6.0))
                                        .child(
                                            svg()
                                                .path("icons/x.svg")
                                                .size(px(14.0))
                                                .text_color(rgb(theme::text_secondary(cx))),
                                        ),
                                ),
                        )
                    }),
            )
            .child(body)
    }
}

/// Split `text` into contiguous `(segment, is_match)` runs using the host's
/// matched character positions, so emphasis mirrors what the host actually
/// scored instead of a separate client-side matcher.
fn highlight_runs(text: &str, match_indices: &[u32]) -> Vec<(String, bool)> {
    let mut runs: Vec<(String, bool)> = Vec::new();
    let mut next = 0usize;
    for (char_pos, ch) in text.chars().enumerate() {
        let pos = char_pos as u32;
        // `match_indices` is sorted ascending; advance past anything behind us.
        while next < match_indices.len() && match_indices[next] < pos {
            next += 1;
        }
        let matched = next < match_indices.len() && match_indices[next] == pos;
        match runs.last_mut() {
            Some((segment, run_matched)) if *run_matched == matched => segment.push(ch),
            _ => runs.push((ch.to_string(), matched)),
        }
    }
    runs
}

/// Render a relative path with the host-matched characters emphasized.
fn build_highlighted_path(rel_path: &str, match_indices: &[u32], cx: &App) -> AnyElement {
    let mut row = div().w_full().min_w_0().flex().flex_row().overflow_hidden();
    for (segment, matched) in highlight_runs(rel_path, match_indices) {
        let seg = div()
            .flex_shrink_0()
            .text_size(px(theme::FONT_DETAIL))
            .child(segment);
        let seg = if matched {
            seg.text_color(rgb(theme::text_primary(cx)))
                .font_weight(FontWeight::BOLD)
        } else {
            seg.text_color(rgb(theme::text_muted(cx)))
        };
        row = row.child(seg);
    }
    row.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::highlight_runs;

    #[test]
    fn highlight_runs_marks_indexed_characters() {
        // Indices into "src/main.rs" selecting 'm','a','i','n'.
        let runs = highlight_runs("src/main.rs", &[4, 5, 6, 7]);
        let rebuilt: String = runs.iter().map(|(s, _)| s.as_str()).collect();
        assert_eq!(rebuilt, "src/main.rs");
        let matched: String = runs
            .iter()
            .filter(|(_, m)| *m)
            .map(|(s, _)| s.as_str())
            .collect();
        assert_eq!(matched, "main");
    }

    #[test]
    fn highlight_runs_without_indices_is_single_unmatched_run() {
        let runs = highlight_runs("main.rs", &[]);
        assert_eq!(runs, vec![("main.rs".to_string(), false)]);
    }
}
