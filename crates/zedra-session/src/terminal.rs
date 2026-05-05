use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tracing::*;
use zedra_rpc::proto::{TermAttachReq, TermInput, TermOutput, ZedraProto};

use crate::session_runtime;

/// Keep track of created terminals.
#[derive(Clone)]
pub struct RemoteTerminal(Arc<RemoteTerminalInner>);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum AttachState {
    #[default]
    Detached,
    Attaching,
    Attached,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AttachTask {
    Input,
    Output,
}

#[derive(Default)]
pub struct RemoteTerminalInner {
    id: Mutex<String>,
    input_tx: Mutex<Option<mpsc::Sender<Vec<u8>>>>,
    output_rx: Mutex<Option<mpsc::Receiver<Vec<u8>>>>,
    last_seq: AtomicU64,
    attach_state: Mutex<AttachState>,
    attach_generation: AtomicU64,
    input_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    output_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl RemoteTerminalInner {
    fn update_seq(&self, seq: u64) {
        self.last_seq.store(seq, Ordering::Release);
    }

    fn begin_attach(&self) -> anyhow::Result<Option<u64>> {
        let mut state = self
            .attach_state
            .lock()
            .map_err(|_| anyhow::anyhow!("terminal attach state lock poisoned"))?;
        match *state {
            AttachState::Detached => {
                *state = AttachState::Attaching;
                let generation = self
                    .attach_generation
                    .fetch_add(1, Ordering::AcqRel)
                    .wrapping_add(1);
                Ok(Some(generation))
            }
            AttachState::Attaching | AttachState::Attached => Ok(None),
        }
    }

    fn attach_failed(&self, generation: u64) {
        let Ok(mut state) = self.attach_state.lock() else {
            return;
        };
        if self.attach_generation.load(Ordering::Acquire) == generation
            && *state == AttachState::Attaching
        {
            *state = AttachState::Detached;
        }
    }

    fn finish_attach(
        &self,
        generation: u64,
        input_tx: mpsc::Sender<Vec<u8>>,
        output_rx: mpsc::Receiver<Vec<u8>>,
    ) -> anyhow::Result<bool> {
        let mut state = self
            .attach_state
            .lock()
            .map_err(|_| anyhow::anyhow!("terminal attach state lock poisoned"))?;
        if self.attach_generation.load(Ordering::Acquire) != generation
            || *state != AttachState::Attaching
        {
            return Ok(false);
        }

        let mut input_tx_slot = self
            .input_tx
            .lock()
            .map_err(|_| anyhow::anyhow!("terminal input channel lock poisoned"))?;
        let mut output_rx_slot = self
            .output_rx
            .lock()
            .map_err(|_| anyhow::anyhow!("terminal output channel lock poisoned"))?;
        *input_tx_slot = Some(input_tx);
        *output_rx_slot = Some(output_rx);
        *state = AttachState::Attached;
        Ok(true)
    }

    fn store_tasks(
        &self,
        generation: u64,
        input_task: tokio::task::JoinHandle<()>,
        output_task: tokio::task::JoinHandle<()>,
    ) {
        if self.attach_generation.load(Ordering::Acquire) != generation {
            input_task.abort();
            output_task.abort();
            return;
        }

        if let (Ok(mut input_task_slot), Ok(mut output_task_slot)) =
            (self.input_task.lock(), self.output_task.lock())
        {
            if let Some(prev_task) = input_task_slot.take() {
                prev_task.abort();
                info!("aborted previous terminal input task from reattach");
            }
            if let Some(prev_task) = output_task_slot.take() {
                prev_task.abort();
                info!("aborted previous terminal output task from reattach");
            }
            *input_task_slot = Some(input_task);
            *output_task_slot = Some(output_task);
        } else {
            input_task.abort();
            output_task.abort();
            self.detach_remote();
        }
    }

    fn take_channels(&self) -> Option<(mpsc::Sender<Vec<u8>>, mpsc::Receiver<Vec<u8>>)> {
        if let (Ok(mut input_tx_slot), Ok(mut output_rx_slot)) =
            (self.input_tx.lock(), self.output_rx.lock())
        {
            match (input_tx_slot.take(), output_rx_slot.take()) {
                (Some(input_tx), Some(output_rx)) => Some((input_tx, output_rx)),
                (input_tx, output_rx) => {
                    *input_tx_slot = input_tx;
                    *output_rx_slot = output_rx;
                    None
                }
            }
        } else {
            None
        }
    }

    fn clear_channels(&self) {
        if let Ok(mut input_tx_slot) = self.input_tx.lock() {
            *input_tx_slot = None;
        }
        if let Ok(mut output_rx_slot) = self.output_rx.lock() {
            *output_rx_slot = None;
        }
    }

    fn abort_tasks(&self, exiting_task: Option<AttachTask>) {
        if exiting_task != Some(AttachTask::Input) {
            if let Ok(mut input_task_slot) = self.input_task.lock() {
                if let Some(task) = input_task_slot.take() {
                    task.abort();
                    info!("aborted terminal input task from detach");
                }
            }
        } else if let Ok(mut input_task_slot) = self.input_task.lock() {
            let _ = input_task_slot.take();
        }

        if exiting_task != Some(AttachTask::Output) {
            if let Ok(mut output_task_slot) = self.output_task.lock() {
                if let Some(task) = output_task_slot.take() {
                    task.abort();
                    info!("aborted terminal output task from detach");
                }
            }
        } else if let Ok(mut output_task_slot) = self.output_task.lock() {
            let _ = output_task_slot.take();
        }
    }

    fn detach_remote(&self) {
        self.attach_generation.fetch_add(1, Ordering::AcqRel);
        if let Ok(mut state) = self.attach_state.lock() {
            *state = AttachState::Detached;
        }
        self.clear_channels();
        self.abort_tasks(None);
    }

    fn teardown_if_current(&self, generation: u64, exiting_task: AttachTask) {
        let should_teardown = {
            let Ok(mut state) = self.attach_state.lock() else {
                return;
            };
            if self.attach_generation.load(Ordering::Acquire) == generation
                && *state != AttachState::Detached
            {
                *state = AttachState::Detached;
                true
            } else {
                false
            }
        };

        if should_teardown {
            // Any TermAttach task exit means the remote stream is no longer a
            // valid bridge for this generation; stale exits must not clear a
            // newer attach that has already advanced the generation.
            self.clear_channels();
            self.abort_tasks(Some(exiting_task));
        }
    }
}

impl RemoteTerminal {
    pub(crate) fn new(id: String) -> Self {
        Self(Arc::new(RemoteTerminalInner {
            id: Mutex::new(id),
            input_tx: Mutex::new(None),
            output_rx: Mutex::new(None),
            last_seq: AtomicU64::new(0),
            attach_state: Mutex::new(AttachState::Detached),
            attach_generation: AtomicU64::new(0),
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

    pub async fn attach_remote(
        &self,
        client: &irpc::Client<ZedraProto>,
    ) -> Result<(), anyhow::Error> {
        let term_id = self.id();
        let Some(generation) = self.0.begin_attach()? else {
            info!("remote terminal attach already active: {}", term_id);
            return Ok(());
        };

        let last_seq = self.last_seq();
        let (irpc_input_tx, mut irpc_output_rx) = match client
            .bidi_streaming::<TermAttachReq, TermInput, TermOutput>(
                TermAttachReq {
                    id: term_id.clone(),
                    last_seq,
                },
                256,
                256,
            )
            .await
        {
            Ok(streams) => streams,
            Err(e) => {
                self.0.attach_failed(generation);
                return Err(e.into());
            }
        };

        info!("attached to remote terminal: {}", term_id);

        let (input_tx, mut input_rx) = mpsc::channel::<Vec<u8>>(256);
        let (output_tx, output_rx) = mpsc::channel::<Vec<u8>>(256);
        match self.0.finish_attach(generation, input_tx, output_rx) {
            Ok(true) => {}
            Ok(false) => {
                info!("dropping stale remote terminal attach: {}", term_id);
                return Ok(());
            }
            Err(e) => {
                self.0.attach_failed(generation);
                return Err(e);
            }
        }

        let terminal_inner = self.0.clone();
        let input_task = session_runtime().spawn(async move {
            while let Some(data) = input_rx.recv().await {
                if let Err(e) = irpc_input_tx.send(TermInput { data }).await {
                    info!("failed to send input: {:?}", e);
                    break;
                }
            }
            terminal_inner.teardown_if_current(generation, AttachTask::Input);
        });

        let terminal_inner = self.0.clone();
        let output_task = session_runtime().spawn(async move {
            loop {
                match irpc_output_rx.recv().await {
                    Ok(Some(output)) => {
                        if output.seq != 0 {
                            terminal_inner.update_seq(output.seq);
                        }
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
            terminal_inner.teardown_if_current(generation, AttachTask::Output);
        });

        self.0.store_tasks(generation, input_task, output_task);
        info!("stored channels for remote terminal: {}", term_id);

        Ok(())
    }

    pub fn detach_remote(&self) {
        self.0.detach_remote();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_attach_is_ignored_while_active() {
        let terminal = RemoteTerminal::new("term-1".to_string());

        let first_generation = terminal.0.begin_attach().unwrap();
        assert!(first_generation.is_some());
        assert_eq!(terminal.0.begin_attach().unwrap(), None);

        terminal.0.attach_failed(first_generation.unwrap());
        assert!(terminal.0.begin_attach().unwrap().is_some());
    }

    #[test]
    fn stale_teardown_does_not_clear_newer_attach_channels() {
        let terminal = RemoteTerminal::new("term-1".to_string());

        let generation_1 = terminal.0.begin_attach().unwrap().unwrap();
        let (input_tx_1, _input_rx_1) = mpsc::channel(1);
        let (_output_tx_1, output_rx_1) = mpsc::channel(1);
        assert!(
            terminal
                .0
                .finish_attach(generation_1, input_tx_1, output_rx_1)
                .unwrap()
        );

        terminal
            .0
            .teardown_if_current(generation_1, AttachTask::Input);

        let generation_2 = terminal.0.begin_attach().unwrap().unwrap();
        let (input_tx_2, _input_rx_2) = mpsc::channel(1);
        let (_output_tx_2, output_rx_2) = mpsc::channel(1);
        assert!(
            terminal
                .0
                .finish_attach(generation_2, input_tx_2, output_rx_2)
                .unwrap()
        );

        terminal
            .0
            .teardown_if_current(generation_1, AttachTask::Output);

        assert!(terminal.take_chanel().is_ok());
    }
}
