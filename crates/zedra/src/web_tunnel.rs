use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Arc, Mutex, OnceLock};

use reqwest::Url;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, tcp::OwnedWriteHalf};
use tokio::sync::RwLock;
use zedra_rpc::proto::{
    WEB_TUNNEL_MAX_CHUNK_BYTES, WebConnectReq, WebTunnelInput, WebTunnelOutput,
};
use zedra_session::SessionHandle;

use crate::platform_bridge::{self, NativeNotificationKind, NativeNotificationOptions};

static WEB_PROXY: OnceLock<Mutex<Option<Arc<SocksProxy>>>> = OnceLock::new();

#[derive(Clone)]
struct TargetOrigin {
    host: String,
    port: u16,
}

struct SocksProxy {
    proxy_url: String,
    session_handle: Arc<RwLock<SessionHandle>>,
}

#[derive(Clone)]
struct SocksDestination {
    host: String,
    port: u16,
}

pub fn is_host_local_http_url(url: &str) -> bool {
    parse_target_origin(url).is_ok()
}

pub fn open_host_local_url(session_handle: SessionHandle, url: String) -> Result<(), String> {
    let target = parse_target_origin(&url)?;
    let title = format!("{}:{}", target.host, target.port);

    zedra_session::session_runtime().spawn(async move {
        match ensure_proxy(session_handle).await {
            Ok(proxy_url) => {
                platform_bridge::bridge().open_webview(&url, &title, Some(&proxy_url));
            }
            Err(error) => {
                platform_bridge::show_native_notification(
                    NativeNotificationOptions::new("Web tunnel failed")
                        .message(error)
                        .kind(NativeNotificationKind::Error),
                );
            }
        }
    });

    Ok(())
}

fn proxy_slot() -> &'static Mutex<Option<Arc<SocksProxy>>> {
    WEB_PROXY.get_or_init(|| Mutex::new(None))
}

fn parse_target_origin(url: &str) -> Result<TargetOrigin, String> {
    let url = Url::parse(url).map_err(|error| format!("Invalid URL: {error}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("Only http and https localhost URLs can use the web tunnel".to_string());
    }
    let host = url
        .host_str()
        .ok_or_else(|| "URL is missing a host".to_string())?
        .to_string();
    if !is_loopback_host(&host) {
        return Err("Only localhost URLs can use the web tunnel".to_string());
    }
    let port = url
        .port_or_known_default()
        .ok_or_else(|| "URL is missing a port".to_string())?;
    Ok(TargetOrigin { host, port })
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .map(|addr| addr.is_loopback())
            .unwrap_or(false)
}

async fn ensure_proxy(session_handle: SessionHandle) -> Result<String, String> {
    if let Some(proxy) = proxy_slot()
        .lock()
        .ok()
        .and_then(|entry| entry.as_ref().cloned())
    {
        *proxy.session_handle.write().await = session_handle;
        return Ok(proxy.proxy_url.clone());
    }

    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|error| format!("Failed to bind local web proxy: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("Failed to read local web proxy port: {error}"))?
        .port();
    let session_handle = Arc::new(RwLock::new(session_handle));
    let proxy = Arc::new(SocksProxy {
        proxy_url: format!("socks://127.0.0.1:{port}"),
        session_handle: session_handle.clone(),
    });

    spawn_accept_loop(listener, session_handle);

    if let Ok(mut entry) = proxy_slot().lock() {
        *entry = Some(proxy.clone());
    }

    Ok(proxy.proxy_url.clone())
}

fn spawn_accept_loop(listener: TcpListener, session_handle: Arc<RwLock<SessionHandle>>) {
    zedra_session::session_runtime().spawn(async move {
        loop {
            let (stream, _) = match listener.accept().await {
                Ok(accepted) => accepted,
                Err(error) => {
                    tracing::warn!("web tunnel: SOCKS accept failed: {error}");
                    break;
                }
            };
            let session_handle = session_handle.clone();
            zedra_session::session_runtime().spawn(async move {
                if let Err(error) = handle_socks_connection(stream, session_handle).await {
                    tracing::debug!("web tunnel: SOCKS connection closed: {error}");
                }
            });
        }
    });
}

async fn handle_socks_connection(
    mut stream: TcpStream,
    session_handle: Arc<RwLock<SessionHandle>>,
) -> Result<(), String> {
    negotiate_socks_auth(&mut stream).await?;
    let destination = read_socks_destination(&mut stream).await?;
    if !is_loopback_host(&destination.host) {
        send_socks_reply(&mut stream, SOCKS_REPLY_CONNECTION_NOT_ALLOWED).await?;
        return Err(format!(
            "rejected non-loopback destination {}:{}",
            destination.host, destination.port
        ));
    }

    let session_handle = session_handle.read().await.clone();
    let (tx, mut rx) = session_handle
        .web_connect(WebConnectReq {
            host: destination.host.clone(),
            port: destination.port,
        })
        .await
        .map_err(|error| format!("Web tunnel RPC failed: {error}"))?;

    let initial = match rx.recv().await {
        Ok(Some(output)) if output.connected && output.error.is_none() => Some(output),
        Ok(Some(output)) => {
            send_socks_reply(&mut stream, SOCKS_REPLY_HOST_UNREACHABLE).await?;
            return Err(output
                .error
                .unwrap_or_else(|| "host rejected web tunnel connection".to_string()));
        }
        Ok(None) => {
            send_socks_reply(&mut stream, SOCKS_REPLY_HOST_UNREACHABLE).await?;
            return Err("host closed web tunnel before connection completed".to_string());
        }
        Err(error) => {
            send_socks_reply(&mut stream, SOCKS_REPLY_HOST_UNREACHABLE).await?;
            return Err(format!("web tunnel connection handshake failed: {error:?}"));
        }
    };

    send_socks_reply(&mut stream, SOCKS_REPLY_SUCCEEDED).await?;
    tracing::debug!(
        host = %destination.host,
        port = destination.port,
        "web tunnel SOCKS stream connected"
    );

    bridge_stream(stream, tx, rx, initial).await
}

const SOCKS_VERSION: u8 = 0x05;
const SOCKS_AUTH_NONE: u8 = 0x00;
const SOCKS_AUTH_NO_ACCEPTABLE: u8 = 0xff;
const SOCKS_CMD_CONNECT: u8 = 0x01;
const SOCKS_ATYP_IPV4: u8 = 0x01;
const SOCKS_ATYP_DOMAIN: u8 = 0x03;
const SOCKS_ATYP_IPV6: u8 = 0x04;
const SOCKS_REPLY_SUCCEEDED: u8 = 0x00;
const SOCKS_REPLY_GENERAL_FAILURE: u8 = 0x01;
const SOCKS_REPLY_CONNECTION_NOT_ALLOWED: u8 = 0x02;
const SOCKS_REPLY_COMMAND_NOT_SUPPORTED: u8 = 0x07;
const SOCKS_REPLY_ADDRESS_TYPE_NOT_SUPPORTED: u8 = 0x08;
const SOCKS_REPLY_HOST_UNREACHABLE: u8 = 0x04;

async fn negotiate_socks_auth(stream: &mut TcpStream) -> Result<(), String> {
    let mut header = [0; 2];
    stream
        .read_exact(&mut header)
        .await
        .map_err(|error| format!("failed to read SOCKS greeting: {error}"))?;
    if header[0] != SOCKS_VERSION {
        return Err("unsupported SOCKS version".to_string());
    }

    let mut methods = vec![0; header[1] as usize];
    stream
        .read_exact(&mut methods)
        .await
        .map_err(|error| format!("failed to read SOCKS auth methods: {error}"))?;
    if !methods.contains(&SOCKS_AUTH_NONE) {
        let _ = stream
            .write_all(&[SOCKS_VERSION, SOCKS_AUTH_NO_ACCEPTABLE])
            .await;
        return Err("SOCKS client did not offer no-auth mode".to_string());
    }

    stream
        .write_all(&[SOCKS_VERSION, SOCKS_AUTH_NONE])
        .await
        .map_err(|error| format!("failed to send SOCKS auth selection: {error}"))
}

async fn read_socks_destination(stream: &mut TcpStream) -> Result<SocksDestination, String> {
    let mut header = [0; 4];
    stream
        .read_exact(&mut header)
        .await
        .map_err(|error| format!("failed to read SOCKS request: {error}"))?;
    if header[0] != SOCKS_VERSION {
        send_socks_reply(stream, SOCKS_REPLY_GENERAL_FAILURE).await?;
        return Err("unsupported SOCKS request version".to_string());
    }
    if header[1] != SOCKS_CMD_CONNECT {
        send_socks_reply(stream, SOCKS_REPLY_COMMAND_NOT_SUPPORTED).await?;
        return Err("only SOCKS CONNECT requests are supported".to_string());
    }

    let host = match header[3] {
        SOCKS_ATYP_IPV4 => {
            let mut bytes = [0; 4];
            stream
                .read_exact(&mut bytes)
                .await
                .map_err(|error| format!("failed to read SOCKS IPv4 address: {error}"))?;
            IpAddr::from(bytes).to_string()
        }
        SOCKS_ATYP_IPV6 => {
            let mut bytes = [0; 16];
            stream
                .read_exact(&mut bytes)
                .await
                .map_err(|error| format!("failed to read SOCKS IPv6 address: {error}"))?;
            IpAddr::from(bytes).to_string()
        }
        SOCKS_ATYP_DOMAIN => {
            let mut len = [0; 1];
            stream
                .read_exact(&mut len)
                .await
                .map_err(|error| format!("failed to read SOCKS domain length: {error}"))?;
            let mut bytes = vec![0; len[0] as usize];
            stream
                .read_exact(&mut bytes)
                .await
                .map_err(|error| format!("failed to read SOCKS domain: {error}"))?;
            String::from_utf8(bytes).map_err(|error| format!("invalid SOCKS domain: {error}"))?
        }
        _ => {
            send_socks_reply(stream, SOCKS_REPLY_ADDRESS_TYPE_NOT_SUPPORTED).await?;
            return Err("unsupported SOCKS address type".to_string());
        }
    };

    let mut port = [0; 2];
    stream
        .read_exact(&mut port)
        .await
        .map_err(|error| format!("failed to read SOCKS port: {error}"))?;
    let port = u16::from_be_bytes(port);
    if port == 0 {
        send_socks_reply(stream, SOCKS_REPLY_GENERAL_FAILURE).await?;
        return Err("SOCKS destination port is invalid".to_string());
    }

    Ok(SocksDestination { host, port })
}

async fn send_socks_reply(stream: &mut TcpStream, reply: u8) -> Result<(), String> {
    stream
        .write_all(&[
            SOCKS_VERSION,
            reply,
            0x00,
            SOCKS_ATYP_IPV4,
            127,
            0,
            0,
            1,
            0,
            0,
        ])
        .await
        .map_err(|error| format!("failed to send SOCKS reply: {error}"))
}

async fn bridge_stream(
    stream: TcpStream,
    tx: irpc::channel::mpsc::Sender<WebTunnelInput>,
    rx: irpc::channel::mpsc::Receiver<WebTunnelOutput>,
    initial: Option<WebTunnelOutput>,
) -> Result<(), String> {
    let (mut local_reader, local_writer) = stream.into_split();
    let app_to_host = tokio::spawn(async move {
        let mut buffer = vec![0; WEB_TUNNEL_MAX_CHUNK_BYTES];
        loop {
            match local_reader.read(&mut buffer).await {
                Ok(0) => {
                    let _ = tx
                        .send(WebTunnelInput {
                            data: vec![],
                            close: true,
                        })
                        .await;
                    break;
                }
                Ok(n) => {
                    if tx
                        .send(WebTunnelInput {
                            data: buffer[..n].to_vec(),
                            close: false,
                        })
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Err(error) => {
                    tracing::debug!(error = %error, "web tunnel local read failed");
                    let _ = tx
                        .send(WebTunnelInput {
                            data: vec![],
                            close: true,
                        })
                        .await;
                    break;
                }
            }
        }
    });

    let host_to_app = tokio::spawn(async move {
        write_host_outputs(local_writer, rx, initial).await;
    });

    let _ = tokio::join!(app_to_host, host_to_app);
    Ok(())
}

async fn write_host_outputs(
    mut local_writer: OwnedWriteHalf,
    mut rx: irpc::channel::mpsc::Receiver<WebTunnelOutput>,
    initial: Option<WebTunnelOutput>,
) {
    if let Some(output) = initial {
        if !write_host_output(&mut local_writer, output).await {
            return;
        }
    }
    loop {
        match rx.recv().await {
            Ok(Some(output)) => {
                if !write_host_output(&mut local_writer, output).await {
                    break;
                }
            }
            Ok(None) => break,
            Err(error) => {
                tracing::debug!(error = ?error, "web tunnel host output closed");
                break;
            }
        }
    }
    let _ = local_writer.shutdown().await;
}

async fn write_host_output(local_writer: &mut OwnedWriteHalf, output: WebTunnelOutput) -> bool {
    if let Some(error) = output.error {
        tracing::debug!(error = %error, "web tunnel host output error");
        return false;
    }
    if !output.data.is_empty() && local_writer.write_all(&output.data).await.is_err() {
        return false;
    }
    !output.close
}
