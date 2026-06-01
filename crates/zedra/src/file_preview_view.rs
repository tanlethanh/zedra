use gpui::*;
use tracing::error;
use zedra_session::SessionHandle;
use zedra_terminal::terminal::{TerminalHyperlink, TerminalHyperlinkTarget};

use crate::editor::code_editor::{EditorView, ParsedEditorSyntax};
use crate::editor::markdown::{MarkdownView, is_markdown_path, parse_markdown_source};
use crate::fonts;
use crate::native_presentation;
use crate::placeholder::render_placeholder;
use crate::theme;
use crate::workspace_state::WorkspaceState;

#[derive(Clone, Debug)]
enum PreviewState {
    Idle,
    Loading,
    Loaded,
    TooLarge,
    Error(String),
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
}

impl FilePreviewView {
    pub fn new(
        session_handle: SessionHandle,
        workspace_state: Entity<WorkspaceState>,
        cx: &mut Context<Self>,
    ) -> Self {
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
        }
    }

    pub fn open_hyperlink(&mut self, hyperlink: TerminalHyperlink, cx: &mut Context<Self>) {
        self.open_epoch = self.open_epoch.wrapping_add(1);
        let epoch = self.open_epoch;
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
        native_presentation::set_sheet_content_at_top(true);

        let prev_task = self.read_task.take();
        drop(prev_task);

        self.active_path = Some(path.to_string());
        self.title = title.into();
        self.subtitle = subtitle.into();
        self.content = preview_content_for_path(path);
        self.state = PreviewState::Loading;
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

impl Render for FilePreviewView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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

        div()
            .id("file-preview-sheet")
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
