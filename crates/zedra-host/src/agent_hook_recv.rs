use std::path::PathBuf;
use std::sync::Arc;

use zedra_rpc::proto::{AgentEventKind, AgentEventSummary, AgentKind, HostEvent};

use crate::session_registry::{ServerSession, SessionRegistry};
use crate::{agent_utils, delta::DeltaClient};

// ---------------------------------------------------------------------------
// Dispatch context — built once in the handler, owned by each receiver
// ---------------------------------------------------------------------------

pub struct HookContext {
    pub registry: Arc<SessionRegistry>,
    pub terminal_id: Option<String>,
    pub delta: Option<Arc<DeltaClient>>,
    pub workdir: PathBuf,
}

impl HookContext {
    pub async fn session(&self) -> Option<Arc<ServerSession>> {
        let tid = self.terminal_id.as_deref().filter(|id| !id.is_empty())?;
        for info in self.registry.list_sessions().await {
            let Some(session) = self.registry.get(&info.id).await else {
                continue;
            };
            if session.terminal_infos().await.iter().any(|t| t.id == tid) {
                return Some(session);
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Notification payload
// ---------------------------------------------------------------------------

pub struct HookNotification {
    pub title: String,
    pub body: Option<String>,
    pub content_state: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Delta send client — accepts a pre-built payload, no content logic here
// ---------------------------------------------------------------------------

pub struct DeltaHookClient {
    client: Arc<DeltaClient>,
}

impl DeltaHookClient {
    pub fn from_client(client: Option<Arc<DeltaClient>>) -> Option<Self> {
        client.map(|client| Self { client })
    }

    pub async fn send(&self, notification: HookNotification) {
        let (notify_result, activity_result) = tokio::join!(
            self.client.send_notification_to_stack(
                notification.title.clone(),
                notification.body.clone(),
                Some("agent".to_string()),
                None,
            ),
            self.client.update_live_activity_for_stack(
                "zedra-agent".to_string(),
                Some(notification.title),
                notification.body,
                notification.content_state,
                false,
            ),
        );
        if let Err(err) = notify_result {
            tracing::warn!(error = %err, "agent hook Delta notification failed");
        }
        if let Err(err) = activity_result {
            tracing::warn!(error = %err, "agent hook Delta live activity failed");
        }
    }
}

// ---------------------------------------------------------------------------
// Per-agent receivers
// ---------------------------------------------------------------------------

pub struct ClaudeHookReceiver {
    pub transcript_path: Option<PathBuf>,
}

impl ClaudeHookReceiver {
    pub async fn receive(&self, event: AgentEventSummary, ctx: HookContext) {
        let session = ctx.session().await;
        if push_rpc(AgentKind::Claude, &event, session).await {
            return;
        }
        let Some(delta) = DeltaHookClient::from_client(ctx.delta) else {
            return;
        };
        delta.send(self.build_notification(&event).await).await;
    }

    async fn build_notification(&self, event: &AgentEventSummary) -> HookNotification {
        let agent = agent_utils::display_name(AgentKind::Claude);
        let title = event_title(agent, event.kind);
        let transcript_path = self.transcript_path.clone();
        let body = tokio::task::spawn_blocking(move || {
            transcript_path
                .as_deref()
                .and_then(crate::agent_claude::title_from_transcript_path)
        })
        .await
        .unwrap_or(None);
        HookNotification {
            title,
            body,
            content_state: serde_json::json!({
                "agent": agent,
                "event": format!("{:?}", event.kind),
            }),
        }
    }
}

pub struct OpenCodeHookReceiver;

impl OpenCodeHookReceiver {
    pub async fn receive(&self, event: AgentEventSummary, ctx: HookContext) {
        let session = ctx.session().await;
        if push_rpc(AgentKind::OpenCode, &event, session).await {
            return;
        }
        let Some(delta) = DeltaHookClient::from_client(ctx.delta) else {
            return;
        };
        delta.send(self.build_notification(&event).await).await;
    }

    async fn build_notification(&self, event: &AgentEventSummary) -> HookNotification {
        let agent = agent_utils::display_name(AgentKind::OpenCode);
        HookNotification {
            title: event_title(agent, event.kind),
            body: None,
            content_state: serde_json::json!({
                "agent": agent,
                "event": format!("{:?}", event.kind),
            }),
        }
    }
}

pub struct CodexHookReceiver;

impl CodexHookReceiver {
    pub async fn receive(&self, event: AgentEventSummary, ctx: HookContext) {
        let session = ctx.session().await;
        if push_rpc(AgentKind::Codex, &event, session).await {
            return;
        }
        let Some(delta) = DeltaHookClient::from_client(ctx.delta) else {
            return;
        };
        delta
            .send(self.build_notification(&event, &ctx.workdir).await)
            .await;
    }

    async fn build_notification(
        &self,
        event: &AgentEventSummary,
        workdir: &PathBuf,
    ) -> HookNotification {
        let agent = agent_utils::display_name(AgentKind::Codex);
        let title = event_title(agent, event.kind);
        let workdir = workdir.clone();
        let session_id = event.session_id.clone();
        let body = tokio::task::spawn_blocking(move || {
            session_id
                .as_deref()
                .and_then(|id| crate::agent_codex::title_for_session(&workdir, id))
        })
        .await
        .unwrap_or(None);
        HookNotification {
            title,
            body,
            content_state: serde_json::json!({
                "agent": agent,
                "event": format!("{:?}", event.kind),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Push an RPC event to the session's connected client.
/// Returns true if the event was delivered to an active subscriber.
async fn push_rpc(
    agent_kind: AgentKind,
    event: &AgentEventSummary,
    session: Option<Arc<ServerSession>>,
) -> bool {
    let Some(session) = session else { return false };
    session
        .push_event(HostEvent::AgentHookReceived {
            agent_kind,
            event: event.clone(),
        })
        .await
}

fn event_title(agent: &str, kind: AgentEventKind) -> String {
    match kind {
        AgentEventKind::PermissionRequested => format!("{agent} requires approval"),
        AgentEventKind::TaskCompleted => format!("{agent} task completed"),
        AgentEventKind::SessionStarted => format!("{agent} session started"),
        AgentEventKind::TurnCompleted | AgentEventKind::TurnFailed => {
            format!("{agent} turn finished")
        }
        _ => format!("{agent} updated"),
    }
}
