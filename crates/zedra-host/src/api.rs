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

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;

use crate::agent;
use crate::metrics;
use crate::pty::SpawnOptions;
use crate::qr;
use crate::rpc_daemon::{create_terminal, AgentHookEventRecord, AgentHookProviderIds, DaemonState};
use crate::session_registry::{PairingSlotMode, ServerSession, SessionRegistry};
use chrono::Utc;
use zedra_rpc::proto::{
    AgentEventSummary, AgentResumeResult, HostEvent, ManagedAgentKind, TerminalColorScheme,
};
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
struct StatusLspServer {
    language: String,
    state: String,
    pid: Option<u32>,
    rss_bytes: u64,
    uptime_secs: u64,
    diagnostic_errors: u32,
    diagnostic_warnings: u32,
    last_request_ms: Option<u32>,
    last_kill_reason: Option<String>,
    peak_rss_bytes: u64,
}

#[derive(Debug, Serialize)]
struct StatusLsp {
    enabled: bool,
    servers: Vec<StatusLspServer>,
    aggregate_rss_bytes: u64,
    aggregate_rss_cap_bytes: u64,
    concurrent_cap: u32,
}

#[derive(Debug, Serialize)]
struct StatusResp {
    ok: bool,
    version: &'static str,
    endpoint_id: String,
    workdir: String,
    uptime_secs: u64,
    sessions: Vec<StatusSession>,
    terminals: Vec<StatusTerminal>,
    lsp: StatusLsp,
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

    let lsp = lsp_status_snapshot(&s).await;

    Json(StatusResp {
        ok: true,
        version,
        endpoint_id,
        workdir,
        uptime_secs,
        sessions,
        terminals: all_terminals,
        lsp,
    })
    .into_response()
}

async fn lsp_status_snapshot(s: &ApiState) -> StatusLsp {
    let snap = s.daemon_state.lsp.status_snapshot().await;
    StatusLsp {
        enabled: snap.enabled,
        servers: snap
            .servers
            .into_iter()
            .map(|server| StatusLspServer {
                language: lsp_language_label(server.language).to_string(),
                state: lsp_state_label(server.state).to_string(),
                pid: server.pid,
                rss_bytes: server.rss_bytes,
                uptime_secs: server.uptime_secs,
                diagnostic_errors: server.diagnostic_errors,
                diagnostic_warnings: server.diagnostic_warnings,
                last_request_ms: server.last_request_ms,
                last_kill_reason: server
                    .last_kill_reason
                    .map(|r| lsp_kill_reason_label(r).to_string()),
                peak_rss_bytes: server.peak_rss_bytes,
            })
            .collect(),
        aggregate_rss_bytes: snap.aggregate_rss_bytes,
        aggregate_rss_cap_bytes: snap.aggregate_rss_cap_bytes,
        concurrent_cap: snap.concurrent_cap,
    }
}

fn lsp_language_label(language: zedra_rpc::proto::LspLanguage) -> &'static str {
    use zedra_rpc::proto::LspLanguage::*;
    match language {
        Rust => "rust",
        Go => "go",
        TypeScript => "typescript",
        JavaScript => "javascript",
        Python => "python",
    }
}

fn lsp_state_label(state: zedra_rpc::proto::LspServerState) -> &'static str {
    use zedra_rpc::proto::LspServerState::*;
    match state {
        Idle => "idle",
        Starting => "starting",
        Ready => "ready",
        Failed => "failed",
        Killed => "killed",
        Disabled => "disabled",
    }
}

fn lsp_kill_reason_label(reason: zedra_rpc::proto::LspKillReason) -> &'static str {
    use zedra_rpc::proto::LspKillReason::*;
    match reason {
        Oom => "oom",
        AggregateOom => "aggregate_oom",
        Cpu => "cpu",
        Idle => "idle",
        Manual => "manual",
        Crash => "crash",
    }
}

fn parse_lsp_language(raw: &str) -> Option<zedra_rpc::proto::LspLanguage> {
    use zedra_rpc::proto::LspLanguage::*;
    match raw.trim().to_ascii_lowercase().as_str() {
        "rust" | "rs" | "rust-analyzer" => Some(Rust),
        "go" | "gopls" | "golang" => Some(Go),
        "typescript" | "ts" => Some(TypeScript),
        "javascript" | "js" => Some(JavaScript),
        "python" | "py" => Some(Python),
        _ => None,
    }
}

#[derive(Debug, Deserialize)]
struct LspLanguageRequest {
    language: String,
}

async fn lsp_enable_handler(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Json(body): Json<LspLanguageRequest>,
) -> impl IntoResponse {
    if !verify_token(&headers, &s.token) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "unauthorized"})),
        )
            .into_response();
    }
    let Some(language) = parse_lsp_language(&body.language) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("unknown language: {}", body.language)})),
        )
            .into_response();
    };
    if let Err(e) = s.daemon_state.lsp.enable(language).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response();
    }
    Json(serde_json::json!({
        "ok": true,
        "language": lsp_language_label(language),
        "enabled": true,
    }))
    .into_response()
}

async fn lsp_disable_handler(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Json(body): Json<LspLanguageRequest>,
) -> impl IntoResponse {
    if !verify_token(&headers, &s.token) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "unauthorized"})),
        )
            .into_response();
    }
    let Some(language) = parse_lsp_language(&body.language) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("unknown language: {}", body.language)})),
        )
            .into_response();
    };
    if let Err(e) = s.daemon_state.lsp.disable(language).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response();
    }
    Json(serde_json::json!({
        "ok": true,
        "language": lsp_language_label(language),
        "enabled": false,
    }))
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
            session
                .push_event(HostEvent::TerminalCreated {
                    id: id.clone(),
                    launch_cmd,
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

fn invalid_agent(kind: &str) -> axum::response::Response {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({"error": format!("unsupported managed agent: {kind}")})),
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
    AxumPath(kind): AxumPath<String>,
) -> impl IntoResponse {
    if !verify_token(&headers, &s.token) {
        return unauthorized();
    }
    let Some(kind) = agent::managed_kind_from_slug(&kind) else {
        return invalid_agent(&kind);
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
            kind,
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
    AxumPath(kind): AxumPath<String>,
    Json(req): Json<ResumeAgentReq>,
) -> impl IntoResponse {
    if !verify_token(&headers, &s.token) {
        return unauthorized();
    }
    let Some(kind) = agent::managed_kind_from_slug(&kind) else {
        return invalid_agent(&kind);
    };
    let Some((session, workdir)) = agent_session_context(&s).await else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "no session available"})),
        )
            .into_response();
    };
    let Some(launch_cmd) = agent::resume_launch_command(kind, &req.session_id) else {
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
            session
                .push_event(HostEvent::TerminalCreated {
                    id: terminal_id.clone(),
                    launch_cmd: Some(launch_cmd),
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

#[derive(Debug, Serialize)]
struct AgentHookResp {
    ok: bool,
    seq: u64,
    kind: ManagedAgentKind,
    provider_event_name: String,
    provider_ids: AgentHookProviderIds,
    normalized: Option<AgentEventSummary>,
    terminal_bound: bool,
    warning: Option<String>,
}

async fn receive_agent_hook_handler(
    State(s): State<ApiState>,
    headers: HeaderMap,
    AxumPath(kind): AxumPath<String>,
    Json(req): Json<AgentHookReq>,
) -> impl IntoResponse {
    if !verify_token(&headers, &s.token) {
        return unauthorized();
    }
    let Some(kind) = agent::managed_kind_from_slug(&kind) else {
        return invalid_agent(&kind);
    };

    let event_name = req
        .event_name
        .clone()
        .or_else(|| payload_string(&req.payload, "hook_event_name"))
        .or_else(|| payload_string(&req.payload, "event_name"))
        .or_else(|| payload_string(&req.payload, "type"))
        .or_else(|| {
            req.payload
                .get("event")
                .and_then(|event| payload_string(event, "type"))
        })
        .unwrap_or_else(|| "unknown".to_string());

    let terminal_id = req
        .terminal_id
        .clone()
        .or_else(|| payload_string(&req.payload, "terminal_id"));
    let provider_ids = extract_provider_ids(&req);
    let terminal_bound = match terminal_id.as_deref() {
        Some(id) if !id.is_empty() => terminal_exists(&s.registry, id).await,
        _ => false,
    };
    let warning = if terminal_id.as_deref().filter(|id| !id.is_empty()).is_none() {
        Some("hook did not include ZEDRA_TERMINAL_ID".to_string())
    } else if !terminal_bound {
        Some("hook terminal id is not active in this daemon".to_string())
    } else {
        None
    };

    let normalized =
        agent::normalize_event(kind, &event_name).map(|(event_kind, status)| AgentEventSummary {
            kind: event_kind,
            status,
            at: Some(Utc::now()),
            terminal_id: terminal_id.clone(),
            session_id: req
                .session_id
                .clone()
                .or_else(|| provider_ids.session_id.clone())
                .or_else(|| payload_string(&req.payload, "session_id"))
                .or_else(|| payload_string(&req.payload, "sessionID")),
            turn_id: req
                .turn_id
                .clone()
                .or_else(|| provider_ids.turn_id.clone())
                .or_else(|| payload_string(&req.payload, "turn_id"))
                .or_else(|| payload_string(&req.payload, "turnID")),
            tool_name: req
                .tool_name
                .clone()
                .or_else(|| payload_string(&req.payload, "tool_name"))
                .or_else(|| payload_string(&req.payload, "tool")),
        });
    let record = AgentHookEventRecord {
        seq: 0,
        kind,
        provider_event_name: event_name.clone(),
        provider_ids: provider_ids.clone(),
        terminal_id: terminal_id.clone(),
        normalized: normalized.clone(),
        terminal_bound,
        warning: warning.clone(),
    };
    let seq = s.daemon_state.record_agent_hook_event(record).await;

    tracing::info!(
        ?kind,
        provider_event_name = %event_name,
        terminal_bound,
        normalized = ?normalized.as_ref().map(|event| event.kind),
        "agent hook received"
    );

    Json(AgentHookResp {
        ok: normalized.is_some(),
        seq,
        kind,
        provider_event_name: event_name,
        provider_ids,
        normalized,
        terminal_bound,
        warning,
    })
    .into_response()
}

#[derive(Debug, Deserialize)]
pub struct AgentHookEventsQuery {
    pub terminal_id: Option<String>,
    #[serde(default)]
    pub after: u64,
    #[serde(default = "default_hook_event_limit")]
    pub limit: usize,
}

fn default_hook_event_limit() -> usize {
    100
}

#[derive(Debug, Serialize)]
struct AgentHookEventsResp {
    events: Vec<AgentHookEventRecord>,
}

async fn list_agent_hook_events_handler(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Query(query): Query<AgentHookEventsQuery>,
) -> impl IntoResponse {
    if !verify_token(&headers, &s.token) {
        return unauthorized();
    }
    let events = s
        .daemon_state
        .list_agent_hook_events(query.terminal_id.as_deref(), query.after, query.limit)
        .await;
    Json(AgentHookEventsResp { events }).into_response()
}

fn payload_string(payload: &serde_json::Value, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn extract_provider_ids(req: &AgentHookReq) -> AgentHookProviderIds {
    let mut ids = AgentHookProviderIds {
        session_id: req
            .session_id
            .clone()
            .or_else(|| payload_string(&req.payload, "session_id"))
            .or_else(|| payload_string(&req.payload, "sessionID")),
        turn_id: req
            .turn_id
            .clone()
            .or_else(|| payload_string(&req.payload, "turn_id"))
            .or_else(|| payload_string(&req.payload, "turnID")),
        tool_use_id: payload_string(&req.payload, "tool_use_id")
            .or_else(|| payload_string(&req.payload, "toolUseID"))
            .or_else(|| payload_string(&req.payload, "toolUseId")),
        task_id: payload_string(&req.payload, "task_id")
            .or_else(|| payload_string(&req.payload, "taskID"))
            .or_else(|| payload_string(&req.payload, "taskId")),
        agent_id: payload_string(&req.payload, "agent_id")
            .or_else(|| payload_string(&req.payload, "agentID"))
            .or_else(|| payload_string(&req.payload, "agentId")),
        elicitation_id: payload_string(&req.payload, "elicitation_id")
            .or_else(|| payload_string(&req.payload, "elicitationID"))
            .or_else(|| payload_string(&req.payload, "elicitationId")),
        transcript_id: payload_string(&req.payload, "transcript_path")
            .and_then(|path| transcript_id_from_path(&path)),
        batch_tool_use_ids: Vec::new(),
    };
    ids.batch_tool_use_ids = req
        .payload
        .get("tool_calls")
        .and_then(serde_json::Value::as_array)
        .map(|tool_calls| {
            tool_calls
                .iter()
                .filter_map(|tool_call| payload_string(tool_call, "tool_use_id"))
                .collect()
        })
        .unwrap_or_default();
    ids
}

fn transcript_id_from_path(path: &str) -> Option<String> {
    std::path::Path::new(path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .map(str::to_string)
}

async fn terminal_exists(registry: &SessionRegistry, terminal_id: &str) -> bool {
    for session in registry.list_sessions().await {
        let Some(session) = registry.get(&session.id).await else {
            continue;
        };
        if session
            .terminal_infos()
            .await
            .iter()
            .any(|terminal| terminal.id == terminal_id)
        {
            return true;
        }
    }
    false
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
        .route(
            "/api/agent-hooks/events",
            get(list_agent_hook_events_handler),
        )
        .route("/api/agent-hooks/:kind", post(receive_agent_hook_handler))
        .route("/api/lsp/enable", post(lsp_enable_handler))
        .route("/api/lsp/disable", post(lsp_disable_handler))
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
        let daemon_state = Arc::new(DaemonState::new(dir.path().to_path_buf(), identity.clone()));
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

    #[test]
    fn extracts_safe_provider_ids_from_claude_tool_payload() {
        let req = AgentHookReq {
            event_name: Some("PreToolUse".to_string()),
            terminal_id: None,
            session_id: None,
            turn_id: None,
            tool_name: None,
            payload: serde_json::json!({
                "session_id": "claude-session",
                "transcript_path": "/Users/me/.claude/projects/repo/claude-session.jsonl",
                "tool_use_id": "toolu_123",
                "tool_input": {"command": "secret command should not be copied"}
            }),
        };

        let ids = extract_provider_ids(&req);
        assert_eq!(ids.session_id.as_deref(), Some("claude-session"));
        assert_eq!(ids.tool_use_id.as_deref(), Some("toolu_123"));
        assert_eq!(ids.transcript_id.as_deref(), Some("claude-session"));
    }

    #[test]
    fn extracts_batch_tool_ids_without_tool_outputs() {
        let req = AgentHookReq {
            event_name: Some("PostToolBatch".to_string()),
            terminal_id: None,
            session_id: None,
            turn_id: None,
            tool_name: None,
            payload: serde_json::json!({
                "session_id": "claude-session",
                "tool_calls": [
                    {"tool_use_id": "toolu_1", "tool_response": "large output"},
                    {"tool_use_id": "toolu_2", "tool_input": {"file_path": "/tmp/a"}}
                ]
            }),
        };

        let ids = extract_provider_ids(&req);
        assert_eq!(ids.batch_tool_use_ids, vec!["toolu_1", "toolu_2"]);
    }
}
