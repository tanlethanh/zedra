use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tracing::*;
use zedra_rpc::osc::{OscEvent, OscScanner, TerminalMeta};
use zedra_rpc::proto::{TermAttachReq, TermInput, TermOutput, ZedraProto};

/// Keep track of created terminals.
#[derive(Clone)]
pub struct RemoteTerminal(Arc<RemoteTerminalInner>);

#[derive(Default)]
pub struct RemoteTerminalInner {
    id: Mutex<String>,
    meta: Mutex<TerminalMeta>,
    osc_events: Mutex<VecDeque<OscEvent>>,
    osc_scanner: Mutex<OscScanner>,
    input_tx: Mutex<Option<mpsc::Sender<Vec<u8>>>>,
    output_rx: Mutex<Option<mpsc::Receiver<Vec<u8>>>>,
    last_seq: AtomicU64,
    /// Whether the terminal is currently attached to a remote client.
    remote_attached: AtomicBool,
    input_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    output_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl RemoteTerminalInner {
    fn update_seq(&self, seq: u64) {
        self.last_seq.store(seq, Ordering::Release);
    }

    fn store_channels(&self, input_tx: mpsc::Sender<Vec<u8>>, output_rx: mpsc::Receiver<Vec<u8>>) {
        if let (Ok(mut input_tx_slot), Ok(mut output_rx_slot)) =
            (self.input_tx.lock(), self.output_rx.lock())
        {
            *input_tx_slot = Some(input_tx);
            *output_rx_slot = Some(output_rx);
            self.remote_attached.store(true, Ordering::Release);
        }
    }

    fn take_channels(&self) -> Option<(mpsc::Sender<Vec<u8>>, mpsc::Receiver<Vec<u8>>)> {
        if let (Ok(mut input_tx_slot), Ok(mut output_rx_slot)) =
            (self.input_tx.lock(), self.output_rx.lock())
        {
            let input_tx = input_tx_slot.take()?;
            let output_rx = output_rx_slot.take()?;
            Some((input_tx, output_rx))
        } else {
            None
        }
    }
}

impl RemoteTerminal {
    pub(crate) fn new(id: String) -> Self {
        Self(Arc::new(RemoteTerminalInner {
            id: Mutex::new(id),
            meta: Mutex::new(TerminalMeta::default()),
            osc_events: Mutex::new(VecDeque::new()),
            osc_scanner: Mutex::new(OscScanner::new()),
            input_tx: Mutex::new(None),
            output_rx: Mutex::new(None),
            last_seq: AtomicU64::new(0),
            remote_attached: AtomicBool::new(false),
            input_task: Mutex::new(None),
            output_task: Mutex::new(None),
        }))
    }

    pub fn id(&self) -> String {
        self.0.id.lock().ok().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn last_seq(&self) -> u64 {
        self.0.last_seq.load(Ordering::Acquire)
    }

    pub fn update_seq(&self, seq: u64) {
        self.0.last_seq.store(seq, Ordering::Release);
    }

    pub fn update_meta(&self, title: Option<String>, cwd: Option<String>) {
        self.0
            .meta
            .lock()
            .map(|mut m| {
                m.title = title;
                m.cwd = cwd;
            })
            .ok();
    }

    pub fn scan_osc(&self, data: &[u8]) {
        let events = self
            .0
            .osc_scanner
            .lock()
            .map(|mut s| s.feed(&data))
            .unwrap_or_default();

        if !events.is_empty() {
            if let (Ok(mut meta), Ok(mut queue)) = (self.0.meta.lock(), self.0.osc_events.lock()) {
                for ev in events {
                    meta.apply(&ev);
                    queue.push_back(ev);
                }
            }
        }
    }

    pub fn meta(&self) -> TerminalMeta {
        self.0.meta.lock().map(|m| m.clone()).unwrap_or_default()
    }

    pub fn drain_osc_events(&self) -> Vec<OscEvent> {
        self.0
            .osc_events
            .lock()
            .map(|mut q| q.drain(..).collect())
            .unwrap_or_default()
    }

    pub async fn attach_remote(
        &self,
        client: &irpc::Client<ZedraProto>,
    ) -> Result<(), anyhow::Error> {
        let (irpc_input_tx, mut irpc_output_rx) = client
            .bidi_streaming::<TermAttachReq, TermInput, TermOutput>(
                TermAttachReq {
                    id: self.id(),
                    last_seq: self.last_seq(),
                },
                256,
                256,
            )
            .await?;

        info!("attached to remote terminal: {}", self.id());

        let (input_tx, mut input_rx) = mpsc::channel::<Vec<u8>>(256);
        let (output_tx, output_rx) = mpsc::channel::<Vec<u8>>(256);

        let input_task = tokio::spawn(async move {
            while let Some(data) = input_rx.recv().await {
                if let Err(e) = irpc_input_tx.send(TermInput { data }).await {
                    info!("failed to send input: {:?}", e);
                    break;
                }
            }
        });
        if let Ok(mut input_task_slot) = self.0.input_task.lock() {
            if let Some(prev_task) = input_task_slot.take() {
                prev_task.abort();
                info!("aborted previous terminal input task from reattach");
            }
            *input_task_slot = Some(input_task);
        }

        let terminal_inner = self.0.clone();
        let output_task = tokio::spawn(async move {
            loop {
                match irpc_output_rx.recv().await {
                    Ok(Some(output)) => {
                        terminal_inner.update_seq(output.seq);
                        if let Err(e) = output_tx.send(output.data).await {
                            warn!("failed to forward terminal output: {:?}", e);
                        }
                    }
                    Ok(None) => {
                        info!("remote terminal closed or sender dropped, stopping output task");
                        break;
                    }
                    Err(e) => {
                        warn!("failed to receive terminal output: {:?}", e);
                        break;
                    }
                }
            }
        });
        if let Ok(mut output_task_slot) = self.0.output_task.lock() {
            if let Some(prev_task) = output_task_slot.take() {
                prev_task.abort();
                info!("aborted previous terminal output task from reattach");
            }
            *output_task_slot = Some(output_task);
        }

        self.0.store_channels(input_tx, output_rx);
        info!("stored channels for remote terminal: {}", self.id());

        Ok(())
    }

    /// Takes ownership of the input/output channels.
    pub fn take_chanel(&self) -> Result<(mpsc::Sender<Vec<u8>>, mpsc::Receiver<Vec<u8>>), String> {
        if let Some((input_tx, output_rx)) = self.0.take_channels() {
            Ok((input_tx, output_rx))
        } else {
            Err("no input/output channels available".to_string())
        }
    }
}

// Automatically abort the input/output tasks when the RemoteTerminalInner is dropped.
// This is necessary to avoid leaking tasks when the RemoteTerminal is dropped without
// taking ownership of the input/output channels. Mainly happens when user disconnects or closes the terminal.
impl Drop for RemoteTerminalInner {
    fn drop(&mut self) {
        if let Ok(mut output_task_slot) = self.output_task.lock() {
            if let Some(task) = output_task_slot.take() {
                task.abort();
                info!("aborted previous terminal output task from drop");
            }
        }
        if let Ok(mut input_task_slot) = self.input_task.lock() {
            if let Some(task) = input_task_slot.take() {
                task.abort();
                info!("aborted previous terminal input task from drop");
            }
        }
    }
}
