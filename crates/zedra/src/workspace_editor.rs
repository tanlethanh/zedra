use gpui::*;
use zedra_session::SessionHandle;

use crate::editor::code_editor::EditorView;
use crate::editor::markdown::{
    MarkdownView, ParsedMarkdownSource, is_markdown_path, parse_markdown_source,
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

enum LoadedContent {
    Code(String),
    Markdown(ParsedMarkdownSource),
}

pub struct WorkspaceEditor {
    filename: String,
    state: FileState,
    content: EditorContent,
    editor_view: Entity<EditorView>,
    markdown_view: Entity<MarkdownView>,
    session_handle: SessionHandle,
    read_task: Option<Task<()>>,
}

impl WorkspaceEditor {
    pub fn new(session_handle: SessionHandle, cx: &mut App) -> Self {
        Self {
            filename: String::new(),
            state: FileState::Loading,
            content: EditorContent::Code,
            editor_view: cx.new(|cx| EditorView::new(cx)),
            markdown_view: cx.new(|_cx| MarkdownView::new(SharedString::default())),
            session_handle,
            read_task: None,
        }
    }

    /// Request loading a file from the remote host.
    /// The file will be loaded asynchronously; when ready, a `FileReady` event is emitted.
    pub fn open_file(&mut self, path: String, cx: &mut Context<Self>) {
        let filename = path.rsplit('/').next().unwrap_or(&path).to_string();
        self.filename = filename;
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
            let (state, content) = match handle.fs_read(&path).await {
                Ok(result) if result.too_large => (FileState::TooLarge, None),
                Ok(result) if result.error.is_some() => (
                    FileState::Error {
                        error: result.error.unwrap_or("unknown error".to_string()),
                    },
                    None,
                ),
                Ok(result) => {
                    let content = match content_kind {
                        EditorContent::Code => LoadedContent::Code(result.content),
                        EditorContent::Markdown => {
                            let parsed = cx
                                .background_spawn(
                                    async move { parse_markdown_source(result.content) },
                                )
                                .await;
                            LoadedContent::Markdown(parsed)
                        }
                    };
                    (FileState::Loaded, Some(content))
                }
                Err(e) => {
                    tracing::error!("fs/read failed for {}: {}", path, e);
                    (
                        FileState::Error {
                            error: e.to_string(),
                        },
                        None,
                    )
                }
            };

            if let Err(e) = this.update(cx, |this, cx| {
                this.state = state;
                if let Some(content) = content {
                    match content {
                        LoadedContent::Code(content) => {
                            this.editor_view.update(cx, |editor_view, _cx| {
                                editor_view.set_content(&filename, content);
                            });
                        }
                        LoadedContent::Markdown(parsed) => {
                            this.markdown_view.update(cx, |markdown_view, _cx| {
                                markdown_view.set_parsed_source(parsed);
                            });
                        }
                    }
                }
                cx.notify();
            }) {
                tracing::error!("update failed for {}: {}", path, e);
            }
        });

        self.read_task = Some(read_task);
    }
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
