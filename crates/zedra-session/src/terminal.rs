/// One remote PTY. Durable across reconnects; stored as `Arc<RemoteTerminal>`.
///
/// Raw PTY bytes arrive in the pump task (tokio thread), are scanned for OSC
/// sequences to extract terminal metadata, then pushed to `output` for the
/// `TerminalView` to drain each frame on the UI thread.
///
/// OSC metadata flow (message-passing, no callbacks):
///   pump task → `push_output(bytes)`
///     → `OscScanner::feed` → Vec<OscEvent>
///     → update `meta` snapshot in-place
///     → push events to `osc_events` queue
///     → push raw bytes to `output` ring buffer
///     → `signal_needs_render()` wakes the frame loop
///
/// The UI thread reads `meta()` for immediate display (title, cwd, shell
/// state colour dot) and may drain `osc_events` to react to one-shot events
/// such as bell notifications.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

// Re-export shared OSC types from zedra-rpc so crate consumers continue
// to import from zedra_session::terminal (no breaking change).
pub use zedra_rpc::osc::{OscEvent, OscScanner, ShellState, TerminalMeta};

/// Thread-safe ring buffer for terminal output chunks.
pub type OutputBuffer = Arc<Mutex<VecDeque<Vec<u8>>>>;

// ---------------------------------------------------------------------------
// RemoteTerminal
// ---------------------------------------------------------------------------

pub struct RemoteTerminal {
    pub id: String,
    /// Raw PTY output chunks; written by the pump, drained by `TerminalView::render()`.
    pub output: OutputBuffer,
    /// Set `true` by the pump when new output arrives; cleared by `TerminalView` in render.
    pub needs_render: Arc<AtomicBool>,
    /// Last-known terminal metadata snapshot, updated as OSC sequences arrive.
    pub meta: Arc<Mutex<TerminalMeta>>,
    /// OSC events waiting to be consumed by the UI thread.
    pub osc_events: Arc<Mutex<VecDeque<OscEvent>>>,
    /// Stateful OSC scanner — persisted so sequences split across chunks are handled.
    osc_scanner: Mutex<OscScanner>,
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
            meta: Arc::new(Mutex::new(TerminalMeta::default())),
            osc_events: Arc::new(Mutex::new(VecDeque::new())),
            osc_scanner: Mutex::new(OscScanner::new()),
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

    /// Write a chunk of PTY bytes:
    ///   1. Scan for OSC events and update the `meta` snapshot.
    ///   2. Push events to the `osc_events` queue for the UI thread to consume.
    ///   3. Push raw bytes to the `output` ring buffer for `TerminalView`.
    pub fn push_output(&self, data: Vec<u8>) {
        // Scan for OSC events.
        let events = self
            .osc_scanner
            .lock()
            .map(|mut s| s.feed(&data))
            .unwrap_or_default();

        // Apply events: update meta snapshot + enqueue for UI.
        if !events.is_empty() {
            let meta_ok = self.meta.lock();
            let osc_ok = self.osc_events.lock();
            if let (Ok(mut meta), Ok(mut queue)) = (meta_ok, osc_ok) {
                for ev in events {
                    meta.apply(&ev);
                    queue.push_back(ev);
                }
            }
        }

        // Push raw bytes to output buffer for VTE processing.
        if let Ok(mut buf) = self.output.lock() {
            buf.push_back(data);
        }
    }

    /// Reset the OSC scanner state (call when injecting a terminal reset sequence).
    pub(crate) fn reset_osc_scanner(&self) {
        if let Ok(mut s) = self.osc_scanner.lock() {
            s.reset();
        }
    }

    /// Snapshot of the current terminal metadata. Cheap clone.
    pub fn meta(&self) -> TerminalMeta {
        self.meta.lock().map(|m| m.clone()).unwrap_or_default()
    }

    /// Drain all pending OSC events from the queue.
    pub fn drain_osc_events(&self) -> Vec<OscEvent> {
        self.osc_events
            .lock()
            .map(|mut q| q.drain(..).collect())
            .unwrap_or_default()
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
