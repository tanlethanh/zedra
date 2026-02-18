use anyhow::Result;
use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::client::RelayClient;
use zedra_rpc::Transport;

/// Transport implementation that sends/receives length-delimited frames
/// via the HTTP relay server using base64-encoded messages.
///
/// Adaptive polling adjusts the recv poll interval based on activity:
/// - Active (message received < 1s ago): 50ms
/// - Idle (< 30s since last message): 250ms
/// - Background (> 30s): 1s
pub struct RelayTransport {
    client: RelayClient,
    recv_buffer: VecDeque<Vec<u8>>,
    last_seq: u64,
    last_activity: Instant,
    consecutive_empty: u32,
}

impl RelayTransport {
    pub fn new(client: RelayClient) -> Self {
        Self {
            client,
            recv_buffer: VecDeque::new(),
            last_seq: 0,
            last_activity: Instant::now(),
            consecutive_empty: 0,
        }
    }

    fn adaptive_poll_interval(&self) -> Duration {
        let since_activity = self.last_activity.elapsed();
        if since_activity < Duration::from_secs(1) {
            Duration::from_millis(50)
        } else if since_activity < Duration::from_secs(30) {
            Duration::from_millis(250)
        } else {
            Duration::from_secs(1)
        }
    }
}

#[async_trait]
impl Transport for RelayTransport {
    async fn send(&mut self, payload: &[u8]) -> Result<()> {
        let encoded = BASE64.encode(payload);
        self.client.send_messages(&[encoded]).await?;
        Ok(())
    }

    async fn recv(&mut self) -> Result<Vec<u8>> {
        loop {
            // Return buffered message if available
            if let Some(msg) = self.recv_buffer.pop_front() {
                return Ok(msg);
            }

            // Poll the relay for new messages
            let resp = self.client.recv_messages(self.last_seq).await?;

            if !resp.messages.is_empty() {
                self.last_activity = Instant::now();
                self.consecutive_empty = 0;
                self.last_seq = resp.last_seq;

                for msg in resp.messages {
                    let decoded = BASE64.decode(&msg.data)?;
                    self.recv_buffer.push_back(decoded);
                }

                // Return the first message immediately
                if let Some(msg) = self.recv_buffer.pop_front() {
                    return Ok(msg);
                }
            } else {
                self.consecutive_empty += 1;
                // Update last_seq even if empty to avoid re-fetching same range
                if resp.last_seq > self.last_seq {
                    self.last_seq = resp.last_seq;
                }
            }

            // Sleep with adaptive backoff before polling again
            let interval = self.adaptive_poll_interval();
            tokio::time::sleep(interval).await;
        }
    }

    fn name(&self) -> &str {
        "relay"
    }
}
