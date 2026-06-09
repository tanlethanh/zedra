use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;

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
        if let Some(state) = agent_state {
            ctx.session
                .set_agent_state(ctx.terminal_id.clone(), agent_session_id.unwrap_or(""), state)
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
        match notify_result {
            Ok(response) if response.recipients == 0 => {
                warn!(
                    accepted = response.accepted,
                    recipients = response.recipients,
                    provider_success = response.provider_success,
                    provider_failure = response.provider_failure,
                    errors = ?response.errors,
                    "agent hook Delta notification accepted with no recipients"
                );
            }
            Ok(response) if response.provider_failure > 0 || response.provider_success == 0 => {
                warn!(
                    accepted = response.accepted,
                    recipients = response.recipients,
                    provider_success = response.provider_success,
                    provider_failure = response.provider_failure,
                    errors = ?response.errors,
                    "agent hook Delta notification completed without full provider delivery"
                );
            }
            Ok(response) => {
                info!(
                    accepted = response.accepted,
                    recipients = response.recipients,
                    provider_success = response.provider_success,
                    provider_failure = response.provider_failure,
                    "agent hook Delta notification accepted by provider"
                );
            }
            Err(err) => {
                warn!(error = %err, "agent hook Delta notification failed");
            }
        }
        match activity_result {
            Ok(response) if response.provider_failure > 0 || response.provider_success == 0 => {
                warn!(
                    accepted = response.accepted,
                    recipients = response.recipients,
                    provider_success = response.provider_success,
                    provider_failure = response.provider_failure,
                    errors = ?response.errors,
                    "agent hook Delta live activity completed without full provider delivery"
                );
            }
            Ok(_) => {}
            Err(err) => {
                warn!(error = %err, "agent hook Delta live activity failed");
            }
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
        let Some(event_name) = agent_utils::payload_string(&ctx.payload, "hook_event_name") else {
            warn!(
                "Claude hook payload missing or empty hook_event_name; ignoring, {:?}",
                ctx.payload
            );
            return Ok(());
        };
        let agent_session_id = agent_utils::payload_string(&ctx.payload, "session_id");
        let agent_state = match event_name.as_str() {
            "UserPromptSubmit" => Some(AgentState::Running),
            "PermissionRequest" => Some(AgentState::WaitingApproval),
            "PostToolUse" => Some(AgentState::Running),
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

        // If Delta is not configured, skip sending a notification.
        // TODO: host side might still able to send notification without Delta being configured.
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
            "PostToolUse" => Some(AgentState::Running),
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
        // Notify on approval requests and turn completion, matching Claude.
        let Some(title) = (match event_name.as_str() {
            "PermissionRequest" => Some(format!("{name} requires approval")),
            "Stop" => Some(format!("{name} completed")),
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
        // event object. Accept flat fields as fallbacks for synthetic/test payloads.
        let event_name = agent_utils::payload_string(&ctx.payload, "event_name")
            .or_else(|| agent_utils::payload_string(&ctx.payload, "event"))
            .or_else(|| agent_utils::payload_string(&ctx.payload, "type"))
            .or_else(|| opencode_event_string(&ctx.payload, "type"))
            .unwrap_or_default();
        let agent_session_id = agent_utils::payload_string(&ctx.payload, "sessionID")
            .or_else(|| agent_utils::payload_string(&ctx.payload, "sessionId"))
            .or_else(|| opencode_event_property_string(&ctx.payload, "sessionID"));
        let agent_state = opencode_agent_state(&event_name, &ctx.payload);
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
        // Notify on approval requests and turn completion, matching Claude and Codex.
        let Some(title) = (match event_name.as_str() {
            "permission.asked" => Some(format!("{name} requires approval")),
            "session.idle" => Some(format!("{name} completed")),
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

fn opencode_event_string(payload: &serde_json::Value, key: &str) -> Option<String> {
    agent_utils::payload_string(payload.get("event")?, key)
}

fn opencode_event_property_string(payload: &serde_json::Value, key: &str) -> Option<String> {
    agent_utils::payload_string(payload.get("event")?.get("properties")?, key)
}

fn opencode_session_status(payload: &serde_json::Value) -> Option<String> {
    let status = payload
        .get("status")
        .or_else(|| payload.get("event")?.get("properties")?.get("status"))?;
    if let Some(status) = status.as_str() {
        return Some(status.to_owned());
    }
    status
        .get("type")?
        .as_str()
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn opencode_agent_state(event_name: &str, payload: &serde_json::Value) -> Option<AgentState> {
    match event_name {
        "permission.asked" => Some(AgentState::WaitingApproval),
        "permission.replied" => Some(AgentState::Running),
        "session.idle" => Some(AgentState::Completed),
        "session.status" => match opencode_session_status(payload)?.as_str() {
            "busy" | "retry" => Some(AgentState::Running),
            "idle" => Some(AgentState::Completed),
            _ => None,
        },
        "session.error" => Some(AgentState::Error),
        _ => None,
    }
}

pub struct PiHookReceiver;

impl HookReceiver for PiHookReceiver {
    async fn receive(&self, ctx: HookContext) -> Result<()> {
        // The pi extension normalizes pi's native events to Claude-compatible
        // names (`UserPromptSubmit`, `Stop`) and pipes snake_case `session_id`.
        // Pi exposes no approval hook, so there is no WaitingApproval transition.
        let event_name =
            agent_utils::payload_string(&ctx.payload, "hook_event_name").unwrap_or_default();
        let agent_session_id = agent_utils::payload_string(&ctx.payload, "session_id");
        let agent_state = match event_name.as_str() {
            "UserPromptSubmit" => Some(AgentState::Running),
            "Stop" => Some(AgentState::Completed),
            _ => None,
        };
        if !self
            .apply_and_should_notify(
                AgentKind::Pi,
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

        let name = agent_utils::display_name(AgentKind::Pi);
        // Only notify on completion — pi has no approval event, and Stop is the
        // single user-meaningful turn boundary.
        let Some(title) = (match event_name.as_str() {
            "Stop" => Some(format!("{name} completed")),
            _ => None,
        }) else {
            return Ok(());
        };

        // Pi stores transcripts per workdir; look up the session title for the body.
        let workdir = ctx.workdir.clone();
        let body = tokio::task::spawn_blocking(move || {
            agent_session_id
                .as_deref()
                .and_then(|id| crate::agent_pi::title_for_session(&workdir, id))
        })
        .await
        .unwrap_or(None);

        info!(event_name, "agent hook delta notification (pi)");
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn opencode_native_events_map_to_agent_states() {
        let busy = json!({
            "event": {
                "type": "session.status",
                "properties": {
                    "sessionID": "ses_123",
                    "status": { "type": "busy" }
                }
            }
        });
        assert_eq!(
            opencode_event_property_string(&busy, "sessionID").as_deref(),
            Some("ses_123")
        );
        assert_eq!(
            opencode_agent_state("session.status", &busy),
            Some(AgentState::Running)
        );
        assert_eq!(
            opencode_agent_state("permission.asked", &json!({})),
            Some(AgentState::WaitingApproval)
        );
        assert_eq!(
            opencode_agent_state("permission.replied", &json!({})),
            Some(AgentState::Running)
        );
        assert_eq!(
            opencode_agent_state("session.idle", &json!({})),
            Some(AgentState::Completed)
        );
    }
}
