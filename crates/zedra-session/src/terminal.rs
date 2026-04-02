/// Remote PTY handle. Durable across reconnects.
///
/// Pump task writes PTY output → UI subscribes via `subscribe_output()` channel.
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};

pub use zedra_rpc::osc::{OscEvent, OscScanner, ShellState, TerminalMeta};

pub type OutputBuffer = Arc<Mutex<VecDeque<Vec<u8>>>>;
pub type NotifyReceiver = UnboundedReceiver<()>;

pub struct RemoteTerminal {
    pub id: String,
    pub output: OutputBuffer,
    pub needs_render: Arc<AtomicBool>,
    pub meta: Arc<Mutex<TerminalMeta>>,
    pub osc_events: Arc<Mutex<VecDeque<OscEvent>>>,
    osc_scanner: Mutex<OscScanner>,
    input_tx: Mutex<Option<tokio::sync::mpsc::Sender<Vec<u8>>>>,
    last_seq: AtomicU64,
    notify_tx: Mutex<Option<UnboundedSender<()>>>,
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
            notify_tx: Mutex::new(None),
        })
    }

    pub fn subscribe_output(&self) -> NotifyReceiver {
        let (tx, rx) = unbounded();
        if let Ok(mut g) = self.notify_tx.lock() {
            *g = Some(tx);
        }
        rx
    }

    pub fn last_seq(&self) -> u64 {
        self.last_seq.load(Ordering::Relaxed)
    }

    pub(crate) fn update_seq(&self, seq: u64) {
        self.last_seq.store(seq, Ordering::Relaxed);
    }

    pub(crate) fn set_input_tx(&self, tx: tokio::sync::mpsc::Sender<Vec<u8>>) {
        if let Ok(mut g) = self.input_tx.lock() {
            *g = Some(tx);
        }
    }

    pub fn push_output(&self, data: Vec<u8>) {
        let events = self
            .osc_scanner
            .lock()
            .map(|mut s| s.feed(&data))
            .unwrap_or_default();

        if !events.is_empty() {
            if let (Ok(mut meta), Ok(mut queue)) = (self.meta.lock(), self.osc_events.lock()) {
                for ev in events {
                    meta.apply(&ev);
                    queue.push_back(ev);
                }
            }
        }

        if let Ok(mut buf) = self.output.lock() {
            buf.push_back(data);
        }
    }

    pub(crate) fn reset_osc_scanner(&self) {
        if let Ok(mut s) = self.osc_scanner.lock() {
            s.reset();
        }
    }

    pub fn meta(&self) -> TerminalMeta {
        self.meta.lock().map(|m| m.clone()).unwrap_or_default()
    }

    pub fn drain_osc_events(&self) -> Vec<OscEvent> {
        self.osc_events
            .lock()
            .map(|mut q| q.drain(..).collect())
            .unwrap_or_default()
    }

    pub(crate) fn signal_needs_render(&self) {
        self.needs_render.store(true, Ordering::Release);
        if let Some(tx) = self.notify_tx.lock().ok().and_then(|g| g.clone()) {
            let _ = tx.unbounded_send(());
        }
    }

    pub fn make_input_fn(self: &Arc<Self>) -> Box<dyn Fn(Vec<u8>) + Send + 'static> {
        let terminal = self.clone();
        Box::new(move |data| {
            terminal.send_input(data);
        })
    }

    pub fn send_input(&self, data: Vec<u8>) -> bool {
        let Some(tx) = self.input_tx.lock().ok().and_then(|g| g.clone()) else {
            return false;
        };
        if let Err(e) = tx.try_send(data) {
            tracing::warn!("terminal input: {e}");
            return matches!(e, tokio::sync::mpsc::error::TrySendError::Full(_));
        }
        true
    }
}
