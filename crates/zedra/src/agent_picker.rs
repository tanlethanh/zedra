use gpui::*;
use tracing::error;
use zedra_rpc::proto::InstalledAgentEntry;
use zedra_session::SessionHandle;

use crate::pending::SharedPendingSlot;
use crate::platform_bridge::{self, ListPickerItem};
use crate::workspace::PendingWorkspaceAction;

enum State {
    Idle,
    Loading,
    Ready(Vec<InstalledAgentEntry>),
}

pub struct AgentPicker {
    session_handle: SessionHandle,
    pending_action: SharedPendingSlot<PendingWorkspaceAction>,
    state: State,
    _task: Option<Task<()>>,
}

impl AgentPicker {
    pub fn new(
        session_handle: SessionHandle,
        pending_action: SharedPendingSlot<PendingWorkspaceAction>,
    ) -> Self {
        Self {
            session_handle,
            pending_action,
            state: State::Idle,
            _task: None,
        }
    }

    /// Called when the user taps "Create Agent". Shows the native picker
    /// immediately with cached items if available, otherwise fetches first.
    pub fn trigger(&mut self, cx: &mut Context<Self>) {
        match &self.state {
            State::Ready(agents) => {
                self.present(agents.clone());
            }
            State::Loading => {
                // fetch already in progress; picker will appear when it completes
            }
            State::Idle => {
                self.fetch(cx);
            }
        }
    }

    fn fetch(&mut self, cx: &mut Context<Self>) {
        self.state = State::Loading;
        let handle = self.session_handle.clone();
        let task = cx.spawn(
            async move |this, cx| match handle.agent_installed_list(false).await {
                Ok(agents) => {
                    let _ = this.update(cx, |this, _cx| {
                        let available: Vec<_> =
                            agents.into_iter().filter(|a| a.available).collect();
                        if available.is_empty() {
                            this.present_placeholder("No agents installed on the host.");
                            this.state = State::Idle;
                        } else {
                            this.present(available.clone());
                            this.state = State::Ready(available);
                        }
                    });
                }
                Err(err) => {
                    error!("agent picker: fetch failed: {}", err);
                    let _ = this.update(cx, |this, _cx| {
                        this.state = State::Idle;
                        this.present_placeholder(&err.to_string());
                    });
                }
            },
        );
        self._task = Some(task);
    }

    fn present(&self, agents: Vec<InstalledAgentEntry>) {
        let picker_items: Vec<ListPickerItem> = agents
            .iter()
            .map(|a| ListPickerItem {
                label: a.display_name.clone(),
                subtitle: a.version.clone(),
                image_name: Some(a.icon_name.clone()),
            })
            .collect();

        let launch_targets: Vec<(String, Option<String>, String)> = agents
            .iter()
            .map(|a| {
                (
                    format!("Launching {}...", a.display_name),
                    a.launch_cmd.clone(),
                    a.slug.clone(),
                )
            })
            .collect();

        let pending = self.pending_action.clone();
        platform_bridge::show_list_picker(
            "Create Agent",
            "Launch an agent in a new terminal",
            picker_items,
            move |selection| {
                let Some(index) = selection else { return };
                let Some((initial_title, Some(cmd), agent_slug)) =
                    launch_targets.get(index).cloned()
                else {
                    return;
                };
                pending.set(PendingWorkspaceAction::SpawnAgentTerminal {
                    launch_cmd: cmd,
                    initial_title,
                    agent_slug,
                });
            },
        );
    }

    /// Picker with one "Dismiss" row carrying the reason in its subtitle.
    /// `show_list_picker` has no disabled-row affordance, so naming the
    /// label "Dismiss" keeps the tap unambiguous.
    fn present_placeholder(&self, message: &str) {
        let items = vec![ListPickerItem {
            label: "Dismiss".to_string(),
            subtitle: Some(message.to_string()),
            image_name: None,
        }];
        platform_bridge::show_list_picker(
            "Create Agent",
            "Agent management is unavailable right now.",
            items,
            move |_selection| {},
        );
    }
}
