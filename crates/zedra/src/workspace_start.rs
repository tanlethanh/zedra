use gpui::*;

use crate::platform_bridge::{self, HapticFeedback};
use crate::theme;
use crate::workspace_action;

/// Shown when a workspace has no active terminal: a flat, centered list of
/// entry actions. Replaces the old auto-create-terminal-on-connect flow.
pub struct WorkspaceStart;

struct WorkspaceStartItem {
    id: &'static str,
    icon: &'static str,
    icon_size: Pixels,
    label: &'static str,
    action: Box<dyn Action>,
}

impl WorkspaceStart {
    fn items() -> Vec<WorkspaceStartItem> {
        vec![
            WorkspaceStartItem {
                id: "workspace-start-create-agent",
                icon: "icons/plus.svg",
                icon_size: px(18.0),
                label: "Create Agent",
                action: workspace_action::CreateAgent.boxed_clone(),
            },
            WorkspaceStartItem {
                id: "workspace-start-new-terminal",
                icon: "icons/terminal.svg",
                icon_size: px(18.0),
                label: "New Terminal",
                action: workspace_action::CreateNewTerminal.boxed_clone(),
            },
            WorkspaceStartItem {
                id: "workspace-start-view-sessions",
                icon: "icons/history.svg",
                icon_size: px(16.0),
                label: "Resume Session",
                action: workspace_action::OpenAgentSessions.boxed_clone(),
            },
            WorkspaceStartItem {
                id: "workspace-start-manage-agents",
                icon: "icons/layers-2.svg",
                icon_size: px(16.0),
                label: "Manage Agents",
                action: workspace_action::OpenAgentManage.boxed_clone(),
            },
        ]
    }
}

impl Render for WorkspaceStart {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list = div()
            .flex()
            .flex_col()
            .items_start()
            .gap(px(theme::SPACING_XL))
            // Balance the optical center against the header above.
            .top(px(-64.0));

        for item in Self::items() {
            let action = item.action;
            list = list.child(
                div()
                    .id(item.id)
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(theme::SPACING_LG))
                    .cursor_pointer()
                    .hit_slop(px(theme::SPACING_SM))
                    .on_press(move |_event, window, cx| {
                        platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
                        window.dispatch_action(action.boxed_clone(), cx);
                    })
                    .child(
                        div()
                            .flex()
                            .w(px(20.0))
                            .h(px(20.0))
                            .items_center()
                            .justify_center()
                            .child(
                                svg()
                                    .path(item.icon)
                                    .size(item.icon_size)
                                    .text_color(rgb(theme::text_muted(cx))),
                            ),
                    )
                    .child(
                        div()
                            .text_color(rgb(theme::text_muted(cx)))
                            .text_size(px(theme::FONT_BODY))
                            .child(item.label),
                    ),
            );
        }

        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .child(list)
    }
}
