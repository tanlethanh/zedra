use std::time::Duration;

use gpui::prelude::FluentBuilder;
use gpui::*;
use tracing::{error, info, warn};
use zedra_session::SessionHandle;
use zedra_terminal::terminal::{TerminalHyperlink, TerminalHyperlinkTarget};

use crate::editor::code_editor::{EditorView, ParsedEditorSyntax};
use crate::editor::markdown::{MarkdownView, is_markdown_path, parse_markdown_source};
use crate::fonts;
use crate::native_presentation;
use crate::placeholder::render_placeholder;
use crate::theme;
use crate::ui::input::{Input, InputChanged};
use crate::workspace::ActiveWorkspace;
use crate::workspace_action::AddSelectionToChat;
use crate::workspace_editor::{EditorSelection, resolve_read_only_selection};
use crate::workspace_state::WorkspaceState;

/// Debounce before a live edit is flushed to the host. Coalesces bursts of
/// keystrokes into a single `fs/write` while keeping saves feeling immediate.
const MARKDOWN_AUTOSAVE_DEBOUNCE: Duration = Duration::from_millis(500);

#[derive(Clone, Debug)]
enum PreviewState {
    Idle,
    Loading,
    Loaded,
    TooLarge,
    Error(String),
}

/// Live-save status for the markdown editor, surfaced in the sheet header.
#[derive(Clone, Debug, PartialEq, Eq)]
enum SaveState {
    Idle,
    Saving,
    Saved,
    /// The file changed on the host since we last read it; autosave is paused
    /// until the user reloads or overwrites.
    Conflict,
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PreviewContent {
    Editor,
    Markdown,
}

/// File preview rendered in a native sheet. Shared by the terminal (file links,
/// loaded lazily via [`open_hyperlink`]) and the agent detail view (read-only
/// config/memory files whose contents arrive with the listing, shown via
/// [`open_content`]). Both render through the same editor/markdown views and
/// native scroll-boundary handoff.
///
/// [`open_hyperlink`]: FilePreviewView::open_hyperlink
/// [`open_content`]: FilePreviewView::open_content
pub struct FilePreviewView {
    session_handle: SessionHandle,
    workspace_state: Entity<WorkspaceState>,
    editor_view: Entity<EditorView>,
    markdown_view: Entity<MarkdownView>,
    state: PreviewState,
    content: PreviewContent,
    title: SharedString,
    subtitle: SharedString,
    active_path: Option<String>,
    read_task: Option<Task<()>>,
    open_epoch: u64,
    /// Multiline source editor used in markdown edit mode.
    editor_input: Entity<Input>,
    /// True while the markdown file is being edited (rendered preview replaced
    /// by `editor_input`).
    editing: bool,
    /// Whether the active markdown file may be edited. Only workspace files
    /// loaded via [`open_hyperlink`] are writable; read-only agent content from
    /// [`open_content`] is not.
    ///
    /// [`open_hyperlink`]: FilePreviewView::open_hyperlink
    /// [`open_content`]: FilePreviewView::open_content
    allow_edit: bool,
    /// Current raw markdown source, kept in sync with `editor_input` while
    /// editing so the preview can re-parse it on exit.
    markdown_source: String,
    /// Optimistic-concurrency token `(mtime, size)` for the active file, from
    /// the last read or successful write. Drives conflict detection on save.
    version: Option<(Option<u64>, u64)>,
    save_state: SaveState,
    /// True while the save loop is running; keeps writes single-flighted so the
    /// concurrency token can never drift from overlapping writes.
    saving: bool,
    save_task: Option<Task<()>>,
    /// Current host content captured on a conflict, offered for reload.
    conflict_content: Option<String>,
    _input_subscription: Subscription,
}

impl FilePreviewView {
    pub fn new(
        session_handle: SessionHandle,
        workspace_state: Entity<WorkspaceState>,
        cx: &mut Context<Self>,
    ) -> Self {
        let editor_input = cx.new(|cx| {
            Input::new(cx)
                .multiline(true)
                .native_suggestions(true)
                .placeholder("Markdown source")
        });
        let input_subscription = cx.subscribe(
            &editor_input,
            |this: &mut Self, _input, event: &InputChanged, cx| {
                this.on_source_input(event.value.clone(), cx);
            },
        );
        Self {
            session_handle,
            workspace_state,
            editor_view: cx.new(|cx| EditorView::new(cx)),
            markdown_view: cx.new(|cx| MarkdownView::new(SharedString::default(), cx)),
            state: PreviewState::Idle,
            content: PreviewContent::Editor,
            title: "Terminal Link".into(),
            subtitle: "Tap a file link in the terminal to preview it here.".into(),
            active_path: None,
            read_task: None,
            open_epoch: 0,
            editor_input,
            editing: false,
            allow_edit: false,
            markdown_source: String::new(),
            version: None,
            save_state: SaveState::Idle,
            saving: false,
            save_task: None,
            conflict_content: None,
            _input_subscription: input_subscription,
        }
    }

    /// Resolve this sheet window's active read-only selection into an
    /// [`EditorSelection`] against the preview's own editor/markdown views.
    fn selected_agent_context(&self, window: &Window, cx: &App) -> Option<EditorSelection> {
        if !matches!(self.state, PreviewState::Loaded) {
            return None;
        }
        let path = self.active_path.clone()?;
        resolve_read_only_selection(&self.editor_view, &self.markdown_view, path, window, cx)
    }

    fn handle_add_selection_to_chat(
        &mut self,
        _action: &AddSelectionToChat,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(selection) = self.selected_agent_context(window, cx) else {
            warn!("agent: add selection to chat (preview) missing selection");
            return;
        };
        // The selection lives in this sheet window; clear it here, then route to
        // the foreground workspace's agent-target picker via ambient context.
        window.clear_read_only_selection_cache();
        let Some(workspace) = ActiveWorkspace::get(cx) else {
            return;
        };
        workspace.update(cx, |workspace, cx| {
            workspace.present_add_to_chat(selection, cx);
        });
    }

    pub fn open_hyperlink(&mut self, hyperlink: TerminalHyperlink, cx: &mut Context<Self>) {
        self.open_epoch = self.open_epoch.wrapping_add(1);
        let epoch = self.open_epoch;
        self.reset_edit_state();
        native_presentation::set_sheet_content_at_top(true);
        match hyperlink.target {
            TerminalHyperlinkTarget::Url { url } => {
                let prev_task = self.read_task.take();
                drop(prev_task);

                self.active_path = None;
                self.title = "External Link".into();
                self.subtitle = url.into();
                self.state = PreviewState::Error(
                    "Only file hyperlinks are supported in the preview sheet.".into(),
                );
                self.update_sheet_scroll_boundary(cx);
                cx.notify();
            }
            TerminalHyperlinkTarget::File {
                path,
                relative_path,
                line,
                column,
            } => {
                self.active_path = Some(path.clone());
                self.title = relative_path
                    .rsplit('/')
                    .next()
                    .unwrap_or(&relative_path)
                    .to_string()
                    .into();
                let homedir = self.workspace_state.read(cx).homedir.clone();
                let stripped_path = if !homedir.is_empty() && path.starts_with(&homedir) {
                    format!("~{}", &path[homedir.len()..])
                } else {
                    path.clone()
                };
                self.subtitle = match (line, column) {
                    (Some(line), Some(column)) => format!("{stripped_path}:{line}:{column}").into(),
                    (Some(line), None) => format!("{stripped_path}:{line}").into(),
                    _ => stripped_path.into(),
                };
                self.content = preview_content_for_path(&path);
                self.state = PreviewState::Loading;
                self.update_sheet_scroll_boundary(cx);
                cx.notify();

                let prev_task = self.read_task.take();
                drop(prev_task);

                let handle = self.session_handle.clone();
                let filename = self.title.to_string();
                let content_kind = self.content;
                let read_task = cx.spawn(async move |this, cx| match handle.fs_read(&path).await {
                    Ok(result) if result.too_large => {
                        let _ = this.update(cx, |this, cx| {
                            if this.should_ignore_load_result(epoch, &path, content_kind) {
                                return;
                            }
                            this.state = PreviewState::TooLarge;
                            cx.notify();
                        });
                    }
                    Ok(result) if result.error.is_some() => {
                        let msg = result.error.unwrap_or("unknown error".to_string());
                        let _ = this.update(cx, |this, cx| {
                            if this.should_ignore_load_result(epoch, &path, content_kind) {
                                return;
                            }
                            error!("terminal link preview fs/read error for {}: {}", path, msg);
                            this.state = PreviewState::Error(msg);
                            cx.notify();
                        });
                    }
                    Ok(result) => match content_kind {
                        PreviewContent::Editor => {
                            let content = result.content;
                            let content_for_syntax = content.clone();
                            let syntax_filename = filename.clone();
                            if let Err(e) = this.update(cx, |this, cx| {
                                if this.should_ignore_load_result(
                                    epoch,
                                    &path,
                                    PreviewContent::Editor,
                                ) {
                                    return;
                                }
                                this.state = PreviewState::Loaded;
                                this.editor_view.update(cx, |editor_view, _cx| {
                                    editor_view
                                        .set_content_with_initial_line(&filename, content, line);
                                    native_presentation::set_sheet_content_at_top(
                                        editor_view.is_scrolled_to_file_top(),
                                    );
                                });
                                this.update_sheet_scroll_boundary(cx);
                                cx.notify();
                            }) {
                                error!("terminal link preview update failed for {}: {}", path, e);
                                return;
                            }

                            let parsed_syntax = cx
                                .background_spawn(async move {
                                    ParsedEditorSyntax::build(&syntax_filename, content_for_syntax)
                                })
                                .await;

                            let _ = this.update(cx, |this, cx| {
                                if this.should_ignore_load_result(
                                    epoch,
                                    &path,
                                    PreviewContent::Editor,
                                ) {
                                    return;
                                }
                                this.editor_view.update(cx, |editor_view, _cx| {
                                    editor_view.apply_parsed_syntax(parsed_syntax);
                                });
                                cx.notify();
                            });
                        }
                        PreviewContent::Markdown => {
                            let source = result.content.clone();
                            let version = (result.mtime, result.size);
                            let parsed = cx
                                .background_spawn(
                                    async move { parse_markdown_source(result.content) },
                                )
                                .await;
                            let _ = this.update(cx, |this, cx| {
                                if this.should_ignore_load_result(
                                    epoch,
                                    &path,
                                    PreviewContent::Markdown,
                                ) {
                                    return;
                                }
                                this.state = PreviewState::Loaded;
                                // Workspace markdown loaded over fs/read is editable;
                                // seed the live-save source and concurrency token.
                                this.allow_edit = true;
                                this.markdown_source = source;
                                this.version = Some(version);
                                this.markdown_view.update(cx, |markdown_view, cx| {
                                    markdown_view.set_parsed_source(parsed, cx);
                                });
                                this.update_sheet_scroll_boundary(cx);
                                cx.notify();
                            });
                        }
                    },
                    Err(err) => {
                        let msg = err.to_string();
                        let _ = this.update(cx, |this, cx| {
                            if this.should_ignore_load_result(epoch, &path, content_kind) {
                                return;
                            }
                            error!("terminal link preview fs/read error for {}: {}", path, msg);
                            this.state = PreviewState::Error(msg);
                            this.update_sheet_scroll_boundary(cx);
                            cx.notify();
                        });
                    }
                });
                self.read_task = Some(read_task);
            }
        }
    }

    /// Preview a file whose contents are already in hand, with no `fs/read`.
    /// Used for files sourced outside the workspace — e.g. an agent's read-only
    /// config/memory files delivered alongside its listing. `path` drives only
    /// the editor-vs-markdown choice and syntax detection.
    pub fn open_content(
        &mut self,
        title: impl Into<SharedString>,
        subtitle: impl Into<SharedString>,
        path: &str,
        content: String,
        cx: &mut Context<Self>,
    ) {
        self.open_epoch = self.open_epoch.wrapping_add(1);
        let epoch = self.open_epoch;
        self.reset_edit_state();
        native_presentation::set_sheet_content_at_top(true);

        let prev_task = self.read_task.take();
        drop(prev_task);

        self.active_path = Some(path.to_string());
        self.title = title.into();
        self.subtitle = subtitle.into();
        self.content = preview_content_for_path(path);
        self.state = PreviewState::Loading;
        self.update_sheet_scroll_boundary(cx);
        cx.notify();

        self.apply_loaded_content(epoch, path.to_string(), content, cx);
    }

    /// Render already-loaded `content` into the active view and parse syntax /
    /// markdown in the background. The caller must have set `active_path`,
    /// `content`, and bumped `open_epoch` first. Installs a fresh `read_task`,
    /// so call it from a main-thread context — never from inside the current
    /// `read_task`, which the assignment would cancel.
    fn apply_loaded_content(
        &mut self,
        epoch: u64,
        path: String,
        content: String,
        cx: &mut Context<Self>,
    ) {
        match self.content {
            PreviewContent::Editor => {
                self.state = PreviewState::Loaded;
                let filename = self.title.to_string();
                let content_for_syntax = content.clone();
                self.editor_view.update(cx, |editor_view, _cx| {
                    editor_view.set_content_with_initial_line(&filename, content, None);
                    native_presentation::set_sheet_content_at_top(
                        editor_view.is_scrolled_to_file_top(),
                    );
                });
                self.update_sheet_scroll_boundary(cx);
                cx.notify();
                self.read_task = Some(cx.spawn(async move |this, cx| {
                    let parsed = cx
                        .background_spawn(async move {
                            ParsedEditorSyntax::build(&filename, content_for_syntax)
                        })
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        if this.should_ignore_load_result(epoch, &path, PreviewContent::Editor) {
                            return;
                        }
                        this.editor_view.update(cx, |editor_view, _cx| {
                            editor_view.apply_parsed_syntax(parsed)
                        });
                        cx.notify();
                    });
                }));
            }
            PreviewContent::Markdown => {
                self.read_task = Some(cx.spawn(async move |this, cx| {
                    let parsed = cx
                        .background_spawn(async move { parse_markdown_source(content) })
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        if this.should_ignore_load_result(epoch, &path, PreviewContent::Markdown) {
                            return;
                        }
                        this.state = PreviewState::Loaded;
                        this.markdown_view.update(cx, |markdown_view, cx| {
                            markdown_view.set_parsed_source(parsed, cx);
                        });
                        this.update_sheet_scroll_boundary(cx);
                        cx.notify();
                    });
                }));
            }
        }
    }

    /// Clear all per-file edit/save state. Called when a new file is opened so
    /// edit mode, the live-save loop, and the concurrency token never leak from
    /// the previously previewed file.
    fn reset_edit_state(&mut self) {
        self.editing = false;
        self.allow_edit = false;
        self.markdown_source.clear();
        self.version = None;
        self.save_state = SaveState::Idle;
        self.saving = false;
        self.save_task = None;
        self.conflict_content = None;
    }

    /// True when the active file is editable markdown currently loaded.
    fn can_edit(&self) -> bool {
        self.allow_edit
            && self.content == PreviewContent::Markdown
            && matches!(self.state, PreviewState::Loaded)
    }

    /// Toggle between rendered preview and the source editor.
    fn toggle_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing {
            self.exit_edit(cx);
        } else {
            if !self.can_edit() {
                return;
            }
            self.editing = true;
            let source = self.markdown_source.clone();
            self.editor_input
                .update(cx, |input, _cx| input.set_value(source));
            self.editor_input
                .read(cx)
                .focus_handle(cx)
                .focus(window, cx);
            cx.notify();
        }
    }

    /// Leave edit mode: re-render the (possibly edited) source as preview.
    fn exit_edit(&mut self, cx: &mut Context<Self>) {
        self.editing = false;
        let source = self.markdown_source.clone();
        let parsed = parse_markdown_source(source);
        self.markdown_view.update(cx, |markdown_view, cx| {
            markdown_view.set_parsed_source(parsed, cx)
        });
        cx.notify();
    }

    /// Handle a live edit from the source editor: track the new source and
    /// schedule an autosave.
    fn on_source_input(&mut self, value: String, cx: &mut Context<Self>) {
        if !self.editing {
            return;
        }
        self.markdown_source = value;
        // A fresh edit clears a prior terminal save status but not an unresolved
        // conflict — that must be reloaded or overwritten explicitly.
        if matches!(self.save_state, SaveState::Saved | SaveState::Error) {
            self.save_state = SaveState::Idle;
        }
        self.schedule_save(cx);
    }

    /// Start (or let the running loop pick up) a debounced live save.
    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        if !self.allow_edit || self.active_path.is_none() || self.save_state == SaveState::Conflict
        {
            return;
        }
        self.save_state = SaveState::Saving;
        cx.notify();
        if self.saving {
            // The running loop re-reads `markdown_source` after its debounce.
            return;
        }
        self.saving = true;
        let handle = self.session_handle.clone();
        self.save_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(MARKDOWN_AUTOSAVE_DEBOUNCE)
                    .await;
                let Ok(Some((path, source, expected))) = this.update(cx, |this, _cx| {
                    if !this.allow_edit || this.save_state == SaveState::Conflict {
                        return None;
                    }
                    let path = this.active_path.clone()?;
                    Some((path, this.markdown_source.clone(), this.version))
                }) else {
                    break;
                };

                let result = handle.fs_write(&path, &source, expected, false).await;

                let keep_going = this
                    .update(cx, |this, cx| this.apply_write_result(result, &source, cx))
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
            let _ = this.update(cx, |this, _cx| this.saving = false);
        }));
    }

    /// Apply a write result. Returns `true` if the source changed again during
    /// the write and the loop should write once more.
    fn apply_write_result(
        &mut self,
        result: anyhow::Result<zedra_rpc::proto::FsWriteResult>,
        written: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        match result {
            Ok(res) if res.conflict => {
                info!(
                    "[debug:fs-conflict] markdown autosave paused for {:?}",
                    self.active_path
                );
                self.version = Some((res.mtime, res.size));
                self.conflict_content = res.current_content;
                self.save_state = SaveState::Conflict;
                cx.notify();
                false
            }
            Ok(res) if res.ok => {
                self.version = Some((res.mtime, res.size));
                if self.markdown_source != written {
                    // More edits arrived during the write; write again.
                    true
                } else {
                    self.save_state = SaveState::Saved;
                    cx.notify();
                    false
                }
            }
            other => {
                if let Err(e) = other {
                    error!("markdown autosave failed for {:?}: {}", self.active_path, e);
                }
                self.save_state = SaveState::Error;
                cx.notify();
                false
            }
        }
    }

    /// Reload the host's current content, discarding local edits, to resolve a
    /// conflict.
    fn reload_conflict(&mut self, cx: &mut Context<Self>) {
        let Some(content) = self.conflict_content.take() else {
            return;
        };
        self.markdown_source = content.clone();
        self.save_state = SaveState::Idle;
        if self.editing {
            self.editor_input
                .update(cx, |input, _cx| input.set_value(content.clone()));
        }
        let parsed = parse_markdown_source(content);
        self.markdown_view.update(cx, |markdown_view, cx| {
            markdown_view.set_parsed_source(parsed, cx)
        });
        cx.notify();
    }

    /// Force-overwrite the host with local edits, resolving a conflict in favor
    /// of this editor.
    fn overwrite_conflict(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.active_path.clone() else {
            return;
        };
        self.conflict_content = None;
        self.save_state = SaveState::Saving;
        cx.notify();
        let handle = self.session_handle.clone();
        let source = self.markdown_source.clone();
        self.save_task = Some(cx.spawn(async move |this, cx| {
            let result = handle.fs_write(&path, &source, None, true).await;
            let _ = this.update(cx, |this, cx| {
                this.apply_write_result(result, &source, cx);
            });
        }));
    }

    fn should_ignore_load_result(&self, epoch: u64, path: &str, content: PreviewContent) -> bool {
        self.open_epoch != epoch
            || self.active_path.as_deref() != Some(path)
            || self.content != content
    }

    fn update_sheet_scroll_boundary(&self, cx: &mut Context<Self>) {
        let is_at_top = match self.state {
            PreviewState::Loaded => match self.content {
                PreviewContent::Editor => self.editor_view.read(cx).is_scrolled_to_top(),
                PreviewContent::Markdown => self.markdown_view.read(cx).is_scrolled_to_top(),
            },
            PreviewState::Idle
            | PreviewState::Loading
            | PreviewState::TooLarge
            | PreviewState::Error(_) => true,
        };
        native_presentation::set_sheet_content_at_top(is_at_top);
    }
}

fn preview_content_for_path(path: &str) -> PreviewContent {
    if is_markdown_path(path) {
        PreviewContent::Markdown
    } else {
        PreviewContent::Editor
    }
}

impl FilePreviewView {
    /// "Edit" / "Done" pill shown for editable markdown.
    fn render_edit_action(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let label = if self.editing { "Done" } else { "Edit" };
        div()
            .id("file-preview-edit-toggle")
            .flex_none()
            .px(px(12.0))
            .py(px(6.0))
            .rounded(px(6.0))
            .bg(rgb(theme::bg_surface(cx)))
            .border_1()
            .border_color(rgb(theme::border_subtle(cx)))
            .text_size(px(theme::FONT_DETAIL))
            .text_color(rgb(theme::text_primary(cx)))
            .child(label)
            .on_press(cx.listener(|this, event: &PressEvent, window, cx| {
                if event.completed() {
                    this.toggle_edit(window, cx);
                }
            }))
    }

    fn save_status_label(&self) -> Option<&'static str> {
        match self.save_state {
            SaveState::Saving => Some("Saving…"),
            SaveState::Saved => Some("Saved"),
            SaveState::Error => Some("Save failed"),
            SaveState::Idle | SaveState::Conflict => None,
        }
    }

    /// Banner shown while autosave is paused on a conflict, offering reload or
    /// overwrite.
    fn render_conflict_banner(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let button = |id: &'static str, label: &'static str, cx: &mut Context<Self>| {
            div()
                .id(id)
                .px(px(12.0))
                .py(px(6.0))
                .rounded(px(6.0))
                .bg(rgb(theme::bg_surface(cx)))
                .border_1()
                .border_color(rgb(theme::border_subtle(cx)))
                .text_size(px(theme::FONT_DETAIL))
                .text_color(rgb(theme::text_primary(cx)))
                .child(label)
        };
        div()
            .w_full()
            .px(px(theme::SPACING_LG))
            .py(px(8.0))
            .border_b_1()
            .border_color(rgb(theme::border_subtle(cx)))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .child(
                div()
                    .flex_1()
                    .text_size(px(theme::FONT_DETAIL))
                    .text_color(rgb(theme::text_muted(cx)))
                    .child("File changed on the host. Reload or overwrite?"),
            )
            .child(
                button("file-preview-conflict-reload", "Reload", cx).on_press(cx.listener(
                    |this, event: &PressEvent, _window, cx| {
                        if event.completed() {
                            this.reload_conflict(cx);
                        }
                    },
                )),
            )
            .child(
                button("file-preview-conflict-overwrite", "Overwrite", cx).on_press(cx.listener(
                    |this, event: &PressEvent, _window, cx| {
                        if event.completed() {
                            this.overwrite_conflict(cx);
                        }
                    },
                )),
            )
    }
}

impl Render for FilePreviewView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let editing_markdown =
            self.editing && self.content == PreviewContent::Markdown && self.can_edit();
        let body: AnyElement = match &self.state {
            PreviewState::Idle => {
                render_placeholder(cx, "Tap a file path in the terminal").into_any_element()
            }
            PreviewState::Loading => render_placeholder(cx, "Loading ...").into_any_element(),
            PreviewState::TooLarge => {
                render_placeholder(cx, "File too large (>500 KB)").into_any_element()
            }
            PreviewState::Error(error) => {
                render_placeholder(cx, format!("Error: {error}")).into_any_element()
            }
            PreviewState::Loaded if editing_markdown => div()
                .id("file-preview-markdown-editor")
                .size_full()
                .min_h_0()
                .overflow_y_scroll()
                .p(px(theme::SPACING_LG))
                .child(self.editor_input.clone())
                .into_any_element(),
            PreviewState::Loaded => match self.content {
                PreviewContent::Editor => self.editor_view.clone().into_any_element(),
                PreviewContent::Markdown => div()
                    .id("file-preview-markdown-viewport")
                    .size_full()
                    .flex()
                    .flex_col()
                    .min_h_0()
                    .child(self.markdown_view.clone())
                    .into_any_element(),
            },
        };

        let show_edit_action = self.can_edit() || self.editing;
        let save_status = self.save_status_label();

        div()
            .id("file-preview-sheet")
            .on_action(cx.listener(Self::handle_add_selection_to_chat))
            .size_full()
            .bg(rgb(theme::bg_primary(cx)))
            .flex()
            .flex_col()
            .child(
                div()
                    .w_full()
                    .px(px(theme::SPACING_LG))
                    .pt(px(if cfg!(target_os = "ios") { 18.0 } else { 8.0 }))
                    .pb(px(8.0))
                    .border_b_1()
                    .border_color(rgb(theme::border_subtle(cx)))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .child(
                                div()
                                    .text_color(rgb(theme::text_primary(cx)))
                                    .text_size(px(theme::FONT_HEADING))
                                    .font_family(fonts::HEADING_FONT_FAMILY)
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(self.title.clone()),
                            )
                            .child(
                                div()
                                    .text_color(rgb(theme::text_muted(cx)))
                                    .text_size(px(theme::FONT_DETAIL))
                                    .font_family(fonts::MONO_FONT_FAMILY)
                                    .child(self.subtitle.clone()),
                            ),
                    )
                    .when_some(save_status, |this, label| {
                        this.child(
                            div()
                                .flex_none()
                                .text_size(px(theme::FONT_DETAIL))
                                .text_color(rgb(theme::text_muted(cx)))
                                .child(label),
                        )
                    })
                    .when(show_edit_action, |this| {
                        this.child(self.render_edit_action(cx))
                    }),
            )
            .when(self.save_state == SaveState::Conflict, |this| {
                this.child(self.render_conflict_banner(cx))
            })
            .child(
                div()
                    .id("file-preview-body")
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    // The custom sheet owns native handoff state; the active
                    // content view only reports whether its scroll is at top.
                    .on_scroll_wheel(cx.listener(|this, _event, _window, cx| {
                        this.update_sheet_scroll_boundary(cx);
                    }))
                    .child(body),
            )
    }
}
