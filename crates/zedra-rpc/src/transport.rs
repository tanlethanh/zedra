// Transport layer: length-delimited JSON-RPC over any async stream.
//
// Each message is framed as: [4-byte big-endian length][JSON payload]
// This works over TCP, Unix sockets, or WebSocket binary frames.

use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot, Mutex};

use crate::protocol::{Message, Notification, Request, Response, INTERNAL_ERROR};

// ---------------------------------------------------------------------------
// Transport trait
// ---------------------------------------------------------------------------

/// Abstraction over a framed byte transport (TCP, relay, etc.).
///
/// Each `send`/`recv` call handles one complete length-delimited frame.
#[async_trait]
pub trait Transport: Send + 'static {
    /// Send a length-delimited frame.
    async fn send(&mut self, payload: &[u8]) -> Result<()>;
    /// Receive a length-delimited frame.
    async fn recv(&mut self) -> Result<Vec<u8>>;
    /// Human-readable transport name for logging.
    fn name(&self) -> &str;
}

// ---------------------------------------------------------------------------
// Framed read/write helpers (for generic AsyncRead/AsyncWrite)
// ---------------------------------------------------------------------------

/// Framed writer: serialize + length-prefix a message.
pub async fn write_message<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    msg: &impl serde::Serialize,
) -> Result<()> {
    let payload = serde_json::to_vec(msg)?;
    let len = (payload.len() as u32).to_be_bytes();
    writer.write_all(&len).await?;
    writer.write_all(&payload).await?;
    writer.flush().await?;
    Ok(())
}

/// Framed reader: read length-prefix + JSON payload.
pub async fn read_message<R: AsyncReadExt + Unpin>(reader: &mut R) -> Result<Message> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;

    if len > 16 * 1024 * 1024 {
        anyhow::bail!("message too large: {} bytes", len);
    }

    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload).await?;
    let msg: Message = serde_json::from_slice(&payload)?;
    Ok(msg)
}

// ---------------------------------------------------------------------------
// RPC Client: sends requests, receives responses
// ---------------------------------------------------------------------------

type PendingMap = Arc<Mutex<HashMap<u64, oneshot::Sender<Response>>>>;

/// Multiplexed RPC client over a framed stream.
pub struct RpcClient {
    tx: mpsc::Sender<Vec<u8>>,
    pending: PendingMap,
    notifications: mpsc::Sender<Notification>,
}

impl RpcClient {
    /// Spawn a client over a split read/write stream.
    /// Returns (client, notification_receiver).
    pub fn spawn<R, W>(reader: R, writer: W) -> (Self, mpsc::Receiver<Notification>)
    where
        R: AsyncReadExt + Unpin + Send + 'static,
        W: AsyncWriteExt + Unpin + Send + 'static,
    {
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let (write_tx, mut write_rx) = mpsc::channel::<Vec<u8>>(64);
        let (notif_tx, notif_rx) = mpsc::channel::<Notification>(64);

        // Writer task
        tokio::spawn(async move {
            let mut writer = writer;
            while let Some(payload) = write_rx.recv().await {
                let len = (payload.len() as u32).to_be_bytes();
                if writer.write_all(&len).await.is_err() {
                    break;
                }
                if writer.write_all(&payload).await.is_err() {
                    break;
                }
                let _ = writer.flush().await;
            }
        });

        // Reader task
        let pending_clone = pending.clone();
        let notif_tx_clone = notif_tx.clone();
        tokio::spawn(async move {
            let mut reader = reader;
            loop {
                match read_message(&mut reader).await {
                    Ok(Message::Response(resp)) => {
                        let mut map = pending_clone.lock().await;
                        if let Some(tx) = map.remove(&resp.id) {
                            let _ = tx.send(resp);
                        }
                    }
                    Ok(Message::Notification(notif)) => {
                        let _ = notif_tx_clone.send(notif).await;
                    }
                    Ok(Message::Request(_)) => {
                        // Client shouldn't receive requests; ignore
                    }
                    Err(_) => break,
                }
            }
        });

        let client = Self {
            tx: write_tx,
            pending,
            notifications: notif_tx,
        };
        (client, notif_rx)
    }

    /// Spawn a client over mpsc channels instead of raw streams.
    ///
    /// `incoming_rx` delivers raw frames from the active transport.
    /// `outgoing_tx` sends raw frames to the active transport.
    ///
    /// This allows a TransportManager to bridge between the active transport
    /// and the RPC client, swapping transports without recreating the client.
    pub fn spawn_from_channels(
        incoming_rx: mpsc::Receiver<Vec<u8>>,
        outgoing_tx: mpsc::Sender<Vec<u8>>,
    ) -> (Self, mpsc::Receiver<Notification>) {
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let (write_tx, mut write_rx) = mpsc::channel::<Vec<u8>>(64);
        let (notif_tx, notif_rx) = mpsc::channel::<Notification>(64);

        // Writer task: forward serialized payloads to the outgoing channel
        tokio::spawn(async move {
            while let Some(payload) = write_rx.recv().await {
                if outgoing_tx.send(payload).await.is_err() {
                    break;
                }
            }
        });

        // Reader task: parse incoming frames into responses/notifications
        let pending_clone = pending.clone();
        let notif_tx_clone = notif_tx.clone();
        tokio::spawn(async move {
            let mut incoming_rx = incoming_rx;
            while let Some(payload) = incoming_rx.recv().await {
                let msg: Message = match serde_json::from_slice(&payload) {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                match msg {
                    Message::Response(resp) => {
                        let mut map = pending_clone.lock().await;
                        if let Some(tx) = map.remove(&resp.id) {
                            let _ = tx.send(resp);
                        }
                    }
                    Message::Notification(notif) => {
                        let _ = notif_tx_clone.send(notif).await;
                    }
                    Message::Request(_) => {
                        // Client shouldn't receive requests; ignore
                    }
                }
            }
        });

        let client = Self {
            tx: write_tx,
            pending,
            notifications: notif_tx,
        };
        (client, notif_rx)
    }

    /// Send a request and wait for the response.
    pub async fn call(
        &self,
        method: impl Into<String>,
        params: serde_json::Value,
    ) -> Result<Response> {
        let req = Request::new(method, params);
        let id = req.id;

        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        let payload = serde_json::to_vec(&req)?;
        self.tx
            .send(payload)
            .await
            .map_err(|_| anyhow::anyhow!("transport closed"))?;

        rx.await
            .map_err(|_| anyhow::anyhow!("response channel dropped"))
    }

    /// Send a notification (no response expected).
    pub async fn notify(
        &self,
        method: impl Into<String>,
        params: serde_json::Value,
    ) -> Result<()> {
        let notif = Notification::new(method, params);
        let payload = serde_json::to_vec(&notif)?;
        self.tx
            .send(payload)
            .await
            .map_err(|_| anyhow::anyhow!("transport closed"))?;
        Ok(())
    }

    /// Access the notification sender (for dropping/closing).
    pub fn notification_sender(&self) -> &mpsc::Sender<Notification> {
        &self.notifications
    }
}

// ---------------------------------------------------------------------------
// RPC Server: dispatches requests to handlers
// ---------------------------------------------------------------------------

/// Handler function type: takes method + params, returns result or error.
pub type HandlerFn =
    Box<dyn Fn(serde_json::Value) -> futures::future::BoxFuture<'static, Result<serde_json::Value>> + Send + Sync>;

/// Simple RPC server that dispatches to registered handlers.
pub struct RpcServer {
    handlers: HashMap<String, HandlerFn>,
}

impl RpcServer {
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }

    pub fn register(
        &mut self,
        method: impl Into<String>,
        handler: impl Fn(serde_json::Value) -> futures::future::BoxFuture<'static, Result<serde_json::Value>>
            + Send
            + Sync
            + 'static,
    ) {
        self.handlers.insert(method.into(), Box::new(handler));
    }

    /// Serve a single connection. Reads requests, dispatches, writes responses.
    pub async fn serve<R, W>(&self, mut reader: R, mut writer: W) -> Result<()>
    where
        R: AsyncReadExt + Unpin + Send,
        W: AsyncWriteExt + Unpin + Send,
    {
        loop {
            let msg = match read_message(&mut reader).await {
                Ok(msg) => msg,
                Err(_) => break,
            };

            match msg {
                Message::Request(req) => {
                    let resp = if let Some(handler) = self.handlers.get(&req.method) {
                        match handler(req.params).await {
                            Ok(result) => Response::ok(req.id, result),
                            Err(e) => Response::err(req.id, INTERNAL_ERROR, e.to_string()),
                        }
                    } else {
                        Response::err(
                            req.id,
                            crate::protocol::METHOD_NOT_FOUND,
                            format!("unknown method: {}", req.method),
                        )
                    };
                    write_message(&mut writer, &resp).await?;
                }
                Message::Notification(_) => {
                    // Server can handle notifications if needed
                }
                Message::Response(_) => {
                    // Ignore stray responses
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    #[tokio::test]
    async fn framed_roundtrip() {
        let (mut client, mut server) = duplex(1024);

        let req = Request::new("test", serde_json::json!({"key": "value"}));
        write_message(&mut client, &req).await.unwrap();

        let msg = read_message(&mut server).await.unwrap();
        match msg {
            Message::Request(r) => {
                assert_eq!(r.method, "test");
            }
            _ => panic!("expected request"),
        }
    }

    #[tokio::test]
    async fn client_server_call() {
        let (client_stream, server_stream) = duplex(4096);
        let (cr, cw) = tokio::io::split(client_stream);
        let (sr, sw) = tokio::io::split(server_stream);

        let mut server = RpcServer::new();
        server.register("echo", |params| {
            Box::pin(async move { Ok(params) })
        });

        let server_handle = tokio::spawn(async move {
            let _ = server.serve(sr, sw).await;
        });

        let (client, _notifs) = RpcClient::spawn(cr, cw);
        let resp = client
            .call("echo", serde_json::json!({"hello": "world"}))
            .await
            .unwrap();

        assert!(resp.error.is_none());
        assert_eq!(
            resp.result.unwrap(),
            serde_json::json!({"hello": "world"})
        );

        server_handle.abort();
    }

    #[tokio::test]
    async fn client_method_not_found() {
        let (client_stream, server_stream) = duplex(4096);
        let (cr, cw) = tokio::io::split(client_stream);
        let (sr, sw) = tokio::io::split(server_stream);

        let server = RpcServer::new();
        let server_handle = tokio::spawn(async move {
            let _ = server.serve(sr, sw).await;
        });

        let (client, _notifs) = RpcClient::spawn(cr, cw);
        let resp = client.call("nonexistent", serde_json::json!({})).await.unwrap();
        assert!(resp.error.is_some());
        assert_eq!(
            resp.error.unwrap().code,
            crate::protocol::METHOD_NOT_FOUND
        );

        server_handle.abort();
    }

    #[tokio::test]
    async fn spawn_from_channels_roundtrip() {
        // Set up channels to simulate a transport
        let (incoming_tx, incoming_rx) = mpsc::channel::<Vec<u8>>(64);
        let (outgoing_tx, mut outgoing_rx) = mpsc::channel::<Vec<u8>>(64);

        let (client, _notifs) = RpcClient::spawn_from_channels(incoming_rx, outgoing_tx);

        // Send a request via the client
        let call_handle = tokio::spawn(async move {
            client.call("test", serde_json::json!({"foo": "bar"})).await
        });

        // Read the outgoing payload (the serialized Request)
        let outgoing_payload = outgoing_rx.recv().await.unwrap();
        let msg: Message = serde_json::from_slice(&outgoing_payload).unwrap();
        let req_id = match msg {
            Message::Request(r) => {
                assert_eq!(r.method, "test");
                r.id
            }
            _ => panic!("expected request"),
        };

        // Simulate sending a response back
        let resp = Response::ok(req_id, serde_json::json!({"result": "ok"}));
        let resp_payload = serde_json::to_vec(&resp).unwrap();
        incoming_tx.send(resp_payload).await.unwrap();

        // The call should resolve with the response
        let result = call_handle.await.unwrap().unwrap();
        assert!(result.error.is_none());
        assert_eq!(result.result.unwrap(), serde_json::json!({"result": "ok"}));
    }

}
