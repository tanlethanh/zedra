use gpui::*;

use crate::file_explorer::{FileExplorer, FileSelected};
use crate::theme;
use zedra_editor::{GitFileSelected, GitSidebar};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DrawerSection {
    Files,
    Git,
    Terminal,
    Packages,
}

#[derive(Clone, Debug)]
pub enum AppDrawerEvent {
    FileSelected(String),
    GitFileSelected(String),
    CloseRequested,
}

impl EventEmitter<AppDrawerEvent> for AppDrawer {}

pub struct AppDrawer {
    file_explorer: Entity<FileExplorer>,
    git_sidebar: Entity<GitSidebar>,
    active_section: DrawerSection,
    focus_handle: FocusHandle,
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
            _subscriptions: subscriptions,
        }
    }

    pub fn set_section(&mut self, section: DrawerSection, cx: &mut Context<Self>) {
        self.active_section = section;
        cx.notify();
    }

    pub fn active_section(&self) -> DrawerSection {
        self.active_section
    }

    fn section_title(&self) -> &'static str {
        match self.active_section {
            DrawerSection::Files => "Files",
            DrawerSection::Git => "Source Control",
            DrawerSection::Terminal => "Terminal",
            DrawerSection::Packages => "Packages",
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
                    .size(px(20.0))
                    .text_color(color),
            )
    }
}

impl Focusable for AppDrawer {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for AppDrawer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = self.section_title();
        let viewport_h = window.viewport_size().height;

        let tab_content: AnyElement = match self.active_section {
            DrawerSection::Files => div()
                .id("drawer-file-tree")
                .flex_1()
                .overflow_y_scroll()
                .child(self.file_explorer.clone())
                .into_any_element(),
            DrawerSection::Git => div()
                .flex_1()
                .child(self.git_sidebar.clone())
                .into_any_element(),
            DrawerSection::Terminal => div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(theme::TEXT_MUTED))
                .text_sm()
                .child("Terminal sessions")
                .into_any_element(),
            DrawerSection::Packages => div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(theme::TEXT_MUTED))
                .text_sm()
                .child("Package manager")
                .into_any_element(),
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
                            .text_sm()
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
                    .child(self.nav_icon("icons/cube.svg", DrawerSection::Packages, cx)),
            )
    }
}
