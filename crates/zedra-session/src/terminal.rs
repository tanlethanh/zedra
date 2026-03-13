/// One remote PTY. Durable across reconnects; stored as `Arc<RemoteTerminal>`.
///
/// `output` is written by the TermAttach pump task and drained by `TerminalView` each frame.
/// `needs_render` is an `Arc<AtomicBool>` shared with `TerminalView`; the pump sets it
/// to `true` and calls `push_callback` so the frame loop forces a re-render. The view
/// clears it in `render()` before calling `process_output()`.
/// `input_tx` is replaced on each reconnect. `last_seq` persists so the server replays
/// only output the client hasn't yet received.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Thread-safe ring buffer for terminal output chunks.
pub type OutputBuffer = Arc<Mutex<VecDeque<Vec<u8>>>>;

pub struct RemoteTerminal {
    pub id: String,
    /// Raw PTY output chunks written by the pump, drained by `TerminalView::render()`.
    pub output: OutputBuffer,
    /// Set `true` by the pump when new output arrives; cleared by `TerminalView` in render.
    pub needs_render: Arc<AtomicBool>,
    /// Sender into the live TermAttach input stream; `None` when disconnected.
    input_tx: Mutex<Option<tokio::sync::mpsc::Sender<Vec<u8>>>>,
    last_seq: AtomicU64,
}

impl RemoteTerminal {
    pub(crate) fn new(id: String) -> Arc<Self> {
        Arc::new(Self {
            id,
            output: Arc::new(Mutex::new(VecDeque::new())),
            needs_render: Arc::new(AtomicBool::new(false)),
            input_tx: Mutex::new(None),
            last_seq: AtomicU64::new(0),
        })
    }

    pub fn last_seq(&self) -> u64 {
        self.last_seq.load(Ordering::Relaxed)
    }

    pub(crate) fn update_seq(&self, seq: u64) {
        self.last_seq.store(seq, Ordering::Relaxed);
    }

    pub(crate) fn set_input_tx(&self, tx: tokio::sync::mpsc::Sender<Vec<u8>>) {
        if let Ok(mut slot) = self.input_tx.lock() {
            *slot = Some(tx);
        }
    }

    /// Mark this terminal as having pending output and push a frame-force callback.
    pub(crate) fn signal_needs_render(&self) {
        self.needs_render.store(true, Ordering::Release);
        crate::push_callback(Box::new(|| {}));
    }

    /// Returns a `Send` closure that routes bytes into this terminal's input stream.
    pub fn make_input_fn(self: &Arc<Self>) -> Box<dyn Fn(Vec<u8>) + Send + 'static> {
        let terminal = self.clone();
        Box::new(move |data| {
            terminal.send_input(data);
        })
    }

    /// Send bytes to the remote PTY. Returns `false` if disconnected.
    pub fn send_input(&self, data: Vec<u8>) -> bool {
        let sender = match self.input_tx.lock().ok().and_then(|g| g.clone()) {
            Some(tx) => tx,
            None => return false,
        };
        match sender.try_send(data) {
            Ok(()) => true,
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                tracing::warn!("terminal input channel full");
                true
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                tracing::warn!("terminal input channel closed");
                false
            }
        }
    }
}
