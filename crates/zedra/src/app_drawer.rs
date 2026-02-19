use std::sync::{Arc, Mutex};

use gpui::prelude::FluentBuilder;
use gpui::*;

use crate::file_explorer::{FileExplorer, FileSelected};
use crate::theme;
use crate::zedra_app::transport_badge_info;
use crate::git_sidebar::{GitFileSelected, GitSidebar};
use crate::git_stack::{GitFileEntry, GitFileStatus, GitRepoState};
use zedra_session::SessionState;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DrawerSection {
    Files,
    Git,
    Terminal,
    Session,
}

#[derive(Clone, Debug)]
pub enum AppDrawerEvent {
    FileSelected(String),
    GitFileSelected(String),
    CloseRequested,
    DisconnectRequested,
    NewTerminalRequested,
    TerminalSelected(String),
}

impl EventEmitter<AppDrawerEvent> for AppDrawer {}

pub struct AppDrawer {
    file_explorer: Entity<FileExplorer>,
    git_sidebar: Entity<GitSidebar>,
    active_section: DrawerSection,
    focus_handle: FocusHandle,
    pending_git_status: Arc<Mutex<Option<GitRepoState>>>,
    git_loaded: bool,
    active_terminal_id: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl AppDrawer {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let file_explorer = cx.new(|cx| FileExplorer::new(cx));
        let git_sidebar = cx.new(|cx| GitSidebar::new(cx));

        let mut subscriptions = Vec::new();

        let sub = cx.subscribe(
            &file_explorer,
            |_this: &mut Self, _emitter, event: &FileSelected, cx| {
                cx.emit(AppDrawerEvent::FileSelected(event.path.clone()));
            },
        );
        subscriptions.push(sub);

        let sub = cx.subscribe(
            &git_sidebar,
            |_this: &mut Self, _emitter, event: &GitFileSelected, cx| {
                cx.emit(AppDrawerEvent::GitFileSelected(event.path.clone()));
            },
        );
        subscriptions.push(sub);

        Self {
            file_explorer,
            git_sidebar,
            active_section: DrawerSection::Files,
            focus_handle: cx.focus_handle(),
            pending_git_status: Arc::new(Mutex::new(None)),
            git_loaded: false,
            active_terminal_id: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn set_section(&mut self, section: DrawerSection, cx: &mut Context<Self>) {
        self.active_section = section;
        if section == DrawerSection::Git {
            self.load_git_status();
        }
        cx.notify();
    }

    pub fn refresh_git(&mut self, _cx: &mut Context<Self>) {
        self.git_loaded = false;
        self.load_git_status();
    }

    /// Reset state after disconnect so next session triggers fresh loads
    pub fn reset_for_disconnect(&mut self, cx: &mut Context<Self>) {
        self.git_loaded = false;
        self.active_terminal_id = None;
        self.file_explorer
            .update(cx, |fe, cx| fe.reset_to_demo(cx));
    }

    /// Update the active terminal indicator in the Terminal tab.
    pub fn set_active_terminal(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        self.active_terminal_id = id;
        cx.notify();
    }

    fn load_git_status(&mut self) {
        if self.git_loaded {
            return;
        }
        let Some(session) = zedra_session::active_session() else {
            return;
        };
        self.git_loaded = true;
        let pending = self.pending_git_status.clone();
        zedra_session::session_runtime().spawn(async move {
            match session.git_status().await {
                Ok(result) => {
                    let mut staged = Vec::new();
                    let mut unstaged = Vec::new();
                    let mut untracked = Vec::new();

                    for entry in &result.entries {
                        let status = GitFileStatus::from_status_str(&entry.status);
                        let file = GitFileEntry::new(&entry.path, status, 0, 0);
                        match entry.status.as_str() {
                            "added" => staged.push(file),
                            "untracked" => untracked.push(file),
                            _ => unstaged.push(file),
                        }
                    }

                    let repo_state = GitRepoState {
                        branch: result.branch,
                        staged_files: staged,
                        unstaged_files: unstaged,
                        untracked_files: untracked,
                        commit_message: String::new(),
                    };

                    if let Ok(mut slot) = pending.lock() {
                        *slot = Some(repo_state);
                    }
                    zedra_session::signal_terminal_data();
                }
                Err(e) => {
                    log::error!("git_status RPC failed: {}", e);
                }
            }
        });
    }

    pub fn active_section(&self) -> DrawerSection {
        self.active_section
    }

    fn section_title(&self) -> &'static str {
        match self.active_section {
            DrawerSection::Files => "Files",
            DrawerSection::Git => "Source Control",
            DrawerSection::Terminal => "Terminal",
            DrawerSection::Session => "Session",
        }
    }

    fn nav_icon(
        &self,
        icon_path: &'static str,
        section: DrawerSection,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let is_active = self.active_section == section;
        let color = if is_active {
            rgb(theme::TEXT_PRIMARY)
        } else {
            rgb(theme::TEXT_MUTED)
        };

        div()
            .w(px(36.0))
            .h(px(36.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .cursor_pointer()
            .hover(|s| s.bg(theme::hover_bg()))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _event, _window, cx| {
                    if this.active_section == section {
                        cx.emit(AppDrawerEvent::CloseRequested);
                    } else {
                        this.set_section(section, cx);
                    }
                }),
            )
            .child(
                svg()
                    .path(icon_path)
                    .size(px(theme::ICON_NAV))
                    .text_color(color),
            )
    }

    fn render_terminal_tab(&self, cx: &mut Context<Self>) -> Div {
        let session = zedra_session::active_session();

        let Some(session) = session else {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(theme::TEXT_MUTED))
                .text_size(px(theme::FONT_BODY))
                .child("No active session");
        };

        let terminal_ids = session.terminal_ids();
        let active_id = self.active_terminal_id.clone();

        let mut content = div()
            .px(px(16.0))
            .pt(px(12.0))
            .flex()
            .flex_col()
            .flex_1();

        if terminal_ids.is_empty() {
            content = content.child(
                div()
                    .py(px(16.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(rgb(theme::TEXT_MUTED))
                    .text_size(px(theme::FONT_BODY))
                    .child("No terminals"),
            );
        } else {
            for (index, tid) in terminal_ids.iter().enumerate() {
                let is_active = active_id.as_deref() == Some(tid.as_str());
                let label = format!("Terminal {}", index + 1);
                let tid_clone = tid.clone();

                let row = div()
                    .id(SharedString::from(format!("term-row-{}", index)))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .py(px(8.0))
                    .px(px(8.0))
                    .rounded(px(6.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::hover_bg()))
                    .when(is_active, |s| s.bg(theme::hover_bg()))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |_this, _event, _window, cx| {
                            cx.emit(AppDrawerEvent::TerminalSelected(tid_clone.clone()));
                        }),
                    )
                    .child(
                        svg()
                            .path("icons/terminal.svg")
                            .size(px(theme::ICON_NAV))
                            .text_color(if is_active {
                                rgb(theme::TEXT_PRIMARY)
                            } else {
                                rgb(theme::TEXT_MUTED)
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(theme::FONT_BODY))
                            .text_color(if is_active {
                                rgb(theme::TEXT_PRIMARY)
                            } else {
                                rgb(theme::TEXT_SECONDARY)
                            })
                            .when(is_active, |s| s.font_weight(FontWeight::MEDIUM))
                            .child(label),
                    )
                    .when(is_active, |s| {
                        s.child(
                            div()
                                .w(px(theme::ICON_STATUS))
                                .h(px(theme::ICON_STATUS))
                                .rounded(px(3.0))
                                .bg(rgb(theme::ACCENT_GREEN)),
                        )
                    });

                content = content.child(row);

                // Separator between items
                if index < terminal_ids.len() - 1 {
                    content = content.child(
                        div()
                            .h(px(1.0))
                            .mx(px(8.0))
                            .bg(rgb(theme::BORDER_SUBTLE)),
                    );
                }
            }
        }

        // "New Terminal" button at the bottom
        let new_terminal_btn = div()
            .id("new-terminal-btn")
            .mx(px(16.0))
            .mt(px(16.0))
            .px(px(12.0))
            .py(px(8.0))
            .rounded(px(6.0))
            .border_1()
            .border_color(rgb(theme::BORDER_DEFAULT))
            .text_color(rgb(theme::TEXT_PRIMARY))
            .text_size(px(theme::FONT_BODY))
            .cursor_pointer()
            .hover(|s| s.bg(theme::hover_bg()))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_this, _event, _window, cx| {
                    cx.emit(AppDrawerEvent::NewTerminalRequested);
                }),
            )
            .child(div().flex().justify_center().child("+ New Terminal"));

        div()
            .flex_1()
            .flex()
            .flex_col()
            .child(content)
            .child(new_terminal_btn)
    }

    fn render_session_tab(&self, cx: &mut Context<Self>) -> Div {
        let session = zedra_session::active_session();

        let Some(session) = session else {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(theme::TEXT_MUTED))
                .text_size(px(theme::FONT_BODY))
                .child("No active session");
        };

        let state = session.state();
        let latency = session.latency_ms();
        let session_id = session
            .session_id()
            .unwrap_or_else(|| "—".to_string());

        let (status_label, status_color) = match &state {
            SessionState::Connected { .. } => ("Connected", theme::ACCENT_GREEN),
            SessionState::Connecting => ("Connecting...", theme::ACCENT_YELLOW),
            SessionState::Disconnected => ("Disconnected", theme::ACCENT_RED),
            SessionState::Error(_) => ("Error", theme::ACCENT_RED),
        };

        let (hostname, workdir) = match &state {
            SessionState::Connected { hostname, workdir } => {
                (hostname.clone(), workdir.clone())
            }
            _ => ("—".to_string(), "—".to_string()),
        };

        let transport_info = if matches!(&state, SessionState::Connected { .. }) {
            Some(transport_badge_info(latency))
        } else {
            None
        };

        let info_row =
            |label: &'static str, value: String| -> Div {
                div()
                    .py(px(6.0))
                    .child(
                        div()
                            .text_color(rgb(theme::TEXT_MUTED))
                            .text_size(px(theme::FONT_DETAIL))
                            .child(label),
                    )
                    .child(
                        div()
                            .mt(px(2.0))
                            .text_color(rgb(theme::TEXT_PRIMARY))
                            .text_size(px(theme::FONT_BODY))
                            .child(value),
                    )
            };

        let mut content = div()
            .px(px(16.0))
            .pt(px(12.0))
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .pb(px(10.0))
                    .border_b_1()
                    .border_color(rgb(theme::BORDER_SUBTLE))
                    .child(
                        div()
                            .w(px(theme::ICON_STATUS))
                            .h(px(theme::ICON_STATUS))
                            .rounded(px(3.0))
                            .bg(rgb(status_color)),
                    )
                    .child(
                        div()
                            .text_color(rgb(theme::TEXT_PRIMARY))
                            .text_size(px(theme::FONT_BODY))
                            .font_weight(FontWeight::MEDIUM)
                            .child(status_label),
                    ),
            )
            .child(info_row("Hostname", hostname))
            .child(info_row("Directory", workdir))
            .child(info_row("Session ID", session_id));

        if let Some((transport_label, dot_color)) = transport_info {
            content = content.child(
                div()
                    .py(px(6.0))
                    .child(
                        div()
                            .text_color(rgb(theme::TEXT_MUTED))
                            .text_size(px(theme::FONT_DETAIL))
                            .child("Transport"),
                    )
                    .child(
                        div()
                            .mt(px(2.0))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(4.0))
                            .child(
                                div()
                                    .w(px(theme::ICON_STATUS))
                                    .h(px(theme::ICON_STATUS))
                                    .rounded(px(3.0))
                                    .bg(rgb(dot_color)),
                            )
                            .child(
                                div()
                                    .text_color(rgb(dot_color))
                                    .text_size(px(theme::FONT_BODY))
                                    .child(transport_label),
                            ),
                    ),
            );
        }

        if let SessionState::Error(msg) = &state {
            content = content.child(
                div()
                    .pt(px(6.0))
                    .text_color(rgb(theme::ACCENT_RED))
                    .text_size(px(theme::FONT_BODY))
                    .child(msg.clone()),
            );
        }

        let disconnect_button = div()
            .id("session-disconnect-btn")
            .mx(px(16.0))
            .mt(px(16.0))
            .px(px(12.0))
            .py(px(8.0))
            .rounded(px(6.0))
            .border_1()
            .border_color(rgb(theme::ACCENT_RED))
            .text_color(rgb(theme::ACCENT_RED))
            .text_size(px(theme::FONT_BODY))
            .cursor_pointer()
            .hover(|s| s.bg(gpui::hsla(0.0, 0.6, 0.5, 0.1)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_this, _event, _window, cx| {
                    cx.emit(AppDrawerEvent::DisconnectRequested);
                }),
            )
            .child(div().flex().justify_center().child("Disconnect"));

        div()
            .flex_1()
            .flex()
            .flex_col()
            .child(content)
            .child(disconnect_button)
    }
}

impl Focusable for AppDrawer {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for AppDrawer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Check for pending git status from async RPC
        if let Ok(mut slot) = self.pending_git_status.lock() {
            if let Some(state) = slot.take() {
                self.git_sidebar
                    .update(cx, |sidebar, cx| sidebar.set_repo_state(state, cx));
            }
        }

        let title = self.section_title();
        let viewport_h = window.viewport_size().height;

        let tab_content: AnyElement = match self.active_section {
            DrawerSection::Files => div()
                .id("drawer-file-tree")
                .flex_1()
                .overflow_hidden()
                .child(self.file_explorer.clone())
                .into_any_element(),
            DrawerSection::Git => div()
                .flex_1()
                .overflow_hidden()
                .child(self.git_sidebar.clone())
                .into_any_element(),
            DrawerSection::Terminal => self.render_terminal_tab(cx).into_any_element(),
            DrawerSection::Session => self.render_session_tab(cx).into_any_element(),
        };

        let density = crate::android_jni::get_density();
        let top_inset = if density > 0.0 {
            crate::android_jni::get_system_inset_top() as f32 / density
        } else {
            0.0
        };

        div()
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .w_full()
            .h(viewport_h)
            .bg(rgb(theme::BG_PRIMARY))
            // Status bar spacer (separate from header to avoid h+pt conflict)
            .child(div().h(px(top_inset)))
            // Section header (fixed 48px, no padding)
            .child(
                div()
                    .h(px(48.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .px(px(16.0))
                    .border_b_1()
                    .border_color(rgb(theme::BORDER_SUBTLE))
                    .child(
                        div()
                            .text_color(rgb(theme::TEXT_SECONDARY))
                            .text_size(px(theme::FONT_HEADING))
                            .font_weight(FontWeight::MEDIUM)
                            .child(title),
                    ),
            )
            // Tab content
            .child(tab_content)
            // Footer nav bar — explicit py for balanced padding
            .child(
                div()
                    .flex()
                    .flex_row()
                    .py(px(10.0))
                    .justify_center()
                    .gap(px(36.0))
                    .border_t_1()
                    .border_color(rgb(theme::BORDER_SUBTLE))
                    .child(self.nav_icon("icons/folder.svg", DrawerSection::Files, cx))
                    .child(self.nav_icon("icons/git-branch.svg", DrawerSection::Git, cx))
                    .child(self.nav_icon("icons/terminal.svg", DrawerSection::Terminal, cx))
                    .child(self.nav_icon("icons/server.svg", DrawerSection::Session, cx)),
            )
    }
}
