// Local REST API server for zedra-host.
//
// Binds to 127.0.0.1 on an OS-assigned port. The bound address and a random
// bearer token are written to the workspace config directory so local tools
// (e.g. the `/zedra-start` Claude Code skill) can authenticate and trigger
// host actions without going through the iroh transport.
//
// Endpoints:
//   GET  /api/status              — daemon health, active sessions, and terminals
//   POST /api/qr                  — create and return a fresh one-time pairing QR
//   POST /api/qr/static           — create and return a static pairing QR
//   POST /api/terminal            — create a terminal in the active session
//   GET  /api/agents              — list supported managed agents
//   GET  /api/agents/:kind/sessions
//   POST /api/agents/:kind/resume — resume an agent session in a new terminal
//   GET  /api/agent-hooks/events — list recent hook events for CLI testing
//   POST /api/agent-hooks/:kind   — ingest a local agent hook event
//
// Auth: every request must carry  Authorization: Bearer <token>
//       where <token> is the contents of  <config_dir>/api-token

use std::sync::Arc;

use axum::extract::{Path as AxumPath, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;

use crate::agent;
use crate::agent::hook::HookContext;
use crate::agent::utils::payload_string;
use crate::metrics;
use crate::pty::SpawnOptions;
use crate::qr;
use crate::rpc_daemon::{create_terminal, DaemonState};
use crate::session_registry::{PairingSlotMode, ServerSession, SessionRegistry};
use zedra_rpc::encode_endpoint_identity;
use zedra_rpc::proto::{AgentResumeResult, HostEvent, TerminalColorScheme};
use zedra_rpc::ZedraPairingTicket;
use zedra_telemetry::Event;

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct ApiState {
    pub registry: Arc<SessionRegistry>,
    pub daemon_state: Arc<DaemonState>,
    pub endpoint: iroh::Endpoint,
    pub relay_urls: Vec<String>,
    pub token: String,
}

// ---------------------------------------------------------------------------
// Auth helper
// ---------------------------------------------------------------------------

fn verify_token(headers: &HeaderMap, token: &str) -> bool {
    headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| t == token)
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
struct StatusTerminal {
    id: String,
    session_id: String,
    session_name: Option<String>,
    title: Option<String>,
    cwd: Option<String>,
    icon_name: Option<String>,
    created_at_unix_secs: u64,
    created_at_elapsed_secs: u64,
    uptime_secs: u64,
}

#[derive(Debug, Serialize)]
struct StatusSession {
    id: String,
    name: Option<String>,
    workdir: Option<String>,
    terminal_count: usize,
    uptime_secs: u64,
    idle_secs: u64,
    is_occupied: bool,
    terminals: Vec<StatusTerminal>,
}

#[derive(Debug, Serialize)]
struct StatusResp {
    ok: bool,
    version: &'static str,
    endpoint_id: String,
    endpoint_addr: String,
    workdir: String,
    uptime_secs: u64,
    sessions: Vec<StatusSession>,
    terminals: Vec<StatusTerminal>,
}

async fn status(State(s): State<ApiState>, headers: HeaderMap) -> impl IntoResponse {
    if !verify_token(&headers, &s.token) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "unauthorized"})),
        )
            .into_response();
    }

    let endpoint_id = s.daemon_state.identity.endpoint_id().to_string();
    // Identity string (id-only) so `status` matches the deeplink/app/saved-state form.
    let endpoint_addr =
        encode_endpoint_identity(s.daemon_state.identity.endpoint_id()).unwrap_or_default();
    let workdir = s.daemon_state.workdir.to_string_lossy().to_string();
    let version = env!("CARGO_PKG_VERSION");
    let uptime_secs = s.daemon_state.started_at.elapsed().as_secs();
    let session_infos = s.registry.list_sessions().await;
    let mut sessions = Vec::with_capacity(session_infos.len());
    let mut all_terminals = Vec::new();

    for session_info in session_infos {
        let terminal_infos = match s.registry.get(&session_info.id).await {
            Some(session) => session.terminal_infos().await,
            None => Vec::new(),
        };
        let terminals: Vec<StatusTerminal> = terminal_infos
            .into_iter()
            .map(|terminal| StatusTerminal {
                id: terminal.id,
                session_id: session_info.id.clone(),
                session_name: session_info.name.clone(),
                title: terminal.title,
                cwd: terminal.cwd,
                icon_name: terminal.icon_name,
                created_at_unix_secs: terminal.created_at_unix_secs,
                created_at_elapsed_secs: terminal.created_at_elapsed_secs,
                uptime_secs: terminal.uptime_secs,
            })
            .collect();
        all_terminals.extend(terminals.clone());
        sessions.push(StatusSession {
            id: session_info.id,
            name: session_info.name,
            workdir: session_info
                .workdir
                .map(|path| path.to_string_lossy().into_owned()),
            terminal_count: session_info.terminal_count,
            uptime_secs: session_info.created_at_elapsed_secs,
            idle_secs: session_info.last_activity_elapsed_secs,
            is_occupied: session_info.is_occupied,
            terminals,
        });
    }

    Json(StatusResp {
        ok: true,
        version,
        endpoint_id,
        endpoint_addr,
        workdir,
        uptime_secs,
        sessions,
        terminals: all_terminals,
    })
    .into_response()
}

async fn create_pairing_qr_handler(
    State(s): State<ApiState>,
    headers: HeaderMap,
) -> axum::response::Response {
    create_pairing_qr_response(s, headers, PairingSlotMode::OneTime).await
}

async fn create_static_pairing_qr_handler(
    State(s): State<ApiState>,
    headers: HeaderMap,
) -> axum::response::Response {
    create_pairing_qr_response(s, headers, PairingSlotMode::Static).await
}

async fn create_pairing_qr_response(
    s: ApiState,
    headers: HeaderMap,
    mode: PairingSlotMode,
) -> axum::response::Response {
    if !verify_token(&headers, &s.token) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "unauthorized"})),
        )
            .into_response();
    }

    let Some(session) = s.registry.most_recent_session().await else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "no session available"})),
        )
            .into_response();
    };

    let ticket = ZedraPairingTicket {
        endpoint_id: s.daemon_state.identity.endpoint_id(),
        handshake_secret: rand::random(),
        session_id: session.id.clone(),
    };
    match qr::build_pairing_info(&ticket, &s.endpoint, &s.relay_urls, mode) {
        Ok(info) => {
            s.registry
                .add_pairing_slot_with_mode(&session.id, ticket.handshake_secret, mode)
                .await;
            if let Err(e) = metrics::record_qr_created(&s.daemon_state.workdir) {
                tracing::warn!("Failed to record QR metrics: {}", e);
            }
            (StatusCode::OK, Json(serde_json::json!(info))).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateTerminalReq {
    /// Session to create the terminal in. Omit to use the first active session.
    pub session_id: Option<String>,
    /// Command run on startup (e.g. "claude --resume").
    pub launch_cmd: Option<String>,
    /// Terminal appearance used for host-side color query replies.
    pub color_scheme: Option<TerminalColorScheme>,
    #[serde(default = "default_cols")]
    pub cols: u16,
    #[serde(default = "default_rows")]
    pub rows: u16,
}

fn default_cols() -> u16 {
    220
}
fn default_rows() -> u16 {
    50
}

#[derive(Serialize)]
struct CreateTerminalResp {
    id: String,
    session_id: String,
}

async fn create_terminal_handler(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Json(req): Json<CreateTerminalReq>,
) -> impl IntoResponse {
    if !verify_token(&headers, &s.token) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "unauthorized"})),
        )
            .into_response();
    }

    // Resolve session: explicit ID → most recently active session.
    let session = if let Some(id) = &req.session_id {
        s.registry.get(id).await
    } else {
        s.registry.most_recent_session().await
    };

    let Some(session) = session else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "no session available"})),
        )
            .into_response();
    };

    let workdir = session
        .workdir
        .clone()
        .or_else(|| Some(s.daemon_state.workdir.clone()));
    let launch_cmd = req.launch_cmd.clone();
    let opts = SpawnOptions {
        workdir,
        launch_cmd: launch_cmd.clone(),
        color_scheme: req.color_scheme,
        env: Vec::new(),
    };

    match create_terminal(&session, req.cols, req.rows, opts).await {
        Ok(id) => {
            let terminal_count = session.terminals.lock().await.len();
            if let Err(e) =
                metrics::record_terminal_created(&s.daemon_state.workdir, terminal_count)
            {
                tracing::warn!("Failed to record terminal metrics: {}", e);
            }
            zedra_telemetry::send(Event::HostTerminalOpen {
                has_launch_cmd: launch_cmd
                    .as_ref()
                    .map(|command| !command.is_empty())
                    .unwrap_or(false),
            });

            // Push TerminalCreated event to the subscribed client (if any).
            // Resolve identity from the launch command (no OSC 1 yet at spawn)
            // so the client shows the agent icon immediately.
            let agent_slug =
                crate::agent::detect::resolve_terminal_agent(launch_cmd.as_deref(), None)
                    .map(str::to_string);
            session
                .push_event(HostEvent::TerminalCreated {
                    id: id.clone(),
                    launch_cmd,
                    agent_slug,
                })
                .await;
            let session_id = session.id.clone();
            (
                StatusCode::OK,
                Json(serde_json::json!(CreateTerminalResp { id, session_id })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

fn unauthorized() -> axum::response::Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({"error": "unauthorized"})),
    )
        .into_response()
}

fn invalid_agent(slug: &str) -> axum::response::Response {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({"error": format!("unsupported agent: {slug}")})),
    )
        .into_response()
}

async fn agent_session_context(s: &ApiState) -> Option<(Arc<ServerSession>, std::path::PathBuf)> {
    let session = s.registry.most_recent_session().await?;
    let workdir = session
        .workdir
        .clone()
        .unwrap_or_else(|| s.daemon_state.workdir.clone());
    Some((session, workdir))
}

async fn list_agents_handler(State(s): State<ApiState>, headers: HeaderMap) -> impl IntoResponse {
    if !verify_token(&headers, &s.token) {
        return unauthorized();
    }

    let Some((session, workdir)) = agent_session_context(&s).await else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "no session available"})),
        )
            .into_response();
    };
    Json(agent::list_agents(&s.daemon_state.agent_cache, &workdir, Some(&session), true).await)
        .into_response()
}

async fn list_agent_sessions_handler(
    State(s): State<ApiState>,
    headers: HeaderMap,
    AxumPath(slug): AxumPath<String>,
) -> impl IntoResponse {
    if !verify_token(&headers, &s.token) {
        return unauthorized();
    }
    let Some(actor) = agent::actor(&slug) else {
        return invalid_agent(&slug);
    };

    let Some((session, workdir)) = agent_session_context(&s).await else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "no session available"})),
        )
            .into_response();
    };
    Json(
        agent::list_agent_sessions(
            &s.daemon_state.agent_cache,
            actor.slug(),
            &workdir,
            Some(&session),
            0,
            true,
        )
        .await,
    )
    .into_response()
}

#[derive(Debug, Deserialize)]
pub struct ResumeAgentReq {
    pub session_id: String,
    #[serde(default = "default_cols")]
    pub cols: u16,
    #[serde(default = "default_rows")]
    pub rows: u16,
}

async fn resume_agent_handler(
    State(s): State<ApiState>,
    headers: HeaderMap,
    AxumPath(slug): AxumPath<String>,
    Json(req): Json<ResumeAgentReq>,
) -> impl IntoResponse {
    if !verify_token(&headers, &s.token) {
        return unauthorized();
    }
    let Some(actor) = agent::actor(&slug) else {
        return invalid_agent(&slug);
    };
    let Some((session, workdir)) = agent_session_context(&s).await else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "no session available"})),
        )
            .into_response();
    };
    let Some(launch_cmd) = agent::resume_launch_command(actor.slug(), &req.session_id) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "missing session id"})),
        )
            .into_response();
    };

    match create_terminal(
        &session,
        req.cols,
        req.rows,
        SpawnOptions {
            workdir: Some(workdir),
            launch_cmd: Some(launch_cmd.clone()),
            color_scheme: None,
            env: Vec::new(),
        },
    )
    .await
    {
        Ok(terminal_id) => {
            zedra_telemetry::send(Event::HostTerminalOpen {
                has_launch_cmd: true,
            });
            let terminal_count = session.terminals.lock().await.len();
            if let Err(e) =
                metrics::record_terminal_created(&s.daemon_state.workdir, terminal_count)
            {
                tracing::warn!("Failed to record terminal metrics: {}", e);
            }
            // Reuse the validated actor slug; re-detecting from launch_cmd drops wrappers.
            let agent_slug = Some(actor.slug().to_string());
            session
                .push_event(HostEvent::TerminalCreated {
                    id: terminal_id.clone(),
                    launch_cmd: Some(launch_cmd),
                    agent_slug,
                })
                .await;
            Json(AgentResumeResult {
                terminal_id,
                error: None,
            })
            .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct AgentHookReq {
    pub event_name: Option<String>,
    pub terminal_id: Option<String>,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub tool_name: Option<String>,
    #[serde(default)]
    pub payload: serde_json::Value,
}

async fn receive_agent_hook_handler(
    State(s): State<ApiState>,
    headers: HeaderMap,
    AxumPath(slug): AxumPath<String>,
    Json(req): Json<AgentHookReq>,
) -> impl IntoResponse {
    if !verify_token(&headers, &s.token) {
        return unauthorized();
    }
    let Some(actor) = agent::actor(&slug) else {
        return invalid_agent(&slug);
    };

    let Some(terminal_id) = req
        .terminal_id
        .clone()
        .or_else(|| payload_string(&req.payload, "terminal_id"))
    else {
        return Json(serde_json::json!({"ok": true})).into_response();
    };

    let registry = s.registry.clone();
    let Some(session) = registry.resolve_session_by_tid(&terminal_id).await else {
        return Json(serde_json::json!({"ok": true})).into_response();
    };

    let endpoint_addr =
        encode_endpoint_identity(s.daemon_state.identity.endpoint_id()).unwrap_or_default();
    let delta = s.daemon_state.delta.read().await.clone();
    // Prefer the owning session's workdir so workspace-scoped hook lookups hit the
    // right state DB; fall back to the daemon default for sessions without one.
    let workdir = session
        .workdir
        .clone()
        .unwrap_or_else(|| s.daemon_state.workdir.clone());
    let payload = req.payload.clone();

    tokio::spawn(async move {
        let ctx = HookContext {
            payload,
            terminal_id,
            endpoint_addr,
            session,
            delta,
            workdir,
        };
        let result = actor.receive_hook(ctx).await;
        if let Err(err) = result {
            tracing::warn!(error = %err, "agent hook receiver failed");
        }
    });

    Json(serde_json::json!({"ok": true})).into_response()
}

// ---------------------------------------------------------------------------
// Server startup
// ---------------------------------------------------------------------------

/// Start the local REST API server.
///
/// Returns the bound `SocketAddr`. The caller is responsible for writing the
/// address and token to the config directory for tool discovery.
pub async fn start(state: ApiState) -> anyhow::Result<std::net::SocketAddr> {
    let app = Router::new()
        .route("/api/status", get(status))
        .route("/api/qr", post(create_pairing_qr_handler))
        .route("/api/qr/static", post(create_static_pairing_qr_handler))
        .route("/api/terminal", post(create_terminal_handler))
        .route("/api/agents", get(list_agents_handler))
        .route(
            "/api/agents/:kind/sessions",
            get(list_agent_sessions_handler),
        )
        .route("/api/agents/:kind/resume", post(resume_agent_handler))
        .route("/api/agent-hooks/:kind", post(receive_agent_hook_handler))
        .with_state(state);

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!("REST API server error: {}", e);
        }
    });
    Ok(addr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::HostIdentity;
    use crate::session_registry::ConsumeSlotResult;

    #[tokio::test]
    async fn qr_endpoint_requires_token_and_creates_pairing_slot() {
        let dir = tempfile::tempdir().unwrap();
        let identity = Arc::new(HostIdentity::load_or_generate_for_workdir(dir.path()).unwrap());
        let registry = Arc::new(SessionRegistry::new());
        let session = registry
            .create_named("test", dir.path().to_path_buf())
            .await;
        let daemon_state = Arc::new(DaemonState::new(
            dir.path().to_path_buf(),
            identity.clone(),
            [7; 32],
            None,
        ));
        let endpoint = iroh::Endpoint::builder()
            .relay_mode(iroh::RelayMode::Disabled)
            .secret_key(iroh::SecretKey::from(rand::random::<[u8; 32]>()))
            .bind()
            .await
            .unwrap();
        let addr = start(ApiState {
            registry: registry.clone(),
            daemon_state,
            endpoint: endpoint.clone(),
            relay_urls: vec!["https://test.relay.zedra.dev".to_string()],
            token: "test-token".to_string(),
        })
        .await
        .unwrap();

        let client = reqwest::Client::new();
        let url = format!("http://{addr}/api/qr");
        let static_url = format!("http://{addr}/api/qr/static");
        let unauthorized = client.post(&url).send().await.unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
        let unauthorized_static = client.post(&static_url).send().await.unwrap();
        assert_eq!(unauthorized_static.status(), StatusCode::UNAUTHORIZED);

        let info: qr::StartupInfo = client
            .post(&url)
            .bearer_auth("test-token")
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(!info.pairing_static);
        assert_eq!(info.pairing_expires_in_secs, Some(600));
        let ticket = ZedraPairingTicket::from_pairing_url(&info.pairing_url).unwrap();
        assert_eq!(ticket.session_id, session.id);
        assert_eq!(ticket.endpoint_id, identity.endpoint_id());

        match registry.consume_pairing_slot(&session.id).await {
            ConsumeSlotResult::Active(slot) => {
                assert_eq!(slot.handshake_secret, ticket.handshake_secret);
            }
            _ => panic!("expected active pairing slot"),
        }
        assert!(matches!(
            registry.consume_pairing_slot(&session.id).await,
            ConsumeSlotResult::Consumed
        ));

        let info: qr::StartupInfo = client
            .post(&static_url)
            .bearer_auth("test-token")
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(info.pairing_static);
        assert_eq!(info.pairing_expires_in_secs, None);
        let ticket = ZedraPairingTicket::from_pairing_url(&info.pairing_url).unwrap();
        assert_eq!(ticket.session_id, session.id);
        assert_eq!(ticket.endpoint_id, identity.endpoint_id());

        for _ in 0..2 {
            match registry.consume_pairing_slot(&session.id).await {
                ConsumeSlotResult::Active(slot) => {
                    assert_eq!(slot.handshake_secret, ticket.handshake_secret);
                }
                _ => panic!("expected static active pairing slot"),
            }
        }

        endpoint.close().await;
    }
}
