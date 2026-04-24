use gpui::*;
use zedra_session::SessionHandle;

use crate::editor::code_editor::EditorView;
use crate::placeholder::render_placeholder;

#[derive(Clone, Debug)]
enum FileState {
    Loading,
    Loaded,
    TooLarge,
    Error { error: String },
}

pub struct WorkspaceEditor {
    filename: String,
    state: FileState,
    editor_view: Entity<EditorView>,
    session_handle: SessionHandle,
    read_task: Option<Task<()>>,
}

impl WorkspaceEditor {
    pub fn new(session_handle: SessionHandle, cx: &mut App) -> Self {
        Self {
            filename: String::new(),
            state: FileState::Loading,
            editor_view: cx.new(|cx| EditorView::new(cx)),
            session_handle,
            read_task: None,
        }
    }

    /// Request loading a file from the remote host.
    /// The file will be loaded asynchronously; when ready, a `FileReady` event is emitted.
    pub fn open_file(&mut self, path: String, cx: &mut Context<Self>) {
        let filename = path.rsplit('/').next().unwrap_or(&path).to_string();
        self.filename = filename;
        self.state = FileState::Loading;
        cx.notify();

        // Drop any previous task before starting a new one.
        let prev_task = self.read_task.take();
        drop(prev_task);

        let handle = self.session_handle.clone();
        let filename = self.filename.clone();
        let read_task = cx.spawn(async move |this, cx| {
            let (state, content) = match handle.fs_read(&path).await {
                Ok(result) if result.too_large => (FileState::TooLarge, None),
                Ok(result) if result.error.is_some() => (
                    FileState::Error {
                        error: result.error.unwrap_or("unknown error".to_string()),
                    },
                    None,
                ),
                Ok(result) => (FileState::Loaded, Some(result.content)),
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
                    this.editor_view.update(cx, |editor_view, _cx| {
                        editor_view.set_content(&filename, content);
                    });
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
            FileState::Loaded => div().size_full().child(self.editor_view.clone()),
        }
    }
}
