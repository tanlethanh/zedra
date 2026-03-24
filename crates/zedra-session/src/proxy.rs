// Local TCP proxy server for dev-server preview.
//
// TcpProxyServer listens on 127.0.0.1:0 (OS-assigned port). For each accepted
// TCP connection it opens a TcpTunnel RPC stream on the active session, then
// pipes data bidirectionally:
//
//   WebView → TcpProxyServer loopback → iroh QUIC → host 127.0.0.1:<target_port>
//
// One TcpProxyServer instance per target port. Multiple concurrent connections
// to the same port are supported (each gets its own TcpTunnel stream).
//
// The server runs until the returned `TcpProxyServer` is dropped.

use anyhow::{Context as _, Result};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

use zedra_rpc::proto::{TcpData, TcpTunnelReq};

use crate::handle::SessionHandle;

/// A running local TCP proxy server.
///
/// Dropping this value shuts down the listener and cancels all active tunnel tasks.
pub struct TcpProxyServer {
    local_port: u16,
    _shutdown_tx: watch::Sender<()>,
}

impl TcpProxyServer {
    /// Start a proxy server that forwards TCP connections to `target_port` on the host.
    ///
    /// Returns immediately once the listener is bound. Returns `Err` if the
    /// listener cannot bind (very rare on loopback with port 0).
    pub async fn start(handle: SessionHandle, target_port: u16) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .context("failed to bind local proxy listener")?;
        let local_port = listener.local_addr()?.port();

        tracing::info!(
            "TcpProxyServer: listening on 127.0.0.1:{} → host:{}",
            local_port,
            target_port,
        );

        zedra_telemetry::send(zedra_telemetry::Event::TcpTunnelOpened(
            zedra_telemetry::TcpTunnelOpened { port: target_port },
        ));

        let (shutdown_tx, shutdown_rx) = watch::channel(());

        tokio::spawn(accept_loop(listener, handle, target_port, shutdown_rx));

        Ok(Self {
            local_port,
            _shutdown_tx: shutdown_tx,
        })
    }

    /// The loopback port the WebView should connect to.
    pub fn local_port(&self) -> u16 {
        self.local_port
    }
}

async fn accept_loop(
    listener: TcpListener,
    handle: SessionHandle,
    target_port: u16,
    mut shutdown_rx: watch::Receiver<()>,
) {
    loop {
        tokio::select! {
            res = listener.accept() => {
                match res {
                    Ok((stream, peer)) => {
                        tracing::debug!("TcpProxyServer: accepted connection from {}", peer);
                        tokio::spawn(handle_connection(stream, handle.clone(), target_port));
                    }
                    Err(e) => {
                        tracing::warn!("TcpProxyServer: accept error: {}", e);
                        break;
                    }
                }
            }
            _ = shutdown_rx.changed() => {
                tracing::info!("TcpProxyServer: shutdown signal received");
                break;
            }
        }
    }
}

async fn handle_connection(stream: TcpStream, handle: SessionHandle, target_port: u16) {
    let started = Instant::now();
    let client = match handle.client() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("TcpProxyServer: no active session for tunnel: {}", e);
            return;
        }
    };

    let (irpc_write_tx, mut irpc_read_rx) = match client
        .bidi_streaming::<TcpTunnelReq, TcpData, TcpData>(
            TcpTunnelReq { port: target_port },
            64,
            64,
        )
        .await
    {
        Ok(channels) => channels,
        Err(e) => {
            tracing::warn!("TcpProxyServer: TcpTunnel RPC failed: {}", e);
            return;
        }
    };

    let (mut tcp_read, mut tcp_write) = stream.into_split();

    // Read first message from host — if closed=true, connection was refused.
    let first = match irpc_read_rx.recv().await {
        Ok(Some(msg)) => msg,
        Ok(None) | Err(_) => {
            tracing::debug!("TcpProxyServer: irpc stream closed before first message");
            return;
        }
    };
    if first.closed {
        tracing::warn!(
            "TcpProxyServer: host refused connection to port {}",
            target_port
        );
        return;
    }
    if !first.data.is_empty() {
        if tcp_write.write_all(&first.data).await.is_err() {
            return;
        }
    }

    let bytes_out = Arc::new(AtomicU64::new(0));
    let bytes_out_write = bytes_out.clone();

    // WebView → irpc: read TCP bytes and forward to host.
    let tcp_to_irpc = tokio::spawn(async move {
        let mut buf = vec![0u8; 65536];
        loop {
            match tcp_read.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    bytes_out_write.fetch_add(n as u64, Ordering::Relaxed);
                    if irpc_write_tx
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
                Err(_) => break,
            }
        }
        let _ = irpc_write_tx
            .send(TcpData {
                data: vec![],
                closed: true,
            })
            .await;
    });

    let mut bytes_in: u64 = first.data.len() as u64;

    // irpc → WebView: forward host chunks to TCP.
    loop {
        match irpc_read_rx.recv().await {
            Ok(Some(chunk)) => {
                if chunk.closed {
                    break;
                }
                bytes_in += chunk.data.len() as u64;
                if tcp_write.write_all(&chunk.data).await.is_err() {
                    break;
                }
            }
            Ok(None) | Err(_) => break,
        }
    }

    tcp_to_irpc.abort();

    zedra_telemetry::send(zedra_telemetry::Event::TcpTunnelClosed(
        zedra_telemetry::TcpTunnelClosed {
            bytes_proxied_in: bytes_in,
            bytes_proxied_out: bytes_out.load(Ordering::Relaxed),
            duration_ms: started.elapsed().as_millis() as u64,
        },
    ));
}
