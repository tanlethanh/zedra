// Layer 4: Durable Message Queue
//
// Provides exactly-once, in-order delivery across transport reconnections.
// Inspired by Magic Wormhole's dilation L4.
//
// Every outgoing message is assigned a monotonically increasing sequence number.
// Messages stay in the outgoing buffer until ACKed by the peer. On reconnection,
// both sides exchange RESUME frames declaring last_received_seq, and the sender
// replays all unACKed messages after the peer's last_received_seq.

use std::collections::{HashSet, VecDeque};

use crate::frame::{Frame, FrameType};

/// Maximum number of unACKed outgoing messages before applying backpressure.
const MAX_UNACKED_MESSAGES: usize = 10_000;

/// Number of received DATA frames before sending a standalone ACK.
const ACK_INTERVAL: u64 = 32;

/// Durable message queue with seq/ACK for exactly-once, in-order delivery.
pub struct DurableQueue {
    // --- Outgoing ---
    /// Messages we sent but haven't been ACKed yet: (seq, encoded_payload).
    outgoing: VecDeque<(u64, Vec<u8>)>,
    /// Next sequence number to assign to outgoing DATA frames.
    next_outgoing_seq: u64,

    // --- Incoming ---
    /// Highest contiguous sequence number we've received from the peer.
    /// All seq <= this value have been delivered to the application.
    next_expected_incoming: u64,
    /// Out-of-order sequences received ahead of the contiguous range.
    out_of_order: HashSet<u64>,

    // --- State ---
    /// Current transport generation (incremented on each reconnect).
    generation: u32,
    /// Counter for received DATA frames since last standalone ACK.
    recv_since_ack: u64,
}

impl DurableQueue {
    /// Create a new queue starting from seq 1 (seq 0 is reserved for control frames).
    pub fn new() -> Self {
        Self {
            outgoing: VecDeque::new(),
            next_outgoing_seq: 1,
            next_expected_incoming: 0,
            out_of_order: HashSet::new(),
            generation: 0,
            recv_since_ack: 0,
        }
    }

    /// Current transport generation.
    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// The highest contiguous incoming seq we've received (for ACK/RESUME).
    pub fn last_received_seq(&self) -> u64 {
        self.next_expected_incoming
    }

    /// Number of unACKed outgoing messages.
    pub fn unacked_count(&self) -> usize {
        self.outgoing.len()
    }

    /// Whether the queue has room for more outgoing messages.
    pub fn can_send(&self) -> bool {
        self.outgoing.len() < MAX_UNACKED_MESSAGES
    }

    /// Wrap an application payload in an L4 DATA frame and enqueue it.
    ///
    /// Returns the encoded L4 frame bytes ready to send on the wire,
    /// or None if backpressure limit is reached.
    pub fn enqueue(&mut self, payload: Vec<u8>) -> Option<Vec<u8>> {
        if !self.can_send() {
            log::warn!(
                "DurableQueue: backpressure — {} unACKed messages at limit",
                self.outgoing.len()
            );
            return None;
        }

        let seq = self.next_outgoing_seq;
        self.next_outgoing_seq += 1;

        let frame = Frame::data(seq, self.next_expected_incoming, payload.clone());
        let encoded = frame.encode();

        self.outgoing.push_back((seq, payload));
        Some(encoded)
    }

    /// Process a received L4 frame.
    ///
    /// Returns:
    /// - `Ok(Some(payload))` if this is a DATA frame with new application data
    /// - `Ok(None)` if this is an ACK, duplicate, or control frame (no app data)
    /// - `Err` on protocol errors
    pub fn receive(&mut self, frame: &Frame) -> anyhow::Result<Option<Vec<u8>>> {
        // Always process the piggybacked ACK
        if frame.ack_seq > 0 {
            self.process_ack(frame.ack_seq);
        }

        match frame.frame_type {
            FrameType::Data => {
                let seq = frame.seq;

                if seq <= self.next_expected_incoming {
                    // Duplicate or already-delivered — ignore data but ACK was processed
                    log::debug!("DurableQueue: ignoring duplicate seq {}", seq);
                    return Ok(None);
                }

                if seq == self.next_expected_incoming + 1 {
                    // Expected next sequence — deliver to application
                    self.next_expected_incoming = seq;

                    // Check if any out-of-order messages are now contiguous
                    while self
                        .out_of_order
                        .remove(&(self.next_expected_incoming + 1))
                    {
                        self.next_expected_incoming += 1;
                    }

                    self.recv_since_ack += 1;
                    Ok(Some(frame.payload.clone()))
                } else {
                    // Out-of-order: seq > expected + 1
                    // Store for later delivery when gap is filled
                    self.out_of_order.insert(seq);
                    self.recv_since_ack += 1;

                    // For now we can't deliver this to the app since we need in-order.
                    // TODO: buffer the payload for delivery when the gap fills.
                    // For the initial implementation, we rely on the sender replaying
                    // the missing messages on reconnect.
                    log::debug!(
                        "DurableQueue: out-of-order seq {} (expected {}), buffering",
                        seq,
                        self.next_expected_incoming + 1
                    );
                    Ok(None)
                }
            }
            FrameType::Ack => {
                // Standalone ACK — process selective ACKs if present
                if let Ok(sacks) = frame.parse_selective_acks() {
                    for _s in sacks {
                        // Selective ACKs tell us specific out-of-order seqs the peer received.
                        // For now, we only use contiguous ACK. Selective ACK optimization
                        // can be added later for better performance over lossy transports.
                    }
                }
                Ok(None)
            }
            FrameType::Resume => {
                // RESUME is handled at a higher level (TransportManager)
                Ok(None)
            }
            FrameType::Reset => {
                // RESET is handled at a higher level (TransportManager)
                Ok(None)
            }
        }
    }

    /// Check if a standalone ACK should be sent.
    ///
    /// Returns Some(encoded_ack_frame) if enough DATA frames have been received
    /// since the last ACK.
    pub fn maybe_ack(&mut self) -> Option<Vec<u8>> {
        if self.recv_since_ack >= ACK_INTERVAL {
            self.recv_since_ack = 0;
            let frame = Frame::ack(self.next_expected_incoming, &[]);
            Some(frame.encode())
        } else {
            None
        }
    }

    /// Force-generate an ACK frame (e.g. before reconnecting).
    pub fn force_ack(&mut self) -> Vec<u8> {
        self.recv_since_ack = 0;
        let frame = Frame::ack(self.next_expected_incoming, &[]);
        frame.encode()
    }

    /// Process an ACK: remove all outgoing messages with seq <= ack_seq.
    fn process_ack(&mut self, ack_seq: u64) {
        while let Some(&(seq, _)) = self.outgoing.front() {
            if seq <= ack_seq {
                self.outgoing.pop_front();
            } else {
                break;
            }
        }
    }

    /// Build a RESUME frame for the current state.
    pub fn build_resume(&self) -> Vec<u8> {
        Frame::resume(self.generation, self.next_expected_incoming).encode()
    }

    /// Handle the peer's RESUME frame on reconnection.
    ///
    /// Returns a list of encoded L4 DATA frames that need to be replayed
    /// (messages the peer hasn't received yet).
    pub fn handle_peer_resume(&mut self, peer_last_received: u64) -> Vec<Vec<u8>> {
        self.generation += 1;
        self.recv_since_ack = 0;

        // Remove messages the peer has confirmed receiving
        self.process_ack(peer_last_received);

        // Build replay frames for everything still unACKed
        self.outgoing
            .iter()
            .map(|(seq, payload)| {
                Frame::data(*seq, self.next_expected_incoming, payload.clone()).encode()
            })
            .collect()
    }

    /// Reset the queue to initial state (e.g. after receiving a RESET frame).
    pub fn reset(&mut self) {
        self.outgoing.clear();
        self.next_outgoing_seq = 1;
        self.next_expected_incoming = 0;
        self.out_of_order.clear();
        self.generation += 1;
        self.recv_since_ack = 0;
    }
}

impl Default for DurableQueue {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enqueue_and_receive_roundtrip() {
        let mut sender = DurableQueue::new();
        let mut receiver = DurableQueue::new();

        // Sender enqueues a message
        let encoded = sender.enqueue(b"hello".to_vec()).unwrap();
        assert_eq!(sender.unacked_count(), 1);

        // Receiver processes it
        let frame = Frame::decode(&encoded).unwrap();
        let data = receiver.receive(&frame).unwrap();
        assert_eq!(data, Some(b"hello".to_vec()));
        assert_eq!(receiver.last_received_seq(), 1);
    }

    #[test]
    fn multiple_messages_in_order() {
        let mut sender = DurableQueue::new();
        let mut receiver = DurableQueue::new();

        for i in 1..=5 {
            let encoded = sender.enqueue(format!("msg-{}", i).into_bytes()).unwrap();
            let frame = Frame::decode(&encoded).unwrap();
            let data = receiver.receive(&frame).unwrap();
            assert_eq!(
                data,
                Some(format!("msg-{}", i).into_bytes())
            );
        }
        assert_eq!(receiver.last_received_seq(), 5);
        assert_eq!(sender.unacked_count(), 5); // nothing ACKed yet
    }

    #[test]
    fn piggybacked_ack_clears_outgoing() {
        let mut sender = DurableQueue::new();
        let mut receiver = DurableQueue::new();

        // Send 3 messages
        let mut encoded_frames = Vec::new();
        for i in 1..=3 {
            encoded_frames.push(sender.enqueue(format!("msg-{}", i).into_bytes()).unwrap());
        }
        assert_eq!(sender.unacked_count(), 3);

        // Receiver processes all 3
        for enc in &encoded_frames {
            let frame = Frame::decode(enc).unwrap();
            receiver.receive(&frame).unwrap();
        }

        // Receiver sends a message back, piggybacking ACK for seq 3
        let reply_encoded = receiver.enqueue(b"reply".to_vec()).unwrap();
        let reply_frame = Frame::decode(&reply_encoded).unwrap();
        assert_eq!(reply_frame.ack_seq, 3); // piggybacked ACK

        // Sender receives the reply — the piggybacked ACK should clear outgoing
        sender.receive(&reply_frame).unwrap();
        assert_eq!(sender.unacked_count(), 0);
    }

    #[test]
    fn duplicate_detection() {
        let mut sender = DurableQueue::new();
        let mut receiver = DurableQueue::new();

        let encoded = sender.enqueue(b"hello".to_vec()).unwrap();
        let frame = Frame::decode(&encoded).unwrap();

        // First receive: delivers data
        assert!(receiver.receive(&frame).unwrap().is_some());
        // Second receive: duplicate, no data
        assert!(receiver.receive(&frame).unwrap().is_none());
    }

    #[test]
    fn resume_and_replay() {
        let mut sender = DurableQueue::new();
        let mut receiver = DurableQueue::new();

        // Sender sends 5 messages
        for i in 1..=5 {
            sender.enqueue(format!("msg-{}", i).into_bytes()).unwrap();
        }

        // Receiver only gets the first 3
        for i in 0..3 {
            let frame = Frame::data(
                (i + 1) as u64,
                0,
                format!("msg-{}", i + 1).into_bytes(),
            );
            receiver.receive(&frame).unwrap();
        }
        assert_eq!(receiver.last_received_seq(), 3);

        // Simulate reconnection: receiver tells sender "I have up to seq 3"
        let replay_frames = sender.handle_peer_resume(3);

        // Should replay messages 4 and 5
        assert_eq!(replay_frames.len(), 2);
        let f4 = Frame::decode(&replay_frames[0]).unwrap();
        let f5 = Frame::decode(&replay_frames[1]).unwrap();
        assert_eq!(f4.seq, 4);
        assert_eq!(f5.seq, 5);
        assert_eq!(f4.payload, b"msg-4");
        assert_eq!(f5.payload, b"msg-5");

        // Receiver processes replayed messages
        for enc in &replay_frames {
            let frame = Frame::decode(enc).unwrap();
            receiver.receive(&frame).unwrap();
        }
        assert_eq!(receiver.last_received_seq(), 5);
    }

    #[test]
    fn generation_increments_on_resume() {
        let mut q = DurableQueue::new();
        assert_eq!(q.generation(), 0);
        q.handle_peer_resume(0);
        assert_eq!(q.generation(), 1);
        q.handle_peer_resume(0);
        assert_eq!(q.generation(), 2);
    }

    #[test]
    fn backpressure_at_limit() {
        let mut q = DurableQueue::new();
        // Fill to the max
        for i in 0..MAX_UNACKED_MESSAGES {
            assert!(
                q.enqueue(format!("msg-{}", i).into_bytes()).is_some(),
                "should accept message {}",
                i
            );
        }
        // Next enqueue should fail (backpressure)
        assert!(q.enqueue(b"overflow".to_vec()).is_none());
        assert!(!q.can_send());
    }

    #[test]
    fn standalone_ack_after_interval() {
        let mut q = DurableQueue::new();

        // Receive ACK_INTERVAL - 1 messages: no ACK yet
        for i in 1..ACK_INTERVAL {
            let frame = Frame::data(i, 0, vec![]);
            q.receive(&frame).unwrap();
            assert!(q.maybe_ack().is_none());
        }

        // One more pushes us to the interval
        let frame = Frame::data(ACK_INTERVAL, 0, vec![]);
        q.receive(&frame).unwrap();
        let ack = q.maybe_ack();
        assert!(ack.is_some());

        let ack_frame = Frame::decode(&ack.unwrap()).unwrap();
        assert_eq!(ack_frame.frame_type, FrameType::Ack);
        assert_eq!(ack_frame.ack_seq, ACK_INTERVAL);
    }

    #[test]
    fn reset_clears_state() {
        let mut q = DurableQueue::new();
        q.enqueue(b"msg1".to_vec());
        q.enqueue(b"msg2".to_vec());

        // Simulate receiving some messages
        let frame = Frame::data(1, 0, b"incoming".to_vec());
        q.receive(&frame).unwrap();

        assert_eq!(q.unacked_count(), 2);
        assert_eq!(q.last_received_seq(), 1);
        let gen_before = q.generation();

        q.reset();

        assert_eq!(q.unacked_count(), 0);
        assert_eq!(q.last_received_seq(), 0);
        assert_eq!(q.generation(), gen_before + 1);
    }

    #[test]
    fn force_ack_resets_counter() {
        let mut q = DurableQueue::new();

        // Receive a few messages
        for i in 1..=3 {
            let frame = Frame::data(i, 0, vec![]);
            q.receive(&frame).unwrap();
        }

        let ack = q.force_ack();
        let ack_frame = Frame::decode(&ack).unwrap();
        assert_eq!(ack_frame.ack_seq, 3);

        // Counter should be reset, so maybe_ack returns None
        assert!(q.maybe_ack().is_none());
    }
}
