//! Domain-alias adapter: an ephemeral in-app SOCKS5 proxy the webview is pointed
//! at via `proxyConfigurations`. Each host gets a per-host alias hostname
//! `<label>.zedra.test`; because it is non-loopback, WKWebView routes it through
//! the proxy (a real device bypasses the proxy for all loopback — see
//! `docs/WEB_TUNNEL_MODES.md`). The proxy decodes the label back to the host's
//! endpoint id and forwards over that host's `WebConnect`. Opt-in fallback used
//! only when exact-port can't bind.

use std::collections::HashMap;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::{Mutex, OnceLock};

use iroh::PublicKey;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::OnceCell;

use super::bridge;

const ALIAS_SUFFIX: &str = ".zedra.test";

struct State {
    proxy_port: OnceCell<u16>,
    labels: Mutex<HashMap<String, PublicKey>>,
}

fn state() -> &'static State {
    static STATE: OnceLock<State> = OnceLock::new();
    STATE.get_or_init(|| State {
        proxy_port: OnceCell::new(),
        labels: Mutex::new(HashMap::new()),
    })
}

/// The per-host alias hostname (`<label>.zedra.test`), registering the label so
/// the proxy can route its CONNECTs back to `endpoint_id`.
pub(super) fn alias_host(endpoint_id: &PublicKey) -> String {
    let label: String = endpoint_id.as_bytes()[..8]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    state()
        .labels
        .lock()
        .unwrap()
        .insert(label.clone(), *endpoint_id);
    format!("{label}{ALIAS_SUFFIX}")
}

/// Bind the shared SOCKS proxy on first use; returns its ephemeral loopback port.
pub(super) async fn ensure_proxy() -> Result<u16, String> {
    state()
        .proxy_port
        .get_or_try_init(|| async {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
                .await
                .map_err(|e| format!("failed to bind SOCKS proxy: {e}"))?;
            let port = listener.local_addr().map_err(|e| e.to_string())?.port();
            spawn_accept_loop(listener);
            tracing::info!("[debug:web-tunnel] alias SOCKS proxy on 127.0.0.1:{port}");
            Ok::<_, String>(port)
        })
        .await
        .copied()
}

fn spawn_accept_loop(listener: TcpListener) {
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let _ = handle_socks(stream).await;
            });
        }
    });
}

fn reply(status: u8) -> [u8; 10] {
    [0x05, status, 0x00, 0x01, 0, 0, 0, 0, 0, 0]
}

async fn handle_socks(mut stream: TcpStream) -> Result<(), String> {
    let mut greeting = [0u8; 2];
    stream
        .read_exact(&mut greeting)
        .await
        .map_err(|e| e.to_string())?;
    if greeting[0] != 0x05 {
        return Err("not a SOCKS5 client".to_string());
    }
    let mut methods = vec![0u8; greeting[1] as usize];
    stream
        .read_exact(&mut methods)
        .await
        .map_err(|e| e.to_string())?;
    stream
        .write_all(&[0x05, 0x00])
        .await
        .map_err(|e| e.to_string())?;

    let mut request = [0u8; 4];
    stream
        .read_exact(&mut request)
        .await
        .map_err(|e| e.to_string())?;
    if request[1] != 0x01 {
        stream.write_all(&reply(0x07)).await.ok();
        return Err("only SOCKS CONNECT is supported".to_string());
    }
    let host = match request[3] {
        0x01 => {
            let mut a = [0u8; 4];
            stream.read_exact(&mut a).await.map_err(|e| e.to_string())?;
            Ipv4Addr::from(a).to_string()
        }
        0x03 => {
            let mut len = [0u8; 1];
            stream
                .read_exact(&mut len)
                .await
                .map_err(|e| e.to_string())?;
            let mut domain = vec![0u8; len[0] as usize];
            stream
                .read_exact(&mut domain)
                .await
                .map_err(|e| e.to_string())?;
            String::from_utf8_lossy(&domain).into_owned()
        }
        0x04 => {
            let mut a = [0u8; 16];
            stream.read_exact(&mut a).await.map_err(|e| e.to_string())?;
            Ipv6Addr::from(a).to_string()
        }
        _ => {
            stream.write_all(&reply(0x08)).await.ok();
            return Err("unsupported SOCKS address type".to_string());
        }
    };
    let mut port_bytes = [0u8; 2];
    stream
        .read_exact(&mut port_bytes)
        .await
        .map_err(|e| e.to_string())?;
    let port = u16::from_be_bytes(port_bytes);

    match endpoint_for_host(&host) {
        Some(endpoint_id) => forward_via_session(stream, endpoint_id, port).await,
        None => forward_direct(stream, host, port).await,
    }
}

/// Resolve `<label>.zedra.test` back to the owning host endpoint id.
fn endpoint_for_host(host: &str) -> Option<PublicKey> {
    let label = host.strip_suffix(ALIAS_SUFFIX)?;
    state().labels.lock().unwrap().get(label).copied()
}

async fn forward_via_session(
    mut stream: TcpStream,
    endpoint_id: PublicKey,
    port: u16,
) -> Result<(), String> {
    let Some(session) = super::session_for(&endpoint_id) else {
        stream.write_all(&reply(0x05)).await.ok();
        return Err("no session for alias host".to_string());
    };
    let (tx, rx, initial) = match bridge::connect(&session, port).await {
        Ok(parts) => parts,
        Err(error) => {
            stream.write_all(&reply(0x05)).await.ok();
            return Err(error);
        }
    };
    stream
        .write_all(&reply(0x00))
        .await
        .map_err(|e| e.to_string())?;
    bridge::pump(stream, tx, rx, initial, |_| {}).await;
    Ok(())
}

/// Non-alias hosts (an external link followed in the webview) dial directly —
/// the session only ever forwards a host's loopback.
async fn forward_direct(mut stream: TcpStream, host: String, port: u16) -> Result<(), String> {
    let mut upstream = match TcpStream::connect((host.as_str(), port)).await {
        Ok(upstream) => upstream,
        Err(error) => {
            stream.write_all(&reply(0x05)).await.ok();
            return Err(format!("direct dial {host}:{port} failed: {error}"));
        }
    };
    stream
        .write_all(&reply(0x00))
        .await
        .map_err(|e| e.to_string())?;
    tokio::io::copy_bidirectional(&mut stream, &mut upstream)
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}
