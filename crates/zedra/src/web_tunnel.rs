use std::collections::HashMap;
use std::convert::Infallible;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::{Arc, Mutex, OnceLock};

use bytes::Bytes;
use http_body_util::{BodyExt, Full, LengthLimitError, Limited};
use hyper::body::Incoming;
use hyper::header::{HeaderName, HeaderValue};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use reqwest::Url;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use zedra_rpc::proto::{WEB_TUNNEL_MAX_REQUEST_BODY_BYTES, WebFetchReq, WebHeader};
use zedra_session::SessionHandle;

use crate::platform_bridge::{self, NativeNotificationKind, NativeNotificationOptions};

static WEB_TUNNELS: OnceLock<Mutex<HashMap<u16, Arc<ProxyEntry>>>> = OnceLock::new();

#[derive(Clone)]
struct TargetOrigin {
    scheme: String,
    host: String,
    port: u16,
}

#[derive(Clone)]
struct ProxyTarget {
    session_handle: SessionHandle,
    origin: TargetOrigin,
}

struct ProxyEntry {
    local_host: String,
    local_port: u16,
    target: Arc<RwLock<ProxyTarget>>,
}

pub fn is_host_local_http_url(url: &str) -> bool {
    parse_target_origin(url).is_ok()
}

pub fn open_host_local_url(session_handle: SessionHandle, url: String) -> Result<(), String> {
    let target = parse_target_origin(&url)?;
    let title = format!("{}:{}", target.host, target.port);

    zedra_session::session_runtime().spawn(async move {
        match ensure_proxy(session_handle, target, &url).await {
            Ok(local_url) => {
                platform_bridge::bridge().open_webview(&local_url, &title);
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

fn tunnels() -> &'static Mutex<HashMap<u16, Arc<ProxyEntry>>> {
    WEB_TUNNELS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn parse_target_origin(url: &str) -> Result<TargetOrigin, String> {
    let url = Url::parse(url).map_err(|error| format!("Invalid URL: {error}"))?;
    if url.scheme() != "http" {
        return Err("Only http localhost URLs can use the web tunnel".to_string());
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
    Ok(TargetOrigin {
        scheme: url.scheme().to_string(),
        host,
        port,
    })
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .map(|addr| addr.is_loopback())
            .unwrap_or(false)
}

async fn ensure_proxy(
    session_handle: SessionHandle,
    target: TargetOrigin,
    initial_url: &str,
) -> Result<String, String> {
    let key = target.port;
    if let Some(entry) = tunnels()
        .lock()
        .ok()
        .and_then(|entries| entries.get(&key).cloned())
    {
        *entry.target.write().await = ProxyTarget {
            session_handle,
            origin: target,
        };
        return local_url_for(initial_url, &entry.local_host, entry.local_port);
    }

    let (listener, local_host, local_port) = bind_primary_listener(key).await?;
    let proxy_target = Arc::new(RwLock::new(ProxyTarget {
        session_handle,
        origin: target,
    }));
    spawn_accept_loop(listener, proxy_target.clone());

    // If the preferred port was available, also listen on IPv6 loopback. Some
    // webviews resolve `localhost` to `::1` before `127.0.0.1`.
    if local_port == key {
        if let Ok(v6_listener) = TcpListener::bind((Ipv6Addr::LOCALHOST, local_port)).await {
            spawn_accept_loop(v6_listener, proxy_target.clone());
        }
    }

    let entry = Arc::new(ProxyEntry {
        local_host: local_host.clone(),
        local_port,
        target: proxy_target,
    });
    if let Ok(mut entries) = tunnels().lock() {
        entries.insert(key, entry);
    }

    local_url_for(initial_url, &local_host, local_port)
}

async fn bind_primary_listener(preferred_port: u16) -> Result<(TcpListener, String, u16), String> {
    match TcpListener::bind((Ipv4Addr::LOCALHOST, preferred_port)).await {
        Ok(listener) => Ok((listener, "127.0.0.1".to_string(), preferred_port)),
        Err(preferred_error) => {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
                .await
                .map_err(|fallback_error| {
                    format!(
                        "Failed to bind localhost:{preferred_port}: {preferred_error}; fallback failed: {fallback_error}"
                    )
                })?;
            let port = listener
                .local_addr()
                .map_err(|error| format!("Failed to read local proxy port: {error}"))?
                .port();
            Ok((listener, "127.0.0.1".to_string(), port))
        }
    }
}

fn local_url_for(initial_url: &str, local_host: &str, local_port: u16) -> Result<String, String> {
    let mut url = Url::parse(initial_url).map_err(|error| format!("Invalid URL: {error}"))?;
    url.set_host(Some(local_host))
        .map_err(|_| "Failed to build local webview URL".to_string())?;
    url.set_port(Some(local_port))
        .map_err(|_| "Failed to set local webview port".to_string())?;
    Ok(url.to_string())
}

fn spawn_accept_loop(listener: TcpListener, target: Arc<RwLock<ProxyTarget>>) {
    zedra_session::session_runtime().spawn(async move {
        loop {
            let (stream, _) = match listener.accept().await {
                Ok(accepted) => accepted,
                Err(error) => {
                    tracing::warn!("web tunnel: accept failed: {error}");
                    break;
                }
            };
            let target = target.clone();
            zedra_session::session_runtime().spawn(async move {
                let io = TokioIo::new(stream);
                let service = service_fn(move |request| proxy_request(request, target.clone()));
                if let Err(error) = http1::Builder::new().serve_connection(io, service).await {
                    tracing::debug!("web tunnel: connection failed: {error}");
                }
            });
        }
    });
}

async fn proxy_request(
    request: Request<Incoming>,
    target: Arc<RwLock<ProxyTarget>>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let response = match proxy_request_inner(request, target).await {
        Ok(response) => response,
        Err(error) => error_response(StatusCode::BAD_GATEWAY, error),
    };
    Ok(response)
}

async fn proxy_request_inner(
    request: Request<Incoming>,
    target: Arc<RwLock<ProxyTarget>>,
) -> Result<Response<Full<Bytes>>, String> {
    let (parts, body) = request.into_parts();
    if parts
        .headers
        .get(hyper::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .is_some_and(|len| len > WEB_TUNNEL_MAX_REQUEST_BODY_BYTES)
    {
        return Ok(error_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            "Web tunnel request body is too large",
        ));
    }

    let body = match Limited::new(body, WEB_TUNNEL_MAX_REQUEST_BODY_BYTES)
        .collect()
        .await
    {
        Ok(body) => body.to_bytes(),
        Err(error) => {
            if error.downcast_ref::<LengthLimitError>().is_some() {
                return Ok(error_response(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "Web tunnel request body is too large",
                ));
            }
            return Err(format!("Failed to read webview request body: {error}"));
        }
    };
    if body.len() > WEB_TUNNEL_MAX_REQUEST_BODY_BYTES {
        return Ok(error_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            "Web tunnel request body is too large",
        ));
    }

    let target = target.read().await.clone();
    let target_url = target_url_for(
        &target.origin,
        parts.uri.path_and_query().map(|v| v.as_str()),
    );
    let result = target
        .session_handle
        .web_fetch(WebFetchReq {
            method: parts.method.as_str().to_string(),
            url: target_url,
            headers: web_headers(&parts.headers),
            body: body.to_vec(),
        })
        .await
        .map_err(|error| format!("Web tunnel RPC failed: {error}"))?;

    if let Some(error) = result.error {
        return Ok(error_response(StatusCode::BAD_GATEWAY, error));
    }

    let status = StatusCode::from_u16(result.status).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut builder = Response::builder().status(status);
    for header in result.headers {
        if is_hop_by_hop_header(&header.name) {
            continue;
        }
        let Ok(name) = HeaderName::from_bytes(header.name.as_bytes()) else {
            continue;
        };
        let Ok(value) = HeaderValue::from_str(&header.value) else {
            continue;
        };
        builder = builder.header(name, value);
    }
    builder
        .body(Full::new(Bytes::from(result.body)))
        .map_err(|error| format!("Failed to build web tunnel response: {error}"))
}

fn target_url_for(origin: &TargetOrigin, path_and_query: Option<&str>) -> String {
    let authority_host = if origin.host.contains(':') && !origin.host.starts_with('[') {
        format!("[{}]", origin.host)
    } else {
        origin.host.clone()
    };
    let path_and_query = path_and_query.unwrap_or("/");
    format!(
        "{}://{}:{}{}",
        origin.scheme, authority_host, origin.port, path_and_query
    )
}

fn web_headers(headers: &hyper::HeaderMap) -> Vec<WebHeader> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            if is_hop_by_hop_header(name.as_str()) {
                return None;
            }
            Some(WebHeader {
                name: name.as_str().to_string(),
                value: value.to_str().ok()?.to_string(),
            })
        })
        .collect()
}

fn is_hop_by_hop_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "host"
            | "content-length"
    )
}

fn error_response(status: StatusCode, message: impl Into<String>) -> Response<Full<Bytes>> {
    let message = message.into();
    Response::builder()
        .status(status)
        .header(hyper::header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Full::new(Bytes::from(message)))
        .unwrap_or_else(|_| Response::new(Full::new(Bytes::new())))
}
