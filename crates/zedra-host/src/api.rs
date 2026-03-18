// Local REST API server for zedra-host.
//
// Binds to 127.0.0.1 on an OS-assigned port. The bound address and a random
// bearer token are written to the workspace config directory so local tools
// (e.g. the `/zedra-start` Claude Code skill) can authenticate and trigger
// host actions without going through the iroh transport.
//
// Endpoints:
//   GET  /api/status              — daemon health + active session count
//   POST /api/terminal            — create a terminal in the active session
//
// Auth: every request must carry  Authorization: Bearer <token>
//       where <token> is the contents of  <config_dir>/api-token

use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;

use crate::pty::SpawnOptions;
use crate::rpc_daemon::{create_terminal, DaemonState};
use crate::session_registry::SessionRegistry;
use zedra_rpc::proto::HostEvent;

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct ApiState {
    pub registry: Arc<SessionRegistry>,
    pub daemon_state: Arc<DaemonState>,
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
    Json(serde_json::json!({
        "ok": true,
        "version": version,
        "endpoint_id": endpoint_id,
        "workdir": workdir,
        "uptime_secs": uptime_secs,
    }))
    .into_response()
}

#[derive(Debug, Deserialize)]
pub struct CreateTerminalReq {
    /// Session to create the terminal in. Omit to use the first active session.
    pub session_id: Option<String>,
    /// Command injected on startup (e.g. "claude --resume").
    pub launch_cmd: Option<String>,
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

    // Resolve session: explicit ID → first active session → any session.
    let session = if let Some(id) = &req.session_id {
        s.registry.get(id).await
    } else {
        s.registry.first_session().await
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
    let launch_cmd = req
        .launch_cmd
        .clone()
        .or_else(|| s.daemon_state.default_launch_cmd.clone());
    let opts = SpawnOptions {
        workdir,
        launch_cmd: launch_cmd.clone(),
    };

    match create_terminal(&session, req.cols, req.rows, opts).await {
        Ok(id) => {
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
        .route("/api/terminal", post(create_terminal_handler))
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
