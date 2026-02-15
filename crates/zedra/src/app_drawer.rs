use gpui::*;

use crate::file_explorer::{FileExplorer, FileSelected};
use crate::theme;

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
    SectionChanged(DrawerSection),
}

impl EventEmitter<AppDrawerEvent> for AppDrawer {}

pub struct AppDrawer {
    file_explorer: Entity<FileExplorer>,
    active_section: DrawerSection,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl AppDrawer {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let file_explorer = cx.new(|cx| FileExplorer::new(cx));

        let mut subscriptions = Vec::new();
        let sub = cx.subscribe(
            &file_explorer,
            |_this: &mut Self, _emitter, event: &FileSelected, cx| {
                cx.emit(AppDrawerEvent::FileSelected(event.path.clone()));
            },
        );
        subscriptions.push(sub);

        Self {
            file_explorer,
            active_section: DrawerSection::Files,
            focus_handle: cx.focus_handle(),
            _subscriptions: subscriptions,
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
                    this.active_section = section;
                    cx.emit(AppDrawerEvent::SectionChanged(section));
                    cx.notify();
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(theme::BG_PRIMARY))
            // Header (88px): logo + project info
            .child(
                div()
                    .h(px(88.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .px(px(16.0))
                    .gap(px(12.0))
                    .child(
                        svg()
                            .path("icons/logo.svg")
                            .size(px(28.0))
                            .text_color(rgb(theme::TEXT_PRIMARY)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_color(rgb(theme::TEXT_PRIMARY))
                                    .text_sm()
                                    .font_weight(FontWeight::BOLD)
                                    .child("Zedra"),
                            )
                            .child(
                                div()
                                    .text_color(rgb(theme::TEXT_MUTED))
                                    .text_xs()
                                    .child("~/projects"),
                            ),
                    ),
            )
            // Separator
            .child(
                div()
                    .h(px(1.0))
                    .bg(rgb(theme::BORDER_SUBTLE)),
            )
            // File tree (scrollable middle)
            .child(
                div()
                    .id("drawer-file-tree")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(self.file_explorer.clone()),
            )
            // Separator
            .child(
                div()
                    .h(px(1.0))
                    .bg(rgb(theme::BORDER_SUBTLE)),
            )
            // Footer (88px): 4 nav icons
            .child(
                div()
                    .h(px(88.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_center()
                    .gap(px(36.0))
                    .child(self.nav_icon("icons/folder.svg", DrawerSection::Files, cx))
                    .child(self.nav_icon("icons/git-branch.svg", DrawerSection::Git, cx))
                    .child(self.nav_icon("icons/terminal.svg", DrawerSection::Terminal, cx))
                    .child(self.nav_icon("icons/cube.svg", DrawerSection::Packages, cx)),
            )
    }
}
