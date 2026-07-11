//! Shared byte bridge: pumps a local `TcpStream` to/from a host `WebConnect`
//! stream. Both adapters resolve which host session to forward to, then funnel
//! the accepted connection through here. `on_host_data` lets the exact-port
//! adapter sniff response bytes for companion ports; the alias adapter passes a
//! no-op.

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::net::tcp::OwnedWriteHalf;
use zedra_rpc::proto::{WEB_TUNNEL_MAX_CHUNK_BYTES, WebConnectReq, WebTunnelInput, WebTunnelOutput};
use zedra_session::SessionHandle;

type Tx = irpc::channel::mpsc::Sender<WebTunnelInput>;
type Rx = irpc::channel::mpsc::Receiver<WebTunnelOutput>;

/// Open a `WebConnect` to the host's loopback `port` and wait for the host to
/// confirm. Returns the paired channels plus the first output frame, or an
/// error string if the host refused or never connected.
pub(super) async fn connect(
    session: &SessionHandle,
    port: u16,
) -> Result<(Tx, Rx, Option<WebTunnelOutput>), String> {
    let (tx, mut rx) = session
        .web_connect(WebConnectReq {
            host: "localhost".to_string(),
            port,
        })
        .await
        .map_err(|error| format!("web tunnel RPC failed: {error}"))?;
    let initial = match rx.recv().await {
        Ok(Some(output)) if output.connected && output.error.is_none() => Some(output),
        Ok(Some(output)) => {
            return Err(output
                .error
                .unwrap_or_else(|| "host rejected web tunnel connection".to_string()));
        }
        Ok(None) => return Err("host closed web tunnel before connecting".to_string()),
        Err(error) => return Err(format!("web tunnel handshake failed: {error:?}")),
    };
    Ok((tx, rx, initial))
}

/// Bridge `stream` to the paired `WebConnect` channels until either side closes.
pub(super) async fn pump(
    stream: TcpStream,
    tx: Tx,
    rx: Rx,
    initial: Option<WebTunnelOutput>,
    on_host_data: impl FnMut(&[u8]) + Send + 'static,
) {
    let (mut reader, writer) = stream.into_split();
    let app_to_host = tokio::spawn(async move {
        let mut buffer = vec![0; WEB_TUNNEL_MAX_CHUNK_BYTES];
        loop {
            match reader.read(&mut buffer).await {
                Ok(0) => {
                    let _ = tx.send(close_frame()).await;
                    break;
                }
                Ok(n) => {
                    let chunk = WebTunnelInput {
                        data: buffer[..n].to_vec(),
                        close: false,
                    };
                    if tx.send(chunk).await.is_err() {
                        break;
                    }
                }
                Err(_) => {
                    let _ = tx.send(close_frame()).await;
                    break;
                }
            }
        }
    });
    let host_to_app = tokio::spawn(async move {
        write_outputs(writer, rx, initial, on_host_data).await;
    });
    let _ = tokio::join!(app_to_host, host_to_app);
}

fn close_frame() -> WebTunnelInput {
    WebTunnelInput {
        data: vec![],
        close: true,
    }
}

async fn write_outputs(
    mut writer: OwnedWriteHalf,
    mut rx: Rx,
    initial: Option<WebTunnelOutput>,
    mut on_host_data: impl FnMut(&[u8]) + Send + 'static,
) {
    if let Some(output) = initial {
        if !write_one(&mut writer, output, &mut on_host_data).await {
            return;
        }
    }
    while let Ok(Some(output)) = rx.recv().await {
        if !write_one(&mut writer, output, &mut on_host_data).await {
            break;
        }
    }
    let _ = writer.shutdown().await;
}

async fn write_one(
    writer: &mut OwnedWriteHalf,
    output: WebTunnelOutput,
    on_host_data: &mut (impl FnMut(&[u8]) + Send + 'static),
) -> bool {
    if output.error.is_some() {
        return false;
    }
    if !output.data.is_empty() {
        on_host_data(&output.data);
        if writer.write_all(&output.data).await.is_err() {
            return false;
        }
    }
    !output.close
}
