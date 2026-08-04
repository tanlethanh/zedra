//! Open another project on an already-connected host: pick the machine, browse
//! its home directory, then have the host start a daemon for that folder and
//! pair with it. Hosts run `zedra start --detach --workdir <path>`, which the
//! screen shows verbatim so the effect on the desktop is never a surprise.

use futures::channel::oneshot;
use gpui::*;
use zedra_rpc::ZedraPairingTicket;
use zedra_rpc::proto::HostDirEntry;
use zedra_session::SessionHandle;
use zedra_telemetry::Event;

use crate::fonts;
use crate::platform_bridge::{self, AlertButton, HapticFeedback};
use crate::theme;
use crate::ui::{subscreen_empty_text, subscreen_padded_body};
use crate::workspaces::Workspaces;

#[derive(Clone, Debug)]
pub enum OpenProjectEvent {
    /// Leave the screen without opening anything.
    Close,
    /// A workspace was opened and connect started.
    NavigateToWorkspace,
}

impl EventEmitter<OpenProjectEvent> for OpenProjectView {}

/// Height reserved under the list for the floating action bar.
const ACTION_BAR_HEIGHT: f32 = 96.0;

struct Listing {
    path: String,
    display_path: String,
    parent: Option<String>,
    entries: Vec<HostDirEntry>,
    truncated: bool,
}

struct HostBrowse {
    hostname: String,
    session: SessionHandle,
    listing: Option<Listing>,
}

pub struct OpenProjectView {
    workspaces: Entity<Workspaces>,
    /// `None` while the host list is showing.
    browse: Option<HostBrowse>,
    loading: bool,
    opening: bool,
    error: Option<String>,
    _task: Option<Task<()>>,
}

impl OpenProjectView {
    pub fn new(workspaces: Entity<Workspaces>, _cx: &mut Context<Self>) -> Self {
        Self {
            workspaces,
            browse: None,
            loading: false,
            opening: false,
            error: None,
            _task: None,
        }
    }

    /// Enter the screen. A single connected host skips the host list.
    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.browse = None;
        self.loading = false;
        self.opening = false;
        self.error = None;
        self._task = None;

        let hosts = self.workspaces.read(cx).connected_hosts(cx);
        if hosts.len() == 1 {
            self.select_host(0, cx);
        }
        cx.notify();
    }

    fn select_host(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(host) = self
            .workspaces
            .read(cx)
            .connected_hosts(cx)
            .into_iter()
            .nth(index)
        else {
            return;
        };
        self.browse = Some(HostBrowse {
            hostname: host.hostname,
            session: host.session,
            listing: None,
        });
        self.load_dir(String::new(), cx);
    }

    /// Empty `path` lists the host user's home directory.
    fn load_dir(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(browse) = self.browse.as_ref() else {
            return;
        };
        let session = browse.session.clone();
        self.loading = true;
        self.error = None;
        cx.notify();

        self._task = Some(cx.spawn(async move |this, cx| {
            let result = session.host_dir_list(&path).await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok(result) => {
                        if let Some(browse) = this.browse.as_mut() {
                            browse.listing = Some(Listing {
                                path: result.path,
                                display_path: result.display_path,
                                parent: result.parent,
                                entries: result.entries,
                                truncated: result.truncated,
                            });
                        }
                    }
                    Err(e) => {
                        tracing::warn!("open-project: dir list failed: {e}");
                        this.error = Some(e.to_string());
                    }
                }
                cx.notify();
            });
        }));
    }

    fn confirm_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(browse), false) = (self.browse.as_ref(), self.opening) else {
            return;
        };
        let Some(listing) = browse.listing.as_ref() else {
            return;
        };
        let path = listing.path.clone();
        let display_path = listing.display_path.clone();
        let hostname = browse.hostname.clone();

        platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
        let (tx, rx) = oneshot::channel();
        platform_bridge::show_alert(
            "Open project",
            &format!(
                "{hostname} will run:\n\nzedra start --detach --workdir {display_path}\n\nThe daemon keeps running on the host until you stop it there."
            ),
            vec![AlertButton::default("Open"), AlertButton::cancel("Cancel")],
            move |index| {
                let _ = tx.send(index);
            },
        );
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(0) = rx.await {
                let _ = this.update_in(cx, |this, window, cx| this.open_workdir(path, window, cx));
            }
        })
        .detach();
    }

    fn open_workdir(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(browse) = self.browse.as_ref() else {
            return;
        };
        let session = browse.session.clone();
        self.opening = true;
        self.error = None;
        cx.notify();

        self._task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = session.host_workspace_open(&path).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.opening = false;
                match result.and_then(|result| {
                    ZedraPairingTicket::from_pairing_url(&result.pairing_url)
                        .map_err(|e| anyhow::anyhow!("host returned an unreadable ticket: {e}"))
                }) {
                    Ok(ticket) => this.connect_opened(ticket, window, cx),
                    Err(e) => {
                        tracing::warn!("open-project: open {path} failed: {e}");
                        this.error = Some(e.to_string());
                    }
                }
                cx.notify();
            });
        }));
    }

    /// Pairing from a remote open is the same handshake as a scanned QR.
    fn connect_opened(
        &mut self,
        ticket: ZedraPairingTicket,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        zedra_telemetry::send(Event::WorkspaceSelected {
            source: "open_project",
        });
        self.workspaces.update(cx, |workspaces, cx| {
            workspaces.connect_ticket(ticket, window, cx);
        });
        cx.emit(OpenProjectEvent::NavigateToWorkspace);
    }

    fn go_back(&mut self, cx: &mut Context<Self>) {
        platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
        if !self.step_back(cx) {
            cx.emit(OpenProjectEvent::Close);
        }
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
        cx.emit(OpenProjectEvent::Close);
    }

    /// One step up the browse stack: parent directory, then the host list.
    /// False when there is nowhere left to go and the screen should close.
    pub fn step_back(&mut self, cx: &mut Context<Self>) -> bool {
        let parent = self
            .browse
            .as_ref()
            .and_then(|browse| browse.listing.as_ref())
            .and_then(|listing| listing.parent.clone());
        if let Some(parent) = parent {
            self.load_dir(parent, cx);
            return true;
        }
        if self.browse.is_some() && self.workspaces.read(cx).connected_hosts(cx).len() > 1 {
            self.browse = None;
            self.error = None;
            cx.notify();
            return true;
        }
        false
    }
}

impl Render for OpenProjectView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Full-screen overlay: it owns both insets.
        let top_inset = platform_bridge::status_bar_inset();
        let bottom_inset = platform_bridge::home_indicator_inset();
        let hosts = self.workspaces.read(cx).connected_hosts(cx);
        let listing = self
            .browse
            .as_ref()
            .and_then(|browse| browse.listing.as_ref());
        let title = match listing {
            Some(listing) => listing.display_path.clone(),
            None => "Open project".to_string(),
        };
        let subtitle = match self.browse.as_ref() {
            Some(browse) => browse.hostname.clone(),
            None => "Pick a connected host".to_string(),
        };

        let body: AnyElement = if let Some(error) = self.error.clone() {
            subscreen_padded_body(subscreen_empty_text(error, cx)).into_any_element()
        } else if self.opening {
            subscreen_padded_body(subscreen_empty_text("Starting the host daemon\u{2026}", cx))
                .into_any_element()
        } else if self.browse.is_none() {
            host_list(&hosts, cx).into_any_element()
        } else {
            match listing.filter(|_| !self.loading) {
                Some(listing) => dir_body(listing, cx).into_any_element(),
                None => subscreen_padded_body(subscreen_empty_text("Loading\u{2026}", cx))
                    .into_any_element(),
            }
        };

        // Scroll content clears the floating bar so the last row stays tappable.
        let action_bar_space = if listing.is_some() && !self.opening {
            ACTION_BAR_HEIGHT + bottom_inset
        } else {
            0.0
        };

        let mut root = div()
            .id("open-project-root")
            .size_full()
            .min_h_0()
            .relative()
            .flex()
            .flex_col()
            .bg(rgb(theme::bg_primary(cx)))
            .child(div().h(px(top_inset)))
            .child(header(title, subtitle, cx))
            .child(
                div()
                    .id("open-project-scroll")
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .w_full()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .pb(px(action_bar_space))
                            .child(body),
                    ),
            );

        if let (Some(listing), false) = (listing, self.opening) {
            root = root.child(action_bar(listing.display_path.clone(), bottom_inset, cx));
        }
        root
    }
}

/// Floating footer: the command the host will run, plus the confirm action.
fn action_bar(
    display_path: String,
    bottom_inset: f32,
    cx: &mut Context<OpenProjectView>,
) -> impl IntoElement {
    div()
        .id("open-project-action-bar")
        .absolute()
        .bottom_0()
        .left_0()
        .right_0()
        .px(px(theme::SUBSCREEN_PADDING_X))
        .pt(px(theme::SPACING_SM))
        .pb(px(bottom_inset.max(theme::SPACING_LG)))
        .bg(rgb(theme::bg_surface(cx)))
        .border_t_1()
        .border_color(rgb(theme::border_subtle(cx)))
        .flex()
        .flex_col()
        .gap(px(theme::SPACING_SM))
        .child(
            div()
                .min_w_0()
                .whitespace_normal()
                .text_size(px(theme::FONT_DETAIL))
                .font_family(fonts::MONO_FONT_FAMILY)
                .text_color(rgb(theme::text_muted(cx)))
                .child(format!("zedra start --detach --workdir {display_path}")),
        )
        .child(
            crate::button::outline_button(cx, "open-project-open-here", "Open this folder")
                .on_press(cx.listener(|this, _event, window, cx| this.confirm_open(window, cx))),
        )
}

fn header(title: String, subtitle: String, cx: &mut Context<OpenProjectView>) -> impl IntoElement {
    div()
        .id("open-project-header")
        .w_full()
        .min_w_0()
        .px(px(theme::SUBSCREEN_PADDING_X))
        .pt(px(theme::SPACING_SM))
        .pb(px(theme::SPACING_SM))
        .border_b_1()
        .border_color(rgb(theme::border_subtle(cx)))
        .child(
            div()
                .min_w_0()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(theme::SPACING_MD))
                .child(back_button(cx))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .min_w_0()
                                .overflow_hidden()
                                .text_size(px(theme::FONT_HEADING))
                                .font_family(fonts::MONO_FONT_FAMILY)
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(theme::text_primary(cx)))
                                .child(title),
                        )
                        .child(
                            div()
                                .text_size(px(theme::FONT_DETAIL))
                                .text_color(rgb(theme::text_muted(cx)))
                                .child(subtitle),
                        ),
                )
                .child(close_button(cx)),
        )
}

/// Nudged left so the larger chevron's optical center still reads as the same
/// column as the folder icons below it.
fn back_button(cx: &mut Context<OpenProjectView>) -> impl IntoElement {
    div()
        .id("open-project-back-btn")
        .flex_shrink_0()
        .ml(px(-6.0))
        .cursor_pointer()
        .hit_slop(px(32.0))
        .on_press(cx.listener(|this, _event, _window, cx| this.go_back(cx)))
        .child(
            svg()
                .path("icons/chevron-left.svg")
                .size(px(theme::ICON_MD))
                .text_color(rgb(theme::text_muted(cx))),
        )
}

fn close_button(cx: &mut Context<OpenProjectView>) -> impl IntoElement {
    div()
        .id("open-project-close-btn")
        .flex_shrink_0()
        .cursor_pointer()
        .hit_slop(px(32.0))
        .on_press(cx.listener(|this, _event, _window, cx| this.close(cx)))
        .child(
            svg()
                .path("icons/x.svg")
                .size(px(theme::ICON_MD))
                .text_color(rgb(theme::text_muted(cx))),
        )
}

fn host_list(
    hosts: &[crate::workspaces::ConnectedHost],
    cx: &mut Context<OpenProjectView>,
) -> impl IntoElement {
    if hosts.is_empty() {
        return subscreen_padded_body(subscreen_empty_text(
            "No connected host. Connect a workspace first.",
            cx,
        ))
        .into_any_element();
    }

    let mut list = div()
        .id("open-project-hosts")
        .w_full()
        .min_w_0()
        .flex()
        .flex_col();
    for (index, host) in hosts.iter().enumerate() {
        let hostname = host.hostname.clone();
        list = list.child(
            row_shell(("open-project-host", index), cx)
                .on_press(cx.listener(move |this, _event, _window, cx| {
                    platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
                    this.select_host(index, cx);
                }))
                .child(row_icon("icons/server.svg", theme::text_muted(cx)))
                .child(row_label(hostname, cx))
                .child(row_chevron(cx)),
        );
    }
    subscreen_padded_body(list).into_any_element()
}

fn dir_body(listing: &Listing, cx: &mut Context<OpenProjectView>) -> impl IntoElement {
    let mut content = div()
        .id("open-project-dirs")
        .w_full()
        .min_w_0()
        .flex()
        .flex_col();

    if listing.entries.is_empty() {
        content = content.child(subscreen_empty_text("No sub-folders here", cx));
    }
    for (index, entry) in listing.entries.iter().enumerate() {
        let path = entry.path.clone();
        let name = entry.name.clone();
        let is_running = entry.is_running;
        content = content.child(
            row_shell(("open-project-dir", index), cx)
                .on_press(cx.listener(move |this, _event, _window, cx| {
                    platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
                    this.load_dir(path.clone(), cx);
                }))
                .child(row_icon("icons/folder.svg", theme::text_muted(cx)))
                .child(row_label(name, cx))
                .child(running_badge(is_running, cx))
                .child(row_chevron(cx)),
        );
    }
    if listing.truncated {
        content = content.child(subscreen_empty_text(
            "Showing the first 300 folders only",
            cx,
        ));
    }

    subscreen_padded_body(content)
}

fn row_shell(id: impl Into<ElementId>, cx: &mut Context<OpenProjectView>) -> Stateful<Div> {
    div()
        .id(id)
        .min_h(px(34.0))
        .py(px(2.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::SPACING_MD))
        .cursor_pointer()
        .border_b_1()
        .border_color(rgb(theme::border_subtle(cx)))
}

fn row_icon(path: &'static str, color: u32) -> impl IntoElement {
    svg()
        .path(path)
        .size(px(theme::ICON_SM))
        .flex_shrink_0()
        .text_color(rgb(color))
}

fn row_label(text: String, cx: &mut Context<OpenProjectView>) -> impl IntoElement {
    div()
        .flex_1()
        .min_w_0()
        .overflow_hidden()
        .text_size(px(theme::FONT_BODY))
        .text_color(rgb(theme::text_primary(cx)))
        .child(text)
}

fn running_badge(is_running: bool, cx: &mut Context<OpenProjectView>) -> impl IntoElement {
    div()
        .flex_shrink_0()
        .text_size(px(theme::FONT_DETAIL))
        .text_color(rgb(theme::text_muted(cx)))
        .child(if is_running { "running" } else { "" })
}

fn row_chevron(cx: &mut Context<OpenProjectView>) -> impl IntoElement {
    svg()
        .path("icons/chevron-right.svg")
        .size(px(theme::ICON_SM))
        .flex_shrink_0()
        .text_color(rgb(theme::text_muted(cx)))
}
