use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use anyhow::Result;
use tracing::{info, warn};

use zedra_rpc::proto::{AgentState, HostEvent};

use crate::session_registry::ServerSession;
use crate::{
    delta::{DeltaClient, NotificationPriority},
    utils,
};

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
}

/// Shared hook plumbing driven by `AgentActor::receive_hook` — the actor is
/// the hook handler; there is no per-agent receiver type.
impl HookContext {
    /// Apply the agent-state transition (when the event maps to one) and forward
    /// the raw hook to the connected client.
    pub async fn apply(
        &self,
        slug: &str,
        event_name: &str,
        agent_state: Option<AgentState>,
        agent_session_id: Option<&str>,
    ) {
        if let Some(state) = agent_state {
            self.session
                .set_agent_state(
                    self.terminal_id.clone(),
                    agent_session_id.unwrap_or(""),
                    state,
                )
                .await;
        }
        self.push_rpc(slug, event_name).await;
    }

    /// Whether the previously signed-in client currently has the app foregrounded.
    /// Notifications are suppressed when it does.
    pub fn client_in_foreground(&self) -> bool {
        self.session.client_in_foreground.load(Ordering::Relaxed)
    }

    /// Forward a raw hook event to the session's connected client.
    pub async fn push_rpc(&self, slug: &str, event_name: &str) {
        let delivered = self
            .session
            .push_event(HostEvent::AgentHookReceived {
                agent_slug: slug.to_string(),
                event_name: event_name.to_owned(),
                payload: self.payload.to_string(),
            })
            .await;
        if delivered {
            info!(
                agent = slug,
                event_name, "agent hook rpc delivered to client"
            );
        }
    }

    /// No-op when the app is foregrounded or Delta is unconfigured; `body` is
    /// awaited only when a push goes out, so defer transcript/DB reads into it.
    pub async fn notify(
        &self,
        agent: &str,
        event_name: &str,
        title: String,
        body: impl Future<Output = Option<String>>,
    ) -> Result<()> {
        if self.client_in_foreground() {
            return Ok(());
        }
        let Some(delta) = self.require_delta() else {
            return Ok(());
        };
        let body = body.await;
        self.send_notification(&delta, self.notification(agent, event_name, title, body))
            .await
    }

    /// Shared Delta send: push notification to the previous signed-in client. Body is reduced to
    /// its first non-empty line and truncated to 100 chars.
    pub async fn send_notification(
        &self,
        client: &DeltaClient,
        notification: HookNotification,
    ) -> Result<()> {
        let body = notification.body.as_deref().and_then(|b| {
            super::utils::first_non_empty_line(b).map(|line| utils::truncate_chars(&line, 100))
        });
        // Deeplink pushes go high priority so a backgrounded device wakes for
        // the tap; Delta only relays the priority header.
        let priority = if notification.deeplink.is_some() {
            NotificationPriority::High
        } else {
            NotificationPriority::Normal
        };
        // TODO: Live Activity send disabled (feature incomplete) — restore the
        // parallel `update_live_activity_for_stack` call to re-enable.
        let notify_result = client
            .send_notification_to_client(
                notification.title,
                body,
                Some("agent".to_string()),
                notification.deeplink,
                priority,
            )
            .await;
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
        Ok(())
    }

    /// Return the in-memory Delta client, logging a skip when Delta is not
    /// configured. Callers early-return when this yields `None`.
    pub fn require_delta(&self) -> Option<Arc<DeltaClient>> {
        if self.delta.is_none() {
            // TODO: host side might still be able to send a notification without Delta configured.
            warn!("agent hook Delta notification skipped: no in-memory client delta");
        }
        self.delta.clone()
    }

    /// Notification envelope; deeplink comes from the context. Restore the
    /// `{agent, event}` content-state field with the Live Activity TODO above.
    pub fn notification(
        &self,
        _agent: &str,
        _event_name: &str,
        title: String,
        body: Option<String>,
    ) -> HookNotification {
        HookNotification {
            title,
            body,
            deeplink: self.build_deeplink(),
        }
    }

    /// Deeplink that opens the originating terminal in the app.
    pub fn build_deeplink(&self) -> Option<String> {
        if self.endpoint_addr.is_empty() || self.terminal_id.is_empty() {
            return None;
        }
        Some(format!(
            "zedra://open?endpoint_addr={}&terminal_id={}",
            self.endpoint_addr, self.terminal_id
        ))
    }
}
