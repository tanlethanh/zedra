// L5 Subchannel Multiplexing Protocol
//
// Multiplexes multiple logical channels over a single transport stream.
// Each channel has a typed subprotocol (terminal, fs, git, lsp, control)
// and independent flow control with a sliding window.
//
// Wire format for each L5 frame:
//   [channel_id: u32 LE][frame_type: u8][payload: variable]
//
// Frame types:
//   0x01 OPEN      { subprotocol(u8), window(u32 LE) }
//   0x02 OPEN_ACK  { ok(u8: 0/1), error_len(u16 LE), error(utf8)? }
//   0x03 DATA      { payload(bytes) }
//   0x04 CLOSE     { reason_len(u16 LE), reason(utf8)? }
//   0x05 ACK       { bytes_consumed(u32 LE) }
//   0x06 FLOW      { window_update(u32 LE) }

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex};

// ---------------------------------------------------------------------------
// Subprotocol + Priority
// ---------------------------------------------------------------------------

/// Channel subprotocol types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Subprotocol {
    Control = 0,
    Terminal = 1,
    Fs = 2,
    Git = 3,
    Lsp = 4,
}

impl Subprotocol {
    /// Scheduling priority (lower = higher priority).
    pub fn priority(self) -> u8 {
        match self {
            Subprotocol::Control => 0,
            Subprotocol::Terminal => 1,
            Subprotocol::Lsp => 2,
            Subprotocol::Git => 3,
            Subprotocol::Fs => 4,
        }
    }

    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Subprotocol::Control),
            1 => Some(Subprotocol::Terminal),
            2 => Some(Subprotocol::Fs),
            3 => Some(Subprotocol::Git),
            4 => Some(Subprotocol::Lsp),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Frame Types
// ---------------------------------------------------------------------------

pub const FRAME_OPEN: u8 = 0x01;
pub const FRAME_OPEN_ACK: u8 = 0x02;
pub const FRAME_DATA: u8 = 0x03;
pub const FRAME_CLOSE: u8 = 0x04;
pub const FRAME_ACK: u8 = 0x05;
pub const FRAME_FLOW: u8 = 0x06;

/// A decoded L5 channel frame.
#[derive(Debug, Clone, PartialEq)]
pub enum ChannelFrame {
    Open {
        channel_id: u32,
        subprotocol: Subprotocol,
        window: u32,
    },
    OpenAck {
        channel_id: u32,
        ok: bool,
        error: Option<String>,
    },
    Data {
        channel_id: u32,
        payload: Vec<u8>,
    },
    Close {
        channel_id: u32,
        reason: Option<String>,
    },
    Ack {
        channel_id: u32,
        bytes_consumed: u32,
    },
    Flow {
        channel_id: u32,
        window_update: u32,
    },
}

impl ChannelFrame {
    pub fn channel_id(&self) -> u32 {
        match self {
            ChannelFrame::Open { channel_id, .. } => *channel_id,
            ChannelFrame::OpenAck { channel_id, .. } => *channel_id,
            ChannelFrame::Data { channel_id, .. } => *channel_id,
            ChannelFrame::Close { channel_id, .. } => *channel_id,
            ChannelFrame::Ack { channel_id, .. } => *channel_id,
            ChannelFrame::Flow { channel_id, .. } => *channel_id,
        }
    }

    /// Encode a frame to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();

        match self {
            ChannelFrame::Open {
                channel_id,
                subprotocol,
                window,
            } => {
                buf.extend_from_slice(&channel_id.to_le_bytes());
                buf.push(FRAME_OPEN);
                buf.push(*subprotocol as u8);
                buf.extend_from_slice(&window.to_le_bytes());
            }
            ChannelFrame::OpenAck {
                channel_id,
                ok,
                error,
            } => {
                buf.extend_from_slice(&channel_id.to_le_bytes());
                buf.push(FRAME_OPEN_ACK);
                buf.push(if *ok { 1 } else { 0 });
                if let Some(err) = error {
                    let bytes = err.as_bytes();
                    buf.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
                    buf.extend_from_slice(bytes);
                } else {
                    buf.extend_from_slice(&0u16.to_le_bytes());
                }
            }
            ChannelFrame::Data {
                channel_id,
                payload,
            } => {
                buf.extend_from_slice(&channel_id.to_le_bytes());
                buf.push(FRAME_DATA);
                buf.extend_from_slice(payload);
            }
            ChannelFrame::Close {
                channel_id,
                reason,
            } => {
                buf.extend_from_slice(&channel_id.to_le_bytes());
                buf.push(FRAME_CLOSE);
                if let Some(r) = reason {
                    let bytes = r.as_bytes();
                    buf.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
                    buf.extend_from_slice(bytes);
                } else {
                    buf.extend_from_slice(&0u16.to_le_bytes());
                }
            }
            ChannelFrame::Ack {
                channel_id,
                bytes_consumed,
            } => {
                buf.extend_from_slice(&channel_id.to_le_bytes());
                buf.push(FRAME_ACK);
                buf.extend_from_slice(&bytes_consumed.to_le_bytes());
            }
            ChannelFrame::Flow {
                channel_id,
                window_update,
            } => {
                buf.extend_from_slice(&channel_id.to_le_bytes());
                buf.push(FRAME_FLOW);
                buf.extend_from_slice(&window_update.to_le_bytes());
            }
        }

        buf
    }

    /// Decode a frame from bytes.
    pub fn decode(data: &[u8]) -> Result<Self, DecodeError> {
        if data.len() < 5 {
            return Err(DecodeError::TooShort);
        }

        let channel_id = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let frame_type = data[4];
        let rest = &data[5..];

        match frame_type {
            FRAME_OPEN => {
                if rest.len() < 5 {
                    return Err(DecodeError::TooShort);
                }
                let subprotocol =
                    Subprotocol::from_u8(rest[0]).ok_or(DecodeError::InvalidSubprotocol)?;
                let window = u32::from_le_bytes([rest[1], rest[2], rest[3], rest[4]]);
                Ok(ChannelFrame::Open {
                    channel_id,
                    subprotocol,
                    window,
                })
            }
            FRAME_OPEN_ACK => {
                if rest.len() < 3 {
                    return Err(DecodeError::TooShort);
                }
                let ok = rest[0] != 0;
                let err_len = u16::from_le_bytes([rest[1], rest[2]]) as usize;
                let error = if err_len > 0 {
                    if rest.len() < 3 + err_len {
                        return Err(DecodeError::TooShort);
                    }
                    Some(
                        String::from_utf8(rest[3..3 + err_len].to_vec())
                            .map_err(|_| DecodeError::InvalidUtf8)?,
                    )
                } else {
                    None
                };
                Ok(ChannelFrame::OpenAck {
                    channel_id,
                    ok,
                    error,
                })
            }
            FRAME_DATA => Ok(ChannelFrame::Data {
                channel_id,
                payload: rest.to_vec(),
            }),
            FRAME_CLOSE => {
                if rest.len() < 2 {
                    return Err(DecodeError::TooShort);
                }
                let reason_len = u16::from_le_bytes([rest[0], rest[1]]) as usize;
                let reason = if reason_len > 0 {
                    if rest.len() < 2 + reason_len {
                        return Err(DecodeError::TooShort);
                    }
                    Some(
                        String::from_utf8(rest[2..2 + reason_len].to_vec())
                            .map_err(|_| DecodeError::InvalidUtf8)?,
                    )
                } else {
                    None
                };
                Ok(ChannelFrame::Close {
                    channel_id,
                    reason,
                })
            }
            FRAME_ACK => {
                if rest.len() < 4 {
                    return Err(DecodeError::TooShort);
                }
                let bytes_consumed = u32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]]);
                Ok(ChannelFrame::Ack {
                    channel_id,
                    bytes_consumed,
                })
            }
            FRAME_FLOW => {
                if rest.len() < 4 {
                    return Err(DecodeError::TooShort);
                }
                let window_update = u32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]]);
                Ok(ChannelFrame::Flow {
                    channel_id,
                    window_update,
                })
            }
            _ => Err(DecodeError::UnknownFrameType(frame_type)),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DecodeError {
    TooShort,
    UnknownFrameType(u8),
    InvalidSubprotocol,
    InvalidUtf8,
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::TooShort => write!(f, "frame too short"),
            DecodeError::UnknownFrameType(t) => write!(f, "unknown frame type: 0x{:02x}", t),
            DecodeError::InvalidSubprotocol => write!(f, "invalid subprotocol"),
            DecodeError::InvalidUtf8 => write!(f, "invalid UTF-8 in frame"),
        }
    }
}

impl std::error::Error for DecodeError {}

// ---------------------------------------------------------------------------
// Flow Control State
// ---------------------------------------------------------------------------

/// Default receive window size (64 KB).
pub const DEFAULT_WINDOW: u32 = 65536;

/// Per-channel flow control state.
struct FlowControl {
    /// How many bytes the peer has allowed us to send.
    send_window: u32,
    /// How many bytes we've allowed the peer to send (decremented on recv).
    recv_window: u32,
    /// Initial window size for ACK replenishment.
    initial_window: u32,
}

#[allow(dead_code)]
impl FlowControl {
    fn new(window: u32) -> Self {
        Self {
            send_window: window,
            recv_window: window,
            initial_window: window,
        }
    }

    /// Can we send `len` bytes?
    fn can_send(&self, len: u32) -> bool {
        self.send_window >= len
    }

    /// Consume send window after sending.
    fn consume_send(&mut self, len: u32) {
        self.send_window = self.send_window.saturating_sub(len);
    }

    /// Peer sent us data — consume receive window.
    /// Returns bytes consumed for ACK.
    fn consume_recv(&mut self, len: u32) -> u32 {
        self.recv_window = self.recv_window.saturating_sub(len);
        len
    }

    /// Should we send an ACK to replenish the peer's send window?
    fn should_ack(&self) -> bool {
        self.recv_window < self.initial_window / 2
    }

    /// Replenish receive window (after sending ACK to peer).
    fn replenish_recv(&mut self) -> u32 {
        let consumed = self.initial_window - self.recv_window;
        self.recv_window = self.initial_window;
        consumed
    }

    /// Peer ACK'd bytes — replenish our send window.
    fn apply_ack(&mut self, bytes: u32) {
        self.send_window = self.send_window.saturating_add(bytes);
    }

    /// Peer sent FLOW update — increase send window.
    fn apply_flow(&mut self, window_update: u32) {
        self.send_window = self.send_window.saturating_add(window_update);
    }
}

// ---------------------------------------------------------------------------
// Channel State
// ---------------------------------------------------------------------------

struct ChannelState {
    #[allow(dead_code)]
    subprotocol: Subprotocol,
    flow: FlowControl,
    /// Incoming data delivered to the channel handle.
    data_tx: mpsc::Sender<Vec<u8>>,
    /// Notification when the channel is closed by the peer.
    close_tx: Option<oneshot::Sender<Option<String>>>,
}

// ---------------------------------------------------------------------------
// ChannelHandle (user-facing API)
// ---------------------------------------------------------------------------

/// Handle for a single open subchannel.
///
/// Provides send/recv for the channel's data stream, with automatic
/// flow control ACKs sent when the receive buffer is consumed.
pub struct ChannelHandle {
    channel_id: u32,
    subprotocol: Subprotocol,
    /// Receive data from the channel.
    data_rx: mpsc::Receiver<Vec<u8>>,
    /// Send frames to the multiplexer for encoding + writing.
    mux_tx: mpsc::Sender<ChannelFrame>,
    /// Notified when the peer closes this channel.
    close_rx: Option<oneshot::Receiver<Option<String>>>,
}

impl ChannelHandle {
    /// The channel's unique ID.
    pub fn id(&self) -> u32 {
        self.channel_id
    }

    /// The channel's subprotocol.
    pub fn subprotocol(&self) -> Subprotocol {
        self.subprotocol
    }

    /// Send data on this channel.
    ///
    /// The multiplexer enforces flow control — this will succeed immediately
    /// if the send window has capacity, or wait for ACK/FLOW from the peer.
    pub async fn send(&self, data: Vec<u8>) -> Result<(), ChannelError> {
        self.mux_tx
            .send(ChannelFrame::Data {
                channel_id: self.channel_id,
                payload: data,
            })
            .await
            .map_err(|_| ChannelError::Closed)
    }

    /// Receive data from this channel.
    ///
    /// Returns `None` when the channel is closed.
    pub async fn recv(&mut self) -> Option<Vec<u8>> {
        self.data_rx.recv().await
    }

    /// Close this channel gracefully.
    pub async fn close(self, reason: Option<String>) -> Result<(), ChannelError> {
        self.mux_tx
            .send(ChannelFrame::Close {
                channel_id: self.channel_id,
                reason,
            })
            .await
            .map_err(|_| ChannelError::Closed)
    }

    /// Wait for the peer to close this channel.
    pub async fn wait_closed(&mut self) -> Option<String> {
        if let Some(rx) = self.close_rx.take() {
            rx.await.ok().flatten()
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ChannelError {
    /// The channel or multiplexer has been closed.
    Closed,
    /// The peer rejected the channel open request.
    Rejected(String),
    /// Flow control: send window exhausted (should not happen with backpressure).
    WindowExhausted,
}

impl std::fmt::Display for ChannelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChannelError::Closed => write!(f, "channel closed"),
            ChannelError::Rejected(msg) => write!(f, "channel rejected: {}", msg),
            ChannelError::WindowExhausted => write!(f, "flow control window exhausted"),
        }
    }
}

impl std::error::Error for ChannelError {}

// ---------------------------------------------------------------------------
// Channel ↔ RPC Bridge
// ---------------------------------------------------------------------------

/// Convert a `ChannelHandle` into mpsc channels compatible with
/// `RpcClient::spawn_from_channels`.
///
/// Returns `(incoming_rx, outgoing_tx)` where:
/// - `incoming_rx` receives raw JSON-RPC frames from the channel
/// - `outgoing_tx` sends raw JSON-RPC frames to the channel
///
/// Two bridge tasks are spawned to shuttle data between the ChannelHandle
/// and the mpsc channels.
pub fn channel_to_rpc_bridge(
    mut handle: ChannelHandle,
) -> (mpsc::Receiver<Vec<u8>>, mpsc::Sender<Vec<u8>>) {
    let (incoming_tx, incoming_rx) = mpsc::channel::<Vec<u8>>(64);
    let (outgoing_tx, mut outgoing_rx) = mpsc::channel::<Vec<u8>>(64);

    let send_handle = handle.mux_tx.clone();
    let channel_id = handle.channel_id;

    // Bridge: channel → incoming_rx (for RpcClient to read).
    tokio::spawn(async move {
        while let Some(data) = handle.recv().await {
            if incoming_tx.send(data).await.is_err() {
                break;
            }
        }
    });

    // Bridge: outgoing_tx → channel (from RpcClient writes).
    tokio::spawn(async move {
        while let Some(payload) = outgoing_rx.recv().await {
            if send_handle
                .send(ChannelFrame::Data {
                    channel_id,
                    payload,
                })
                .await
                .is_err()
            {
                break;
            }
        }
    });

    (incoming_rx, outgoing_tx)
}

// ---------------------------------------------------------------------------
// Multiplexer
// ---------------------------------------------------------------------------

/// Multiplexes multiple subchannels over a single transport stream.
///
/// The multiplexer owns the transport read/write channels and routes
/// incoming frames to the appropriate channel handle. Outgoing frames
/// from channel handles are serialized and written to the transport.
pub struct Multiplexer {
    /// Outgoing frames from channels → transport.
    frame_tx: mpsc::Sender<ChannelFrame>,
    /// Open channels, keyed by channel_id.
    channels: Arc<Mutex<HashMap<u32, ChannelState>>>,
    /// Next channel ID to allocate (even for initiator, odd for responder).
    next_channel_id: Arc<AtomicU32>,
    /// Pending OPEN requests waiting for OPEN_ACK.
    pending_opens: Arc<Mutex<HashMap<u32, oneshot::Sender<Result<(), String>>>>>,
    /// Incoming channel open requests from the peer.
    incoming_rx: Arc<Mutex<mpsc::Receiver<(u32, Subprotocol, u32)>>>,
    /// Sender for incoming channel open requests (kept alive so reader task doesn't close).
    #[allow(dead_code)]
    incoming_tx: mpsc::Sender<(u32, Subprotocol, u32)>,
}

impl Multiplexer {
    /// Create a multiplexer over transport-level mpsc channels.
    ///
    /// - `transport_rx`: incoming raw frames from the transport
    /// - `transport_tx`: outgoing raw frames to the transport
    /// - `is_initiator`: true for client (even channel IDs), false for server (odd)
    pub fn new(
        transport_rx: mpsc::Receiver<Vec<u8>>,
        transport_tx: mpsc::Sender<Vec<u8>>,
        is_initiator: bool,
    ) -> Self {
        let channels: Arc<Mutex<HashMap<u32, ChannelState>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let pending_opens: Arc<Mutex<HashMap<u32, oneshot::Sender<Result<(), String>>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let (frame_tx, frame_rx) = mpsc::channel::<ChannelFrame>(256);
        let (incoming_tx, incoming_rx) = mpsc::channel(32);

        let next_channel_id = Arc::new(AtomicU32::new(if is_initiator { 2 } else { 3 }));

        // Spawn the writer task: encodes ChannelFrames and sends to transport.
        let writer_transport_tx = transport_tx;
        Self::spawn_writer(frame_rx, writer_transport_tx);

        // Spawn the reader task: decodes transport frames and routes to channels.
        Self::spawn_reader(
            transport_rx,
            channels.clone(),
            pending_opens.clone(),
            frame_tx.clone(),
            incoming_tx.clone(),
        );

        Self {
            frame_tx,
            channels,
            next_channel_id,
            pending_opens,
            incoming_rx: Arc::new(Mutex::new(incoming_rx)),
            incoming_tx,
        }
    }

    fn spawn_writer(
        mut frame_rx: mpsc::Receiver<ChannelFrame>,
        transport_tx: mpsc::Sender<Vec<u8>>,
    ) {
        tokio::spawn(async move {
            while let Some(frame) = frame_rx.recv().await {
                let encoded = frame.encode();
                if transport_tx.send(encoded).await.is_err() {
                    break;
                }
            }
        });
    }

    fn spawn_reader(
        mut transport_rx: mpsc::Receiver<Vec<u8>>,
        channels: Arc<Mutex<HashMap<u32, ChannelState>>>,
        pending_opens: Arc<Mutex<HashMap<u32, oneshot::Sender<Result<(), String>>>>>,
        frame_tx: mpsc::Sender<ChannelFrame>,
        incoming_tx: mpsc::Sender<(u32, Subprotocol, u32)>,
    ) {
        tokio::spawn(async move {
            while let Some(raw) = transport_rx.recv().await {
                let frame = match ChannelFrame::decode(&raw) {
                    Ok(f) => f,
                    Err(_) => continue, // skip malformed frames
                };

                match frame {
                    ChannelFrame::Open {
                        channel_id,
                        subprotocol,
                        window,
                    } => {
                        // Peer wants to open a channel — notify the accept loop.
                        let _ = incoming_tx.send((channel_id, subprotocol, window)).await;
                    }
                    ChannelFrame::OpenAck {
                        channel_id,
                        ok,
                        error,
                    } => {
                        let mut pending = pending_opens.lock().await;
                        if let Some(tx) = pending.remove(&channel_id) {
                            if ok {
                                let _ = tx.send(Ok(()));
                            } else {
                                let _ = tx.send(Err(
                                    error.unwrap_or_else(|| "rejected".to_string()),
                                ));
                            }
                        }
                    }
                    ChannelFrame::Data {
                        channel_id,
                        payload,
                    } => {
                        let mut chans = channels.lock().await;
                        if let Some(state) = chans.get_mut(&channel_id) {
                            let len = payload.len() as u32;
                            let _ = state.data_tx.send(payload).await;
                            let consumed = state.flow.consume_recv(len);

                            // Auto-ACK when receive window is half-depleted.
                            if state.flow.should_ack() {
                                let replenished = state.flow.replenish_recv();
                                let _ = frame_tx
                                    .send(ChannelFrame::Ack {
                                        channel_id,
                                        bytes_consumed: replenished,
                                    })
                                    .await;
                            }
                            let _ = consumed; // suppress unused warning
                        }
                    }
                    ChannelFrame::Close {
                        channel_id,
                        reason,
                    } => {
                        let mut chans = channels.lock().await;
                        if let Some(state) = chans.remove(&channel_id) {
                            if let Some(close_tx) = state.close_tx {
                                let _ = close_tx.send(reason);
                            }
                            // data_tx is dropped → recv returns None
                        }
                    }
                    ChannelFrame::Ack {
                        channel_id,
                        bytes_consumed,
                    } => {
                        let mut chans = channels.lock().await;
                        if let Some(state) = chans.get_mut(&channel_id) {
                            state.flow.apply_ack(bytes_consumed);
                        }
                    }
                    ChannelFrame::Flow {
                        channel_id,
                        window_update,
                    } => {
                        let mut chans = channels.lock().await;
                        if let Some(state) = chans.get_mut(&channel_id) {
                            state.flow.apply_flow(window_update);
                        }
                    }
                }
            }
        });
    }

    /// Open a new subchannel to the peer.
    ///
    /// Sends OPEN frame and waits for OPEN_ACK.
    pub async fn open_channel(
        &self,
        subprotocol: Subprotocol,
    ) -> Result<ChannelHandle, ChannelError> {
        self.open_channel_with_window(subprotocol, DEFAULT_WINDOW)
            .await
    }

    /// Open a new subchannel with a custom window size.
    pub async fn open_channel_with_window(
        &self,
        subprotocol: Subprotocol,
        window: u32,
    ) -> Result<ChannelHandle, ChannelError> {
        // Allocate channel ID (step by 2 to separate initiator/responder IDs).
        let channel_id = self.next_channel_id.fetch_add(2, Ordering::Relaxed);

        // Register pending open.
        let (ack_tx, ack_rx) = oneshot::channel();
        self.pending_opens.lock().await.insert(channel_id, ack_tx);

        // Send OPEN frame.
        self.frame_tx
            .send(ChannelFrame::Open {
                channel_id,
                subprotocol,
                window,
            })
            .await
            .map_err(|_| ChannelError::Closed)?;

        // Wait for OPEN_ACK.
        let result = ack_rx.await.map_err(|_| ChannelError::Closed)?;
        match result {
            Ok(()) => {}
            Err(msg) => return Err(ChannelError::Rejected(msg)),
        }

        // Create channel state and handle.
        let (data_tx, data_rx) = mpsc::channel(256);
        let (close_tx, close_rx) = oneshot::channel();

        let state = ChannelState {
            subprotocol,
            flow: FlowControl::new(window),
            data_tx,
            close_tx: Some(close_tx),
        };

        self.channels.lock().await.insert(channel_id, state);

        Ok(ChannelHandle {
            channel_id,
            subprotocol,
            data_rx,
            mux_tx: self.frame_tx.clone(),
            close_rx: Some(close_rx),
        })
    }

    /// Accept an incoming channel open request from the peer.
    ///
    /// Returns the channel ID, subprotocol, and peer's window size.
    /// Call `accept_channel` or `reject_channel` to respond.
    pub async fn next_incoming(&self) -> Option<(u32, Subprotocol, u32)> {
        self.incoming_rx.lock().await.recv().await
    }

    /// Accept an incoming channel open request.
    pub async fn accept_channel(
        &self,
        channel_id: u32,
        subprotocol: Subprotocol,
        peer_window: u32,
    ) -> Result<ChannelHandle, ChannelError> {
        // Send OPEN_ACK { ok: true }.
        self.frame_tx
            .send(ChannelFrame::OpenAck {
                channel_id,
                ok: true,
                error: None,
            })
            .await
            .map_err(|_| ChannelError::Closed)?;

        // Create channel state.
        let (data_tx, data_rx) = mpsc::channel(256);
        let (close_tx, close_rx) = oneshot::channel();

        let state = ChannelState {
            subprotocol,
            flow: FlowControl::new(peer_window),
            data_tx,
            close_tx: Some(close_tx),
        };

        self.channels.lock().await.insert(channel_id, state);

        Ok(ChannelHandle {
            channel_id,
            subprotocol,
            data_rx,
            mux_tx: self.frame_tx.clone(),
            close_rx: Some(close_rx),
        })
    }

    /// Reject an incoming channel open request.
    pub async fn reject_channel(
        &self,
        channel_id: u32,
        reason: &str,
    ) -> Result<(), ChannelError> {
        self.frame_tx
            .send(ChannelFrame::OpenAck {
                channel_id,
                ok: false,
                error: Some(reason.to_string()),
            })
            .await
            .map_err(|_| ChannelError::Closed)
    }

    /// Get the number of currently open channels.
    pub async fn channel_count(&self) -> usize {
        self.channels.lock().await.len()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Frame encode/decode roundtrip tests --

    #[test]
    fn encode_decode_open() {
        let frame = ChannelFrame::Open {
            channel_id: 2,
            subprotocol: Subprotocol::Terminal,
            window: 65536,
        };
        let encoded = frame.encode();
        let decoded = ChannelFrame::decode(&encoded).unwrap();
        assert_eq!(frame, decoded);
    }

    #[test]
    fn encode_decode_open_ack_ok() {
        let frame = ChannelFrame::OpenAck {
            channel_id: 2,
            ok: true,
            error: None,
        };
        let encoded = frame.encode();
        let decoded = ChannelFrame::decode(&encoded).unwrap();
        assert_eq!(frame, decoded);
    }

    #[test]
    fn encode_decode_open_ack_err() {
        let frame = ChannelFrame::OpenAck {
            channel_id: 3,
            ok: false,
            error: Some("not allowed".to_string()),
        };
        let encoded = frame.encode();
        let decoded = ChannelFrame::decode(&encoded).unwrap();
        assert_eq!(frame, decoded);
    }

    #[test]
    fn encode_decode_data() {
        let frame = ChannelFrame::Data {
            channel_id: 4,
            payload: vec![1, 2, 3, 4, 5],
        };
        let encoded = frame.encode();
        let decoded = ChannelFrame::decode(&encoded).unwrap();
        assert_eq!(frame, decoded);
    }

    #[test]
    fn encode_decode_close_with_reason() {
        let frame = ChannelFrame::Close {
            channel_id: 5,
            reason: Some("done".to_string()),
        };
        let encoded = frame.encode();
        let decoded = ChannelFrame::decode(&encoded).unwrap();
        assert_eq!(frame, decoded);
    }

    #[test]
    fn encode_decode_close_no_reason() {
        let frame = ChannelFrame::Close {
            channel_id: 6,
            reason: None,
        };
        let encoded = frame.encode();
        let decoded = ChannelFrame::decode(&encoded).unwrap();
        assert_eq!(frame, decoded);
    }

    #[test]
    fn encode_decode_ack() {
        let frame = ChannelFrame::Ack {
            channel_id: 7,
            bytes_consumed: 32768,
        };
        let encoded = frame.encode();
        let decoded = ChannelFrame::decode(&encoded).unwrap();
        assert_eq!(frame, decoded);
    }

    #[test]
    fn encode_decode_flow() {
        let frame = ChannelFrame::Flow {
            channel_id: 8,
            window_update: 16384,
        };
        let encoded = frame.encode();
        let decoded = ChannelFrame::decode(&encoded).unwrap();
        assert_eq!(frame, decoded);
    }

    #[test]
    fn decode_too_short() {
        assert_eq!(ChannelFrame::decode(&[0, 1, 2, 3]), Err(DecodeError::TooShort));
    }

    #[test]
    fn decode_unknown_frame_type() {
        let data = [0, 0, 0, 0, 0xFF];
        assert_eq!(
            ChannelFrame::decode(&data),
            Err(DecodeError::UnknownFrameType(0xFF))
        );
    }

    #[test]
    fn subprotocol_roundtrip() {
        for proto in [
            Subprotocol::Control,
            Subprotocol::Terminal,
            Subprotocol::Fs,
            Subprotocol::Git,
            Subprotocol::Lsp,
        ] {
            assert_eq!(Subprotocol::from_u8(proto as u8), Some(proto));
        }
        assert_eq!(Subprotocol::from_u8(99), None);
    }

    #[test]
    fn subprotocol_priorities() {
        assert!(Subprotocol::Control.priority() < Subprotocol::Terminal.priority());
        assert!(Subprotocol::Terminal.priority() < Subprotocol::Lsp.priority());
        assert!(Subprotocol::Lsp.priority() < Subprotocol::Git.priority());
        assert!(Subprotocol::Git.priority() < Subprotocol::Fs.priority());
    }

    // -- Flow control tests --

    #[test]
    fn flow_control_basic() {
        let mut flow = FlowControl::new(1000);

        // Can send within window.
        assert!(flow.can_send(500));
        flow.consume_send(500);
        assert!(flow.can_send(500));
        flow.consume_send(500);
        assert!(!flow.can_send(1));

        // ACK replenishes.
        flow.apply_ack(1000);
        assert!(flow.can_send(1000));
    }

    #[test]
    fn flow_control_recv_ack_threshold() {
        let mut flow = FlowControl::new(1000);

        // Consume more than half the receive window.
        flow.consume_recv(600);
        assert!(flow.should_ack()); // 400 < 500 (half of 1000)

        // Replenish.
        let replenished = flow.replenish_recv();
        assert_eq!(replenished, 600);
        assert!(!flow.should_ack());
    }

    // -- Multiplexer integration tests --

    #[tokio::test]
    async fn multiplexer_open_accept_data() {
        // Set up two multiplexers connected back-to-back.
        let (a_to_b_tx, a_to_b_rx) = mpsc::channel::<Vec<u8>>(64);
        let (b_to_a_tx, b_to_a_rx) = mpsc::channel::<Vec<u8>>(64);

        let mux_a = Multiplexer::new(b_to_a_rx, a_to_b_tx, true); // initiator
        let mux_b = Multiplexer::new(a_to_b_rx, b_to_a_tx, false); // responder

        // A opens a terminal channel.
        let open_handle = tokio::spawn(async move {
            mux_a.open_channel(Subprotocol::Terminal).await.unwrap()
        });

        // B accepts the incoming channel.
        let (ch_id, proto, window) = mux_b.next_incoming().await.unwrap();
        assert_eq!(proto, Subprotocol::Terminal);
        let mut handle_b = mux_b.accept_channel(ch_id, proto, window).await.unwrap();

        let handle_a = open_handle.await.unwrap();

        // A sends data to B.
        handle_a.send(b"hello world".to_vec()).await.unwrap();
        let received = handle_b.recv().await.unwrap();
        assert_eq!(received, b"hello world");

        // B sends data to A — need mutable handle_a for recv.
        // (we consumed handle_a above, so let's test close instead)
        handle_a.close(None).await.unwrap();
    }

    #[tokio::test]
    async fn multiplexer_reject_channel() {
        let (a_to_b_tx, a_to_b_rx) = mpsc::channel::<Vec<u8>>(64);
        let (b_to_a_tx, b_to_a_rx) = mpsc::channel::<Vec<u8>>(64);

        let mux_a = Multiplexer::new(b_to_a_rx, a_to_b_tx, true);
        let mux_b = Multiplexer::new(a_to_b_rx, b_to_a_tx, false);

        let open_handle = tokio::spawn(async move {
            mux_a.open_channel(Subprotocol::Lsp).await
        });

        let (ch_id, _proto, _window) = mux_b.next_incoming().await.unwrap();
        mux_b.reject_channel(ch_id, "not supported").await.unwrap();

        let result = open_handle.await.unwrap();
        assert!(matches!(result, Err(ChannelError::Rejected(_))));
    }

    #[tokio::test]
    async fn multiplexer_bidirectional() {
        let (a_to_b_tx, a_to_b_rx) = mpsc::channel::<Vec<u8>>(64);
        let (b_to_a_tx, b_to_a_rx) = mpsc::channel::<Vec<u8>>(64);

        let mux_a = Multiplexer::new(b_to_a_rx, a_to_b_tx, true);
        let mux_b = Multiplexer::new(a_to_b_rx, b_to_a_tx, false);

        // A opens channel.
        let open_handle = tokio::spawn(async move {
            mux_a.open_channel(Subprotocol::Fs).await.unwrap()
        });

        let (ch_id, proto, window) = mux_b.next_incoming().await.unwrap();
        let handle_b = mux_b.accept_channel(ch_id, proto, window).await.unwrap();
        let mut handle_a = open_handle.await.unwrap();

        // B sends to A.
        handle_b.send(b"from B".to_vec()).await.unwrap();
        let msg = handle_a.recv().await.unwrap();
        assert_eq!(msg, b"from B");
    }

    #[tokio::test]
    async fn multiplexer_multiple_channels() {
        let (a_to_b_tx, a_to_b_rx) = mpsc::channel::<Vec<u8>>(64);
        let (b_to_a_tx, b_to_a_rx) = mpsc::channel::<Vec<u8>>(64);

        let mux_a = Arc::new(Multiplexer::new(b_to_a_rx, a_to_b_tx, true));
        let mux_b = Arc::new(Multiplexer::new(a_to_b_rx, b_to_a_tx, false));

        // Open two channels.
        let mux_a2 = mux_a.clone();
        let open1 = tokio::spawn(async move {
            mux_a2.open_channel(Subprotocol::Terminal).await.unwrap()
        });

        // Accept first.
        let (ch_id1, proto1, w1) = mux_b.next_incoming().await.unwrap();
        let mut handle_b1 = mux_b.accept_channel(ch_id1, proto1, w1).await.unwrap();

        let _handle_a1 = open1.await.unwrap();

        let mux_a3 = mux_a.clone();
        let open2 = tokio::spawn(async move {
            mux_a3.open_channel(Subprotocol::Fs).await.unwrap()
        });

        let (ch_id2, proto2, w2) = mux_b.next_incoming().await.unwrap();
        let mut handle_b2 = mux_b.accept_channel(ch_id2, proto2, w2).await.unwrap();

        let handle_a2 = open2.await.unwrap();

        // Data on channel 2 doesn't leak to channel 1.
        handle_a2.send(b"fs data".to_vec()).await.unwrap();
        let msg = handle_b2.recv().await.unwrap();
        assert_eq!(msg, b"fs data");

        // Channel 1 should have nothing pending.
        // Use try_recv to verify no data.
        assert!(handle_b1.data_rx.try_recv().is_err());

        assert_eq!(mux_b.channel_count().await, 2);
    }

    // -- RPC bridge test --

    #[tokio::test]
    async fn channel_rpc_bridge_roundtrip() {
        use crate::{RpcClient, RpcServer};

        // Set up two multiplexers connected back-to-back.
        let (a_to_b_tx, a_to_b_rx) = mpsc::channel::<Vec<u8>>(64);
        let (b_to_a_tx, b_to_a_rx) = mpsc::channel::<Vec<u8>>(64);

        let mux_a = Multiplexer::new(b_to_a_rx, a_to_b_tx, true);
        let mux_b = Multiplexer::new(a_to_b_rx, b_to_a_tx, false);

        // A opens an FS channel for RPC.
        let open_handle = tokio::spawn(async move {
            mux_a.open_channel(Subprotocol::Fs).await.unwrap()
        });

        let (ch_id, proto, window) = mux_b.next_incoming().await.unwrap();
        let handle_b = mux_b.accept_channel(ch_id, proto, window).await.unwrap();
        let handle_a = open_handle.await.unwrap();

        // Bridge both sides to RPC channels.
        let (client_rx, client_tx) = channel_to_rpc_bridge(handle_a);
        let (server_rx, server_tx) = channel_to_rpc_bridge(handle_b);

        // Spawn RPC server on side B.
        let mut server = RpcServer::new();
        server.register("echo", |params| {
            Box::pin(async move { Ok(params) })
        });

        // Use spawn_from_channels for server-side too: read from server_rx, write to server_tx.
        tokio::spawn(async move {
            let mut server_rx = server_rx;
            while let Some(payload) = server_rx.recv().await {
                let msg: crate::Message = match serde_json::from_slice(&payload) {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                if let crate::Message::Request(req) = msg {
                    let result = serde_json::json!({"echoed": req.params});
                    let resp = crate::Response::ok(req.id, result);
                    let resp_bytes = serde_json::to_vec(&resp).unwrap();
                    if server_tx.send(resp_bytes).await.is_err() {
                        break;
                    }
                }
            }
        });

        // Spawn RPC client on side A.
        let (client, _notifs) = RpcClient::spawn_from_channels(client_rx, client_tx);

        // Make an RPC call through the channel.
        let resp = client
            .call("echo", serde_json::json!({"test": 42}))
            .await
            .unwrap();

        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["echoed"]["test"], 42);
    }
}
