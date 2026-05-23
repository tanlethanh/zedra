use gpui::*;
use tracing::error;
use zedra_rpc::proto::InstalledAgentEntry;
use zedra_session::SessionHandle;

use crate::pending::SharedPendingSlot;
use crate::platform_bridge::{self, AlertButton, ListPickerItem};
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
        let task = cx.spawn(async move |this, cx| {
            match handle.agent_installed_list(false).await {
                Ok(agents) => {
                    let _ = this.update(cx, |this, cx| {
                        let available: Vec<_> =
                            agents.into_iter().filter(|a| a.available).collect();
                        if available.is_empty() {
                            this.state = State::Idle;
                            cx.defer(|_| {
                                platform_bridge::show_alert(
                                    "Create Agent",
                                    "No supported agent CLIs are installed on the host.",
                                    vec![AlertButton::default("OK")],
                                    |_| {},
                                );
                            });
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
                        platform_bridge::show_alert(
                            "Create Agent",
                            "Failed to load installed agents.",
                            vec![AlertButton::default("OK")],
                            |_| {},
                        );
                    });
                }
            }
        });
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

        let launch_cmds: Vec<Option<String>> =
            agents.iter().map(|a| a.launch_cmd.clone()).collect();

        let pending = self.pending_action.clone();
        platform_bridge::show_list_picker(
            "Create Agent",
            "Launch an agent in a new terminal",
            picker_items,
            move |selection| {
                let Some(index) = selection else { return };
                let Some(cmd) = launch_cmds.get(index).and_then(|c| c.clone()) else {
                    return;
                };
                pending.set(PendingWorkspaceAction::SpawnAgentTerminal { launch_cmd: cmd });
            },
        );
    }
}
