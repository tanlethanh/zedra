use gpui::*;

use crate::file_explorer::{FileExplorer, FileSelected};
use crate::editor::git_sidebar::{GitFileSelected, GitSidebar};
use crate::editor::git_sidebar::{GitFileEntry, GitFileStatus, GitRepoState};
use crate::pending::{shared_pending_slot, SharedPendingSlot};
use crate::theme;

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
    pending_git_status: SharedPendingSlot<GitRepoState>,
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
            pending_git_status: shared_pending_slot(),
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
        self.file_explorer.update(cx, |fe, cx| fe.reset_to_demo(cx));
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

                    pending.set(repo_state);
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
        crate::terminal_panel::render_terminal_tab(self.active_terminal_id.as_deref(), cx)
    }

    fn render_session_tab(&self, cx: &mut Context<Self>) -> Div {
        crate::session_panel::render_session_tab(cx)
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
        if let Some(state) = self.pending_git_status.take() {
            self.git_sidebar
                .update(cx, |sidebar, cx| sidebar.set_repo_state(state, cx));
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
            DrawerSection::Session => div()
                .id("session-scroll")
                .flex_1()
                .overflow_y_scroll()
                .child(self.render_session_tab(cx))
                .into_any_element(),
        };

        let top_inset = crate::platform_bridge::status_bar_inset();
        let bottom_inset = crate::platform_bridge::home_indicator_inset().max(10.0);

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
            // Footer nav bar — pt matches visual top padding; pb absorbs home indicator
            .child(
                div()
                    .flex()
                    .flex_row()
                    .pt(px(10.0))
                    .pb(px(bottom_inset))
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
