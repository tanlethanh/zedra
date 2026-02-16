// WebSocket Relay Transport
//
// Implements the Transport trait over a persistent WebSocket connection
// to the relay server. Replaces HTTP polling with bidirectional streaming,
// reducing relay latency from 100-1000ms to 50-100ms.
//
// Protocol: binary frames over WebSocket. Each message is a raw byte payload
// (no base64 encoding needed, unlike the HTTP relay).

use anyhow::{Context, Result};
use async_trait::async_trait;
use futures::stream::SplitSink;
use futures::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use zedra_rpc::Transport;

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;
type WsSink = SplitSink<WsStream, Message>;

/// Transport implementation over a persistent WebSocket connection.
///
/// Messages are sent as binary WebSocket frames (no base64 overhead).
/// The connection is persistent — no polling required. Latency is
/// determined only by network RTT + relay forwarding time.
pub struct WsRelayTransport {
    send_tx: mpsc::Sender<Vec<u8>>,
    recv_rx: mpsc::Receiver<Vec<u8>>,
}

impl WsRelayTransport {
    /// Connect to a WebSocket relay endpoint and return a Transport.
    ///
    /// The `ws_url` should be in the format `wss://relay.example.com/v2/ws/:room_id`
    /// with optional query parameters for authentication.
    pub async fn connect(ws_url: &str) -> Result<Self> {
        let (ws_stream, _response) = tokio_tungstenite::connect_async(ws_url)
            .await
            .context("WebSocket connection failed")?;

        log::info!("WsRelayTransport: connected to {}", ws_url);

        let (sink, stream) = ws_stream.split();

        // Channels for bridging async WebSocket to Transport trait
        let (send_tx, send_rx) = mpsc::channel::<Vec<u8>>(64);
        let (recv_tx, recv_rx) = mpsc::channel::<Vec<u8>>(64);

        // Spawn writer task: reads from send channel, writes to WebSocket
        tokio::spawn(ws_writer(sink, send_rx));

        // Spawn reader task: reads from WebSocket, writes to recv channel
        tokio::spawn(ws_reader(stream, recv_tx));

        Ok(Self { send_tx, recv_rx })
    }
}

#[async_trait]
impl Transport for WsRelayTransport {
    async fn send(&mut self, payload: &[u8]) -> Result<()> {
        self.send_tx
            .send(payload.to_vec())
            .await
            .map_err(|_| anyhow::anyhow!("WebSocket writer closed"))?;
        Ok(())
    }

    async fn recv(&mut self) -> Result<Vec<u8>> {
        self.recv_rx
            .recv()
            .await
            .ok_or_else(|| anyhow::anyhow!("WebSocket reader closed"))
    }

    fn name(&self) -> &str {
        "relay-ws"
    }
}

/// Writer task: forwards outgoing payloads to the WebSocket as binary frames.
async fn ws_writer(mut sink: WsSink, mut rx: mpsc::Receiver<Vec<u8>>) {
    while let Some(data) = rx.recv().await {
        if let Err(e) = sink.send(Message::Binary(data.into())).await {
            log::warn!("WsRelayTransport writer: send error: {}", e);
            break;
        }
    }
    // Try to close gracefully
    let _ = sink.close().await;
    log::debug!("WsRelayTransport writer: closed");
}

/// Reader task: forwards incoming binary WebSocket frames to the recv channel.
async fn ws_reader(
    mut stream: futures::stream::SplitStream<WsStream>,
    tx: mpsc::Sender<Vec<u8>>,
) {
    while let Some(msg_result) = stream.next().await {
        match msg_result {
            Ok(Message::Binary(data)) => {
                if tx.send(data.into()).await.is_err() {
                    break; // recv channel dropped
                }
            }
            Ok(Message::Ping(_)) => {
                // Pong is sent automatically by tungstenite
            }
            Ok(Message::Close(_)) => {
                log::debug!("WsRelayTransport reader: received close frame");
                break;
            }
            Ok(_) => {
                // Ignore text frames and other message types
            }
            Err(e) => {
                log::warn!("WsRelayTransport reader: recv error: {}", e);
                break;
            }
        }
    }
    log::debug!("WsRelayTransport reader: closed");
}

/// Build a WebSocket relay URL from components.
///
/// Returns a URL like `wss://relay.example.com/v2/ws/:room_id?secret=:secret&role=:role`
pub fn build_ws_url(relay_url: &str, room_id: &str, secret: &str, role: &str) -> String {
    // Convert https:// to wss:// and http:// to ws://
    let ws_base = if relay_url.starts_with("https://") {
        relay_url.replacen("https://", "wss://", 1)
    } else if relay_url.starts_with("http://") {
        relay_url.replacen("http://", "ws://", 1)
    } else {
        relay_url.to_string()
    };

    let base = ws_base.trim_end_matches('/');
    format!(
        "{}/v2/ws/{}?secret={}&role={}",
        base, room_id, secret, role
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_ws_url_https() {
        let url = build_ws_url("https://relay.zedra.dev", "ROOM123", "secret", "host");
        assert_eq!(
            url,
            "wss://relay.zedra.dev/v2/ws/ROOM123?secret=secret&role=host"
        );
    }

    #[test]
    fn build_ws_url_http() {
        let url = build_ws_url("http://localhost:8787", "ABC", "sec", "mobile");
        assert_eq!(
            url,
            "ws://localhost:8787/v2/ws/ABC?secret=sec&role=mobile"
        );
    }

    #[test]
    fn build_ws_url_trailing_slash() {
        let url = build_ws_url("https://relay.zedra.dev/", "ROOM", "s", "host");
        assert_eq!(
            url,
            "wss://relay.zedra.dev/v2/ws/ROOM?secret=s&role=host"
        );
    }

    #[test]
    fn build_ws_url_already_wss() {
        let url = build_ws_url("wss://relay.zedra.dev", "ROOM", "s", "host");
        assert_eq!(
            url,
            "wss://relay.zedra.dev/v2/ws/ROOM?secret=s&role=host"
        );
    }
}
