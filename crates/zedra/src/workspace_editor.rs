use gpui::*;
use zedra_session::SessionHandle;

use crate::editor::code_editor::EditorView;
use crate::placeholder::render_placeholder;

#[derive(Clone, Debug)]
enum FileState {
    Loading,
    Loaded { content: String },
    TooLarge,
    Error { error: String },
}

pub struct WorkspaceEditor {
    filename: String,
    state: FileState,
    session_handle: SessionHandle,
    read_task: Option<Task<()>>,
}

impl WorkspaceEditor {
    pub fn new(session_handle: SessionHandle) -> Self {
        Self {
            filename: String::new(),
            state: FileState::Loading,
            session_handle,
            read_task: None,
        }
    }

    /// Request loading a file from the remote host.
    /// The file will be loaded asynchronously; when ready, a `FileReady` event is emitted.
    pub fn open_file(&mut self, path: String, cx: &mut Context<Self>) {
        let filename = path.rsplit('/').next().unwrap_or(&path).to_string();
        self.filename = filename;
        cx.notify();

        // Drop any previous task before starting a new one.
        let prev_task = self.read_task.take();
        drop(prev_task);

        let handle = self.session_handle.clone();
        let read_task = cx.spawn(async move |this, cx| {
            let state = match handle.fs_read(&path).await {
                Ok(result) => {
                    if result.too_large {
                        FileState::TooLarge
                    } else {
                        FileState::Loaded {
                            content: result.content,
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("fs/read failed for {}: {}", path, e);
                    FileState::Error {
                        error: e.to_string(),
                    }
                }
            };

            if let Err(e) = this.update(cx, |this, cx| {
                this.state = state;
                cx.notify();
            }) {
                tracing::error!("update failed for {}: {}", path, e);
            }
        });

        self.read_task = Some(read_task);
    }
}

impl Render for WorkspaceEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match self.state.clone() {
            FileState::Loading => render_placeholder("Loading ..."),
            FileState::TooLarge => render_placeholder("File too large (>500 KB)"),
            FileState::Error { error } => render_placeholder(format!("Error: {}", error)),
            FileState::Loaded { content } => {
                let editor_view = cx.new(|cx| EditorView::new(content, &self.filename, cx));
                div().child(editor_view)
            }
        }
    }
}
