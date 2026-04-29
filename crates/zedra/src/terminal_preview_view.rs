use gpui::*;
use tracing::error;
use zedra_session::SessionHandle;
use zedra_terminal::terminal::{TerminalHyperlink, TerminalHyperlinkTarget};

use crate::editor::code_editor::{EditorView, ParsedEditorSyntax};
use crate::editor::markdown::{MarkdownView, is_markdown_path, parse_markdown_source};
use crate::fonts;
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

pub struct TerminalPreviewView {
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

impl TerminalPreviewView {
    pub fn new(
        session_handle: SessionHandle,
        workspace_state: Entity<WorkspaceState>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            session_handle,
            workspace_state,
            editor_view: cx.new(|cx| EditorView::new(cx)),
            markdown_view: cx.new(|_cx| MarkdownView::new_for_sheet(SharedString::default())),
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
                                    editor_view.set_content(&filename, content);
                                });
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
                                this.markdown_view.update(cx, |markdown_view, _cx| {
                                    markdown_view.set_parsed_source(parsed);
                                });
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
                            cx.notify();
                        });
                    }
                });
                self.read_task = Some(read_task);
            }
        }
    }

    fn should_ignore_load_result(&self, epoch: u64, path: &str, content: PreviewContent) -> bool {
        self.open_epoch != epoch
            || self.active_path.as_deref() != Some(path)
            || self.content != content
    }
}

fn preview_content_for_path(path: &str) -> PreviewContent {
    if is_markdown_path(path) {
        PreviewContent::Markdown
    } else {
        PreviewContent::Editor
    }
}

impl Render for TerminalPreviewView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let body: AnyElement = match &self.state {
            PreviewState::Idle => {
                render_placeholder("Tap a file path in the terminal").into_any_element()
            }
            PreviewState::Loading => render_placeholder("Loading ...").into_any_element(),
            PreviewState::TooLarge => {
                render_placeholder("File too large (>500 KB)").into_any_element()
            }
            PreviewState::Error(error) => {
                render_placeholder(format!("Error: {error}")).into_any_element()
            }
            PreviewState::Loaded => match self.content {
                PreviewContent::Editor => self.editor_view.clone().into_any_element(),
                PreviewContent::Markdown => div()
                    .id("terminal-preview-markdown-viewport")
                    .size_full()
                    .flex()
                    .flex_col()
                    .min_h_0()
                    .child(self.markdown_view.clone())
                    .into_any_element(),
            },
        };

        div()
            .id("terminal-preview-sheet")
            .size_full()
            .bg(rgb(theme::BG_PRIMARY))
            .flex()
            .flex_col()
            .child(
                div()
                    .w_full()
                    .px(px(theme::SPACING_LG))
                    .pt(px(18.0))
                    .pb(px(8.0))
                    .border_b_1()
                    .border_color(rgb(theme::BORDER_SUBTLE))
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        div()
                            .text_color(rgb(theme::TEXT_PRIMARY))
                            .text_size(px(theme::FONT_HEADING))
                            .font_family(fonts::HEADING_FONT_FAMILY)
                            .font_weight(FontWeight::MEDIUM)
                            .child(self.title.clone()),
                    )
                    .child(
                        div()
                            .text_color(rgb(theme::TEXT_MUTED))
                            .text_size(px(theme::FONT_DETAIL))
                            .font_family(fonts::MONO_FONT_FAMILY)
                            .child(self.subtitle.clone()),
                    ),
            )
            .child(
                div()
                    .id("terminal-preview-body")
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(body),
            )
    }
}
