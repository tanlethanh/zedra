// TCP proxy handler: connects to a local port on the host and pipes data
// bidirectionally with an irpc TcpTunnel stream.
//
// One call to `handle_tcp_tunnel` corresponds to one TCP connection accepted
// by the mobile proxy server. Multiple concurrent tunnels to the same port are
// supported (each is an independent stream).

use anyhow::Result;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use zedra_rpc::proto::TcpData;

/// Handle a single TcpTunnel RPC stream.
///
/// Connects to `127.0.0.1:port` on the host, then pipes data bidirectionally:
///   - TCP read → `TcpData` chunks sent to client via `tx`
///   - `TcpData` from client via `rx` → TCP write
///
/// If the TCP connection is refused, sends `TcpData { data: [], closed: true }`
/// and returns immediately.
pub async fn handle_tcp_tunnel(
    port: u16,
    mut rx: irpc::channel::mpsc::Receiver<TcpData>,
    tx: irpc::channel::mpsc::Sender<TcpData>,
) -> Result<()> {
    let stream = match TcpStream::connect(("127.0.0.1", port)).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("TcpTunnel: connection refused to 127.0.0.1:{}: {}", port, e);
            let _ = tx
                .send(TcpData {
                    data: vec![],
                    closed: true,
                })
                .await;
            return Ok(());
        }
    };

    tracing::info!("TcpTunnel: connected to 127.0.0.1:{}", port);
    let (mut tcp_read, mut tcp_write) = stream.into_split();

    // Bridge channel: TCP reader task → output task → irpc tx.
    // Keeps the read task and output task decoupled so slow relay sends don't
    // block the TCP reader (same pattern as TermAttach output coalescing).
    let (bridge_tx, mut bridge_rx) = tokio::sync::mpsc::channel::<TcpData>(64);

    let read_task = tokio::spawn(async move {
        let mut buf = vec![0u8; 65536];
        loop {
            match tcp_read.read(&mut buf).await {
                Ok(0) => {
                    let _ = bridge_tx
                        .send(TcpData {
                            data: vec![],
                            closed: true,
                        })
                        .await;
                    break;
                }
                Ok(n) => {
                    if bridge_tx
                        .send(TcpData {
                            data: buf[..n].to_vec(),
                            closed: false,
                        })
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Err(e) => {
                    tracing::debug!("TcpTunnel: TCP read error: {}", e);
                    let _ = bridge_tx
                        .send(TcpData {
                            data: vec![],
                            closed: true,
                        })
                        .await;
                    break;
                }
            }
        }
    });

    let output_task = tokio::spawn(async move {
        while let Some(chunk) = bridge_rx.recv().await {
            let done = chunk.closed;
            if tx.send(chunk).await.is_err() {
                break;
            }
            if done {
                break;
            }
        }
    });

    // Write loop: irpc rx → TCP
    loop {
        match rx.recv().await {
            Ok(Some(chunk)) => {
                if chunk.closed {
                    break;
                }
                if !chunk.data.is_empty() {
                    if tcp_write.write_all(&chunk.data).await.is_err() {
                        break;
                    }
                }
            }
            Ok(None) | Err(_) => break,
        }
    }

    read_task.abort();
    output_task.abort();
    Ok(())
}
