use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use anyhow::Result;
use tracing::{info, warn};

use zedra_rpc::proto::{AgentKind, AgentState, HostEvent};

use crate::session_registry::ServerSession;
use crate::{agent_utils, delta::DeltaClient};

pub struct HookContext {
    pub payload: serde_json::Value,
    pub terminal_id: String,
    pub endpoint_addr: String,
    pub session: Arc<ServerSession>,
    /// `None` when Delta is not configured; the RPC/state path still runs.
    pub delta: Option<Arc<DeltaClient>>,
    pub workdir: PathBuf,
}

pub struct HookNotification {
    pub title: String,
    pub body: Option<String>,
    pub deeplink: Option<String>,
    pub content_state: serde_json::Value,
}

#[allow(async_fn_in_trait)]
pub trait HookReceiver {
    /// Handle one hook event: extract the event name, update agent state, forward
    /// the raw event to the client, and notify when the app is backgrounded.
    async fn receive(&self, ctx: HookContext) -> Result<()>;

    /// Apply the agent-state transition (when the event maps to one) and forward
    /// the raw hook to the connected client. Returns whether a Delta notification
    /// should follow — i.e. the app is currently backgrounded.
    async fn apply_and_should_notify(
        &self,
        kind: AgentKind,
        event_name: &str,
        agent_state: Option<AgentState>,
        agent_session_id: Option<&str>,
        ctx: &HookContext,
    ) -> bool {
        if let (Some(state), Some(sid)) = (agent_state, agent_session_id) {
            ctx.session
                .set_agent_state(ctx.terminal_id.clone(), sid, state)
                .await;
        }
        self.push_rpc(kind, event_name, ctx).await;
        !ctx.session.client_in_foreground.load(Ordering::Relaxed)
    }

    /// Forward a raw hook event to the session's connected client.
    async fn push_rpc(&self, kind: AgentKind, event_name: &str, ctx: &HookContext) {
        let delivered = ctx
            .session
            .push_event(HostEvent::AgentHookReceived {
                agent_kind: kind,
                event_name: event_name.to_owned(),
                payload: ctx.payload.to_string(),
            })
            .await;
        if delivered {
            info!(?kind, event_name, "agent hook rpc delivered to client");
        }
    }

    /// Shared Delta send: notification + live activity in parallel. Body is
    /// reduced to its first non-empty line and truncated to 100 chars.
    async fn send_notification(
        &self,
        client: &DeltaClient,
        notification: HookNotification,
    ) -> Result<()> {
        let body = notification.body.and_then(|b| {
            let first = b.lines().next().unwrap_or("").trim();
            (!first.is_empty()).then(|| truncate_str(first, 100))
        });
        let (notify_result, activity_result) = tokio::join!(
            client.send_notification_to_stack(
                notification.title.clone(),
                body.clone(),
                Some("agent".to_string()),
                notification.deeplink.clone(),
            ),
            client.update_live_activity_for_stack(
                "zedra-agent".to_string(),
                Some(notification.title),
                body,
                notification.content_state,
                false,
            ),
        );
        if let Err(err) = notify_result {
            warn!(error = %err, "agent hook Delta notification failed");
        }
        if let Err(err) = activity_result {
            warn!(error = %err, "agent hook Delta live activity failed");
        }
        Ok(())
    }

    /// Live Activity content state shared by every agent notification.
    fn content_state(&self, agent: &str, event_name: &str) -> serde_json::Value {
        serde_json::json!({ "agent": agent, "event": event_name })
    }

    /// Deeplink that opens the originating terminal in the app.
    fn build_deeplink(&self, ctx: &HookContext) -> Option<String> {
        if ctx.endpoint_addr.is_empty() || ctx.terminal_id.is_empty() {
            return None;
        }
        Some(format!(
            "zedra://open-terminal?endpoint={}&terminal_id={}",
            ctx.endpoint_addr, ctx.terminal_id
        ))
    }

    /// Read the Claude session title from the transcript referenced in the payload.
    async fn read_transcript_title(&self, payload: &serde_json::Value) -> Option<String> {
        let transcript_path = payload
            .get("transcript_path")
            .and_then(|v| v.as_str())
            .map(PathBuf::from);
        tokio::task::spawn_blocking(move || {
            transcript_path
                .as_deref()
                .and_then(crate::agent_claude::title_from_transcript_path)
        })
        .await
        .unwrap_or(None)
    }
}

pub struct ClaudeHookReceiver;

impl HookReceiver for ClaudeHookReceiver {
    async fn receive(&self, ctx: HookContext) -> Result<()> {
        // Claude Code pipes hook JSON with `hook_event_name` and snake_case `session_id`.
        let event_name =
            agent_utils::payload_string(&ctx.payload, "hook_event_name").unwrap_or_default();
        let agent_session_id = agent_utils::payload_string(&ctx.payload, "session_id");
        let agent_state = match event_name.as_str() {
            "UserPromptSubmit" => Some(AgentState::Running),
            "PermissionRequest" => Some(AgentState::WaitingApproval),
            "Stop" => Some(AgentState::Completed),
            _ => None,
        };
        if !self
            .apply_and_should_notify(
                AgentKind::Claude,
                &event_name,
                agent_state,
                agent_session_id.as_deref(),
                &ctx,
            )
            .await
        {
            return Ok(());
        }
        let Some(delta) = ctx.delta.clone() else {
            return Ok(());
        };

        let name = agent_utils::display_name(AgentKind::Claude);
        let Some(title) = (match event_name.as_str() {
            "PermissionRequest" => Some(format!("{name} requires approval")),
            "Stop" => Some(format!("{name} finished")),
            _ => None,
        }) else {
            return Ok(());
        };

        let body = self.read_transcript_title(&ctx.payload).await;
        info!(event_name, "agent hook delta notification (claude)");
        self.send_notification(
            &delta,
            HookNotification {
                title,
                body,
                deeplink: self.build_deeplink(&ctx),
                content_state: self.content_state(name, &event_name),
            },
        )
        .await
    }
}

pub struct CodexHookReceiver;

impl HookReceiver for CodexHookReceiver {
    async fn receive(&self, ctx: HookContext) -> Result<()> {
        // Codex pipes hook JSON with `hook_event_name` and snake_case `session_id`.
        let event_name =
            agent_utils::payload_string(&ctx.payload, "hook_event_name").unwrap_or_default();
        let agent_session_id = agent_utils::payload_string(&ctx.payload, "session_id");
        let agent_state = match event_name.as_str() {
            "UserPromptSubmit" => Some(AgentState::Running),
            "PermissionRequest" => Some(AgentState::WaitingApproval),
            "Stop" => Some(AgentState::Completed),
            _ => None,
        };
        if !self
            .apply_and_should_notify(
                AgentKind::Codex,
                &event_name,
                agent_state,
                agent_session_id.as_deref(),
                &ctx,
            )
            .await
        {
            return Ok(());
        }
        let Some(delta) = ctx.delta.clone() else {
            return Ok(());
        };

        let name = agent_utils::display_name(AgentKind::Codex);
        // Only notify for permission requests — Stop fires on every turn end.
        let Some(title) = (match event_name.as_str() {
            "PermissionRequest" => Some(format!("{name} requires approval")),
            _ => None,
        }) else {
            return Ok(());
        };

        // Codex stores titles in its thread DB; look up by session id.
        let workdir = ctx.workdir.clone();
        let body = tokio::task::spawn_blocking(move || {
            agent_session_id
                .as_deref()
                .and_then(|id| crate::agent_codex::title_for_session(&workdir, id))
        })
        .await
        .unwrap_or(None);

        info!(event_name, "agent hook delta notification (codex)");
        self.send_notification(
            &delta,
            HookNotification {
                title,
                body,
                deeplink: self.build_deeplink(&ctx),
                content_state: self.content_state(name, &event_name),
            },
        )
        .await
    }
}

pub struct OpenCodeHookReceiver;

impl HookReceiver for OpenCodeHookReceiver {
    async fn receive(&self, ctx: HookContext) -> Result<()> {
        // The OpenCode plugin sends a top-level event name and OpenCode's native
        // `sessionID` (capital ID). Accept `type` as a fallback for the event name
        // since synthetic/test payloads use it.
        let event_name = agent_utils::payload_string(&ctx.payload, "event")
            .or_else(|| agent_utils::payload_string(&ctx.payload, "type"))
            .unwrap_or_default();
        let agent_session_id = agent_utils::payload_string(&ctx.payload, "sessionID")
            .or_else(|| agent_utils::payload_string(&ctx.payload, "sessionId"));
        // permission.ask → WaitingApproval; chat.message → Completed.
        // No Running transition — OpenCode has no direct user-submit hook.
        let agent_state = match event_name.as_str() {
            "permission.ask" | "permission.asked" => Some(AgentState::WaitingApproval),
            "chat.message" => Some(AgentState::Completed),
            _ => None,
        };
        if !self
            .apply_and_should_notify(
                AgentKind::OpenCode,
                &event_name,
                agent_state,
                agent_session_id.as_deref(),
                &ctx,
            )
            .await
        {
            return Ok(());
        }
        let Some(delta) = ctx.delta.clone() else {
            return Ok(());
        };

        let name = agent_utils::display_name(AgentKind::OpenCode);
        // Only notify for permission requests — chat.message fires too often.
        let Some(title) = (match event_name.as_str() {
            "permission.ask" | "permission.asked" => Some(format!("{name} requires approval")),
            _ => None,
        }) else {
            return Ok(());
        };

        info!(event_name, "agent hook delta notification (opencode)");
        self.send_notification(
            &delta,
            HookNotification {
                title,
                body: None,
                deeplink: self.build_deeplink(&ctx),
                content_state: self.content_state(name, &event_name),
            },
        )
        .await
    }
}

/// Truncate a string to at most `max_chars` Unicode scalar values, appending `…` if cut.
fn truncate_str(s: &str, max_chars: usize) -> String {
    let mut chars = s.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}
