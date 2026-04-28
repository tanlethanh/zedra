use gpui::*;
use zedra_session::SessionHandle;

use crate::editor::code_editor::{CODE_EDITOR_SELECTION_AREA_ID, EditorView, ParsedEditorSyntax};
use crate::editor::markdown::{
    MARKDOWN_SELECTION_AREA_ID, MarkdownView, is_markdown_path, parse_markdown_source,
};
use crate::placeholder::render_placeholder;

#[derive(Clone, Debug)]
enum FileState {
    Loading,
    Loaded,
    TooLarge,
    Error { error: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EditorContent {
    Code,
    Markdown,
}

pub struct WorkspaceEditor {
    path: String,
    filename: String,
    state: FileState,
    content: EditorContent,
    editor_view: Entity<EditorView>,
    markdown_view: Entity<MarkdownView>,
    session_handle: SessionHandle,
    read_task: Option<Task<()>>,
    open_epoch: u64,
}

impl WorkspaceEditor {
    pub fn new(session_handle: SessionHandle, cx: &mut App) -> Self {
        Self {
            path: String::new(),
            filename: String::new(),
            state: FileState::Loading,
            content: EditorContent::Code,
            editor_view: cx.new(|cx| EditorView::new(cx)),
            markdown_view: cx.new(|_cx| MarkdownView::new(SharedString::default())),
            session_handle,
            read_task: None,
            open_epoch: 0,
        }
    }

    /// Request loading a file from the remote host.
    /// The file will be loaded asynchronously; when ready, a `FileReady` event is emitted.
    pub fn open_file(&mut self, path: String, cx: &mut Context<Self>) {
        let filename = path.rsplit('/').next().unwrap_or(&path).to_string();
        self.path = path.clone();
        self.filename = filename;
        self.open_epoch = self.open_epoch.wrapping_add(1);
        let epoch = self.open_epoch;
        self.content = if is_markdown_path(&path) {
            EditorContent::Markdown
        } else {
            EditorContent::Code
        };
        self.state = FileState::Loading;
        cx.notify();

        // Drop any previous task before starting a new one.
        let prev_task = self.read_task.take();
        drop(prev_task);

        let handle = self.session_handle.clone();
        let filename = self.filename.clone();
        let content_kind = self.content;
        let read_task = cx.spawn(async move |this, cx| {
            let read_result = handle.fs_read(&path).await;
            match read_result {
                Ok(result) if result.too_large => {
                    if let Err(e) = this.update(cx, |this, cx| {
                        if this.open_epoch != epoch {
                            return;
                        }
                        this.state = FileState::TooLarge;
                        cx.notify();
                    }) {
                        tracing::error!("update failed for {}: {}", path, e);
                    }
                }
                Ok(result) if result.error.is_some() => {
                    let error = result.error.unwrap_or("unknown error".to_string());
                    if let Err(e) = this.update(cx, |this, cx| {
                        if this.open_epoch != epoch {
                            return;
                        }
                        this.state = FileState::Error { error };
                        cx.notify();
                    }) {
                        tracing::error!("update failed for {}: {}", path, e);
                    }
                }
                Ok(result) => match content_kind {
                    EditorContent::Code => {
                        let content = result.content;
                        let content_for_syntax = content.clone();
                        let syntax_filename = filename.clone();
                        if let Err(e) = this.update(cx, |this, cx| {
                            if this.open_epoch != epoch {
                                return;
                            }
                            this.state = FileState::Loaded;
                            this.editor_view.update(cx, |editor_view, _cx| {
                                editor_view.set_content(&filename, content);
                            });
                            cx.notify();
                        }) {
                            tracing::error!("update failed for {}: {}", path, e);
                            return;
                        }

                        let parsed_syntax = cx
                            .background_spawn(async move {
                                ParsedEditorSyntax::build(&syntax_filename, content_for_syntax)
                            })
                            .await;

                        if let Err(e) = this.update(cx, |this, cx| {
                            if this.open_epoch != epoch || this.path != path {
                                return;
                            }
                            this.editor_view.update(cx, |editor_view, _cx| {
                                editor_view.apply_parsed_syntax(parsed_syntax);
                            });
                            cx.notify();
                        }) {
                            tracing::error!("syntax apply failed for {}: {}", path, e);
                        }
                    }
                    EditorContent::Markdown => {
                        let parsed = cx
                            .background_spawn(async move { parse_markdown_source(result.content) })
                            .await;
                        if let Err(e) = this.update(cx, |this, cx| {
                            if this.open_epoch != epoch {
                                return;
                            }
                            this.state = FileState::Loaded;
                            this.markdown_view.update(cx, |markdown_view, _cx| {
                                markdown_view.set_parsed_source(parsed);
                            });
                            cx.notify();
                        }) {
                            tracing::error!("update failed for {}: {}", path, e);
                        }
                    }
                },
                Err(e) => {
                    tracing::error!("fs/read failed for {}: {}", path, e);
                    let error = e.to_string();
                    if let Err(e) = this.update(cx, |this, cx| {
                        if this.open_epoch != epoch {
                            return;
                        }
                        this.state = FileState::Error { error };
                        cx.notify();
                    }) {
                        tracing::error!("update failed for {}: {}", path, e);
                    }
                }
            }
        });

        self.read_task = Some(read_task);
    }

    pub fn selected_agent_context_range(
        &self,
        window: &Window,
        cx: &App,
    ) -> Option<EditorSelection> {
        if !matches!(&self.state, FileState::Loaded) {
            return None;
        }

        let selection = window.latest_read_only_selection()?;
        let area_id = selection.area_id.to_string();
        let (start, end) = match self.content {
            EditorContent::Code if area_id == CODE_EDITOR_SELECTION_AREA_ID => self
                .editor_view
                .read(cx)
                .line_range_for_selection(selection.range_utf16)?,
            EditorContent::Markdown if area_id == MARKDOWN_SELECTION_AREA_ID => self
                .markdown_view
                .read(cx)
                .line_range_for_selection(selection.range_utf16)?,
            _ => return None,
        };

        Some(EditorSelection {
            path: self.path.clone(),
            start,
            end,
            text: selection.text,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EditorSelection {
    pub path: String,
    pub start: u32,
    pub end: u32,
    pub text: String,
}

impl Render for WorkspaceEditor {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        match self.state.clone() {
            FileState::Loading => render_placeholder("Loading ..."),
            FileState::TooLarge => render_placeholder("File too large (>500 KB)"),
            FileState::Error { error } => render_placeholder(format!("Error: {}", error)),
            FileState::Loaded => match self.content {
                EditorContent::Code => div().size_full().child(self.editor_view.clone()),
                EditorContent::Markdown => div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .min_h_0()
                    .child(self.markdown_view.clone()),
            },
        }
    }
}
