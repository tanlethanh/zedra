// Async I/O bridge between SSH channel and terminal state
// Reads from SSH channel and posts data to GPUI terminal view

use anyhow::Result;
use gpui::{AsyncApp, WeakEntity};
use russh::{client::Msg, ChannelMsg};
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;

use crate::TerminalSink;

/// Bridges SSH channel I/O with the terminal view
pub struct SSHBridge {
    /// Channel for sending bytes from the terminal to SSH (user input)
    input_tx: mpsc::UnboundedSender<Vec<u8>>,
}

impl SSHBridge {
    /// Start the bridge between an SSH channel and a terminal view.
    /// Returns an SSHBridge with a sender for user input.
    ///
    /// Spawns two async tasks:
    /// 1. Read from SSH channel → feed into terminal view (GPUI-spawned)
    /// 2. Read from input sender → write to SSH channel (tokio-spawned)
    pub fn start<T: TerminalSink>(
        mut channel: russh::Channel<Msg>,
        terminal_view: WeakEntity<T>,
        cx: &mut AsyncApp,
    ) -> Result<Self> {
        let (input_tx, mut input_rx) = mpsc::unbounded_channel::<Vec<u8>>();

        // Create a writer handle from the channel (doesn't consume it)
        let mut writer = channel.make_writer();

        // Task 1: User input → SSH channel (tokio task, no GPUI dependency)
        tokio::spawn(async move {
            while let Some(bytes) = input_rx.recv().await {
                if writer.write_all(&bytes).await.is_err() {
                    break;
                }
            }
        });

        // Task 2: SSH output → terminal view (GPUI-spawned for main thread access)
        let view = terminal_view.clone();
        cx.spawn(async move |cx| {
            while let Some(msg) = channel.wait().await {
                match msg {
                    ChannelMsg::Data { data } => {
                        let bytes = data.to_vec();
                        let v = view.clone();
                        if v.update(cx, |this, cx| {
                            this.advance_bytes(&bytes);
                            cx.notify();
                        })
                        .is_err()
                        {
                            break;
                        }
                    }
                    ChannelMsg::ExitStatus { exit_status } => {
                        log::info!("Shell exited with status: {}", exit_status);
                        let v = view.clone();
                        let _ = v.update(cx, |this, cx| {
                            this.set_connected(false);
                            this.set_status(format!("Exited ({})", exit_status));
                            cx.notify();
                        });
                        break;
                    }
                    ChannelMsg::Eof => {
                        log::info!("SSH channel EOF");
                        let v = view.clone();
                        let _ = v.update(cx, |this, cx| {
                            this.set_connected(false);
                            this.set_status("Connection closed".to_string());
                            cx.notify();
                        });
                        break;
                    }
                    _ => {}
                }
            }
            Ok::<(), anyhow::Error>(())
        })
        .detach();

        Ok(Self { input_tx })
    }

    /// Send bytes from the terminal (user keystroke) to SSH
    pub fn send(&self, bytes: Vec<u8>) {
        let _ = self.input_tx.send(bytes);
    }

    /// Get a clone of the input sender for use as a callback
    pub fn sender(&self) -> mpsc::UnboundedSender<Vec<u8>> {
        self.input_tx.clone()
    }
}
