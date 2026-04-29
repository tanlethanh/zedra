use gpui::*;
use std::{collections::HashSet, f32::consts::TAU, time::Duration};
use tracing::*;

use zedra_rpc::proto::{FS_DOCS_TREE_DEFAULT_LIMIT, FsDocNode, FsDocsTreeError, FsDocsTreeResult};
use zedra_session::SessionHandle;

use crate::pending::{SharedPendingSlot, shared_pending_slot, spawn_periodic_task};
use crate::platform_bridge::{self, AlertButton, HapticFeedback};
use crate::theme;
use crate::workspace_action;
use crate::workspace_state::WorkspaceState;

const DOCS_TREE_BUILD_TIMEOUT: Duration = Duration::from_secs(20);
const DOCS_TREE_ROOT_PATH: &str = ".";
const DOCS_TREE_REBUILD_CONFIRM_POLL: Duration = Duration::from_millis(50);

#[derive(Clone)]
struct DocFlatRow {
    name: String,
    path: String,
    collapse_key: String,
    depth: usize,
    is_dir: bool,
    expanded: bool,
}

#[derive(Clone, Debug)]
enum DocsBuildState {
    NotBuilt,
    Building,
    Ready,
    Error(String),
}

pub struct DocsTree {
    root: Option<FsDocNode>,
    flat_rows: Vec<DocFlatRow>,
    flat_dirty: bool,
    collapsed_dirs: HashSet<String>,
    focus_handle: FocusHandle,
    scroll_handle: UniformListScrollHandle,
    selected_file_path: Option<String>,
    build_state: DocsBuildState,
    build_epoch: u64,
    workdir: String,
    snapshot_id: Option<String>,
    next_offset: u32,
    has_more: bool,
    loading_more: bool,
    pending_rebuild_confirmation: SharedPendingSlot<()>,
    _pending_rebuild_confirmation_task: Task<()>,
    workspace_state: Entity<WorkspaceState>,
    session_handle: SessionHandle,
}

impl DocsTree {
    pub fn new(
        workspace_state: Entity<WorkspaceState>,
        session_handle: SessionHandle,
        cx: &mut Context<Self>,
    ) -> Self {
        let pending_rebuild_confirmation = shared_pending_slot();
        let pending_rebuild_confirmation_task = {
            let pending = pending_rebuild_confirmation.clone();
            // Native alert callbacks are off-thread; poll back onto the GPUI entity.
            spawn_periodic_task(cx, DOCS_TREE_REBUILD_CONFIRM_POLL, move |this, cx| {
                if pending.take().is_some() {
                    this.rebuild(cx);
                }
            })
        };
        let (workdir, collapsed_dirs) = {
            let state = workspace_state.read(cx);
            (state.workdir.to_string(), persisted_collapsed_dirs(state))
        };
        Self {
            root: None,
            flat_rows: Vec::new(),
            flat_dirty: true,
            collapsed_dirs,
            focus_handle: cx.focus_handle(),
            scroll_handle: UniformListScrollHandle::new(),
            selected_file_path: None,
            build_state: DocsBuildState::NotBuilt,
            build_epoch: 0,
            workdir,
            snapshot_id: None,
            next_offset: 0,
            has_more: false,
            loading_more: false,
            pending_rebuild_confirmation,
            _pending_rebuild_confirmation_task: pending_rebuild_confirmation_task,
            workspace_state,
            session_handle,
        }
    }

    pub fn refresh_after_sync(&mut self, cx: &mut Context<Self>) {
        let next_workdir = self.workspace_state.read(cx).workdir.to_string();
        if self.workdir == next_workdir {
            return;
        }
        self.workdir = next_workdir;
        self.collapsed_dirs = persisted_collapsed_dirs(&self.workspace_state.read(cx));
        self.clear_tree();
        self.build_state = DocsBuildState::NotBuilt;
        self.build_epoch = self.build_epoch.wrapping_add(1);
        cx.notify();
    }

    pub fn ensure_built(&mut self, cx: &mut Context<Self>) {
        if matches!(self.build_state, DocsBuildState::NotBuilt) {
            self.rebuild(cx);
        }
    }

    fn clear_tree(&mut self) {
        self.root = None;
        self.flat_rows.clear();
        self.flat_dirty = true;
        self.selected_file_path = None;
        self.snapshot_id = None;
        self.next_offset = 0;
        self.has_more = false;
        self.loading_more = false;
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        if matches!(self.build_state, DocsBuildState::Building) {
            return;
        }

        self.workdir = self.workspace_state.read(cx).workdir.to_string();
        if self.root.is_none() {
            self.clear_tree();
        } else {
            self.loading_more = false;
        }
        self.build_state = DocsBuildState::Building;
        self.build_epoch = self.build_epoch.wrapping_add(1);
        let epoch = self.build_epoch;
        let handle = self.session_handle.clone();
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = request_docs_tree_page(handle, 0, true, None).await;
            let _ = this.update(cx, |this, cx| {
                if this.build_epoch != epoch {
                    return;
                }
                this.apply_docs_tree_result(result, true);
                cx.notify();
            });
        })
        .detach();
    }

    fn request_rebuild_confirmation(&self) {
        if matches!(self.build_state, DocsBuildState::Building) {
            return;
        }

        let pending = self.pending_rebuild_confirmation.clone();
        platform_bridge::show_alert(
            "Refresh documents?",
            "Scans Markdown files. Large workspaces may slow briefly.",
            vec![
                AlertButton::default("Refresh"),
                AlertButton::cancel("Cancel"),
            ],
            move |button_index| {
                if button_index == 0 {
                    pending.set(());
                }
            },
        );
    }

    fn load_more(&mut self, cx: &mut Context<Self>) {
        if self.loading_more
            || !self.has_more
            || matches!(self.build_state, DocsBuildState::Building)
        {
            return;
        }

        let Some(snapshot_id) = self.snapshot_id.clone() else {
            self.build_state =
                DocsBuildState::Error("Docs tree cache expired. Rebuild to refresh.".to_string());
            cx.notify();
            return;
        };

        self.loading_more = true;
        let epoch = self.build_epoch;
        let offset = self.next_offset;
        let handle = self.session_handle.clone();
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = request_docs_tree_page(handle, offset, false, Some(snapshot_id)).await;
            let _ = this.update(cx, |this, cx| {
                if this.build_epoch != epoch {
                    return;
                }
                this.apply_docs_tree_result(result, false);
                cx.notify();
            });
        })
        .detach();
    }

    fn apply_docs_tree_result(&mut self, result: anyhow::Result<FsDocsTreeResult>, rebuild: bool) {
        self.loading_more = false;
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                error!("docs tree request failed: {error}");
                if rebuild && self.root.is_some() {
                    self.build_state = DocsBuildState::Ready;
                    return;
                }
                if rebuild {
                    self.clear_tree();
                }
                self.build_state = DocsBuildState::Error(format!("Build failed: {error}"));
                return;
            }
        };

        if let Some(error) = result.error {
            if rebuild && self.root.is_some() {
                self.build_state = DocsBuildState::Ready;
                return;
            }
            if rebuild {
                self.clear_tree();
            }
            self.build_state = DocsBuildState::Error(docs_tree_error_message(error));
            return;
        }

        let Some(root) = result.root else {
            if rebuild && self.root.is_some() {
                self.build_state = DocsBuildState::Ready;
                return;
            }
            if rebuild {
                self.clear_tree();
            }
            self.build_state = DocsBuildState::Error("Docs tree returned no root".to_string());
            return;
        };

        if rebuild || self.root.is_none() {
            self.root = Some(root);
        } else if let Some(existing_root) = self.root.as_mut() {
            merge_doc_tree(existing_root, root);
        }

        self.snapshot_id = result.snapshot_id;
        self.next_offset = result.next_offset;
        self.has_more = result.has_more;
        self.flat_dirty = true;
        self.build_state = DocsBuildState::Ready;
    }

    fn toggle_dir(&mut self, key: &str, cx: &mut Context<Self>) {
        let key = key.to_string();
        let collapsed = if self.collapsed_dirs.remove(&key) {
            false
        } else {
            self.collapsed_dirs.insert(key.clone());
            true
        };
        self.workspace_state.update(cx, |state, cx| {
            state.set_docs_tree_dir_collapsed(key, collapsed, cx);
        });
        self.flat_dirty = true;
        cx.notify();
    }

    fn docs_count(&self) -> usize {
        self.root.as_ref().map(count_doc_files).unwrap_or(0)
    }

    fn status_message(&self) -> Option<String> {
        match &self.build_state {
            DocsBuildState::NotBuilt => Some("Docs tree not built".to_string()),
            DocsBuildState::Building => None,
            DocsBuildState::Ready => {
                if self.docs_count() == 0 {
                    Some("No markdown docs found".to_string())
                } else {
                    None
                }
            }
            DocsBuildState::Error(error) => Some(error.clone()),
        }
    }

    fn render_doc_row(
        &mut self,
        index: usize,
        row: DocFlatRow,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let indent = row.depth as f32 * 16.0;

        if row.is_dir {
            let collapse_key = row.collapse_key.clone();
            let (icon_path, icon_size) = if row.expanded {
                ("icons/folder-open.svg", px(theme::ICON_FILE_DIR))
            } else {
                ("icons/folder.svg", px(theme::ICON_FILE))
            };
            return div()
                .id(format!("docs-tree-dir-row-{index}"))
                .w_full()
                .h(px(theme::PANEL_ITEM_HEIGHT))
                .pl(px(theme::DRAWER_PADDING + indent))
                .pr(px(theme::DRAWER_PADDING))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(7.0))
                .cursor_pointer()
                .on_press(cx.listener(move |this, _event, _window, cx| {
                    this.toggle_dir(&collapse_key, cx);
                }))
                .child(
                    div().flex_shrink_0().child(
                        svg()
                            .path(icon_path)
                            .size(icon_size)
                            .text_color(rgb(theme::TEXT_MUTED)),
                    ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(rgb(theme::TEXT_SECONDARY))
                        .text_size(px(theme::FONT_BODY))
                        .child(row.name),
                )
                .into_any_element();
        }

        let path = row.path.clone();
        let is_selected = self
            .selected_file_path
            .as_deref()
            .is_some_and(|selected| selected == path);

        let mut row_el = div()
            .id(format!("docs-tree-file-row-{index}"))
            .w_full()
            .h(px(theme::PANEL_ITEM_HEIGHT))
            .pl(px(theme::DRAWER_PADDING + indent))
            .pr(px(theme::DRAWER_PADDING))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(7.0))
            .cursor_pointer()
            .on_press(cx.listener(move |this, _event, window, cx| {
                this.selected_file_path = Some(path.clone());
                window.dispatch_action(
                    workspace_action::OpenFile { path: path.clone() }.boxed_clone(),
                    cx,
                );
            }))
            .child(
                div().flex_shrink_0().child(
                    svg()
                        .path("icons/file-text.svg")
                        .size(px(theme::ICON_FILE))
                        .text_color(rgb(theme::TEXT_MUTED)),
                ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(rgb(theme::TEXT_SECONDARY))
                    .text_size(px(theme::FONT_BODY))
                    .child(row.name),
            );
        if is_selected {
            row_el = row_el.bg(hsla(0.0, 0.0, 1.0, 0.10));
        }
        row_el.into_any_element()
    }

    fn render_status_row(&self, message: String) -> Div {
        div()
            .min_h(px(96.0))
            .w_full()
            .flex()
            .items_center()
            .justify_center()
            .px(px(theme::DRAWER_PADDING))
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .text_color(rgb(theme::TEXT_MUTED))
                    .text_size(px(theme::FONT_BODY))
                    .text_center()
                    .child(message),
            )
    }

    fn render_load_more_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let label = if self.loading_more {
            "Loading more docs..."
        } else {
            "Load more docs"
        };

        div()
            .id("docs-tree-load-more-row")
            .flex_1()
            .min_w_0()
            .flex()
            .flex_row()
            .items_center()
            .h(px(theme::PANEL_ITEM_HEIGHT))
            .cursor_pointer()
            .on_press(cx.listener(|this, _event, _window, cx| {
                platform_bridge::trigger_haptic(HapticFeedback::ImpactLight);
                this.load_more(cx);
            }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(rgb(theme::TEXT_MUTED))
                    .text_size(px(theme::FONT_BODY))
                    .child(label),
            )
            .into_any_element()
    }

    fn render_all_loaded_row(&self) -> impl IntoElement {
        div()
            .id("docs-tree-all-loaded-row")
            .flex_1()
            .min_w_0()
            .flex()
            .items_center()
            .h(px(theme::PANEL_ITEM_HEIGHT))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(rgb(theme::TEXT_MUTED))
                    .text_size(px(theme::FONT_BODY))
                    .child("All loaded"),
            )
    }

    fn render_refresh_icon(&self, is_building: bool) -> AnyElement {
        let icon = svg()
            .path("icons/refresh-ccw.svg")
            .size(px(14.0))
            .text_color(rgb(theme::TEXT_MUTED));

        if !is_building {
            return icon.into_any_element();
        }

        // Repeat while rebuild is in flight; this icon is the progress affordance.
        icon.with_animation(
            ElementId::Name("docs-tree-refresh-spin".into()),
            Animation::new(Duration::from_millis(700)).repeat(),
            |icon, delta| icon.with_transformation(Transformation::rotate(radians(TAU * delta))),
        )
        .into_any_element()
    }

    fn render_refresh_button(&self, id: &'static str, cx: &mut Context<Self>) -> Stateful<Div> {
        let is_building = matches!(self.build_state, DocsBuildState::Building);
        let mut button = div()
            .id(id)
            .w(px(theme::PANEL_ITEM_HEIGHT))
            .h(px(theme::PANEL_ITEM_HEIGHT))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .opacity(if is_building { 0.45 } else { 1.0 })
            .child(self.render_refresh_icon(is_building));

        if !is_building {
            button = button
                .cursor_pointer()
                .on_pointer_down(|_, _, cx| {
                    cx.stop_propagation();
                })
                .on_press(cx.listener(|this, _event, _window, _cx| {
                    this.request_rebuild_confirmation();
                }));
        }

        button
    }

    fn render_empty_action_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut row = div()
            .id("docs-tree-empty-action-row")
            .w_full()
            .px(px(theme::DRAWER_PADDING))
            .py(px(8.0))
            .flex()
            .flex_row()
            .items_center()
            .justify_center()
            .gap(px(theme::SPACING_SM));

        if matches!(self.build_state, DocsBuildState::Building) {
            row = row.child(
                div()
                    .text_color(rgb(theme::TEXT_MUTED))
                    .text_size(px(theme::FONT_BODY))
                    .child("Building documents..."),
            );
        }

        row.child(self.render_refresh_button("docs-tree-empty-refresh-button", cx))
    }

    fn render_list_row(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if let Some(row) = self.flat_rows.get(index).cloned() {
            return self.render_doc_row(index, row, window, cx);
        }

        div()
            .id(format!("docs-tree-empty-list-row-{index}"))
            .w_full()
            .h(px(theme::PANEL_ITEM_HEIGHT))
            .into_any_element()
    }

    fn render_footer_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut footer = div()
            .id("docs-tree-footer")
            .w_full()
            .h(px(theme::PANEL_ITEM_HEIGHT))
            .px(px(theme::DRAWER_PADDING))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(theme::SPACING_SM));

        if self.has_more {
            footer = footer.child(self.render_load_more_row(cx));
        } else {
            footer = footer.child(self.render_all_loaded_row());
        }

        footer
            .child(self.render_refresh_button("docs-tree-refresh-button", cx))
            .into_any_element()
    }

    fn render_bottom_padding_row(&self) -> AnyElement {
        div()
            .id("docs-tree-bottom-padding-row")
            .w_full()
            .h(px(theme::PANEL_ITEM_HEIGHT))
            .into_any_element()
    }
}

impl Focusable for DocsTree {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for DocsTree {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.flat_dirty {
            self.flat_rows = self
                .root
                .as_ref()
                .map(|root| flatten_doc_tree(root, &self.collapsed_dirs, &self.workdir))
                .unwrap_or_default();
            self.flat_dirty = false;
        }

        if self.flat_rows.is_empty() {
            let status_message = self.status_message();
            let mut scroll_content = div()
                .id("docs-tree-scroll")
                .size_full()
                .flex()
                .flex_col()
                .items_stretch()
                .justify_center();
            if let Some(message) = status_message {
                scroll_content = scroll_content.child(self.render_status_row(message));
            }
            return div()
                .track_focus(&self.focus_handle)
                .id("docs-tree-container")
                .size_full()
                .flex()
                .flex_col()
                .min_h_0()
                .overflow_hidden()
                .child(scroll_content.child(self.render_empty_action_row(cx)));
        }

        let list_len = self.flat_rows.len() + 2;
        // Keep the compact footer and bottom padding in the list so they scroll with the docs rows.
        div()
            .track_focus(&self.focus_handle)
            .id("docs-tree-container")
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .overflow_hidden()
            .child(
                uniform_list(
                    "docs-tree-list",
                    list_len,
                    cx.processor(|this, range: std::ops::Range<usize>, window, cx| {
                        range
                            .map(|index| {
                                if index == this.flat_rows.len() {
                                    this.render_footer_row(cx)
                                } else if index == this.flat_rows.len() + 1 {
                                    this.render_bottom_padding_row()
                                } else {
                                    this.render_list_row(index, window, cx)
                                }
                            })
                            .collect::<Vec<_>>()
                    }),
                )
                .track_scroll(&self.scroll_handle)
                .size_full()
                .flex_grow(),
            )
    }
}

async fn request_docs_tree_page(
    handle: SessionHandle,
    offset: u32,
    rebuild: bool,
    snapshot_id: Option<String>,
) -> anyhow::Result<FsDocsTreeResult> {
    let (tx, rx) = futures::channel::oneshot::channel();
    zedra_session::session_runtime().spawn(async move {
        let result = tokio::time::timeout(
            DOCS_TREE_BUILD_TIMEOUT,
            handle.fs_docs_tree(
                DOCS_TREE_ROOT_PATH,
                offset,
                FS_DOCS_TREE_DEFAULT_LIMIT,
                rebuild,
                snapshot_id,
            ),
        )
        .await;
        let _ = tx.send(result);
    });

    match rx.await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err(anyhow::anyhow!("docs tree request timed out")),
        Err(_) => Err(anyhow::anyhow!("docs tree request task dropped")),
    }
}

fn docs_tree_error_message(error: FsDocsTreeError) -> String {
    match error {
        FsDocsTreeError::InvalidPath => "Docs tree path is invalid".to_string(),
        FsDocsTreeError::InvalidRequest(message) => format!("Docs tree request failed: {message}"),
        FsDocsTreeError::CacheMiss | FsDocsTreeError::StaleSnapshot => {
            "Docs tree cache expired. Rebuild to refresh.".to_string()
        }
        FsDocsTreeError::Busy => "Docs tree is already building".to_string(),
        FsDocsTreeError::ScanFailed(message) => format!("Build failed: {message}"),
        FsDocsTreeError::Unsupported => {
            "Docs tree requires a newer host. Update and restart the host.".to_string()
        }
    }
}

fn merge_doc_tree(existing: &mut FsDocNode, incoming: FsDocNode) {
    for incoming_child in incoming.children {
        merge_doc_node(existing, incoming_child);
    }
    sort_doc_node_children(existing);
}

fn merge_doc_node(parent: &mut FsDocNode, incoming: FsDocNode) {
    if let Some(existing) = parent
        .children
        .iter_mut()
        .find(|child| child.path == incoming.path && child.is_dir == incoming.is_dir)
    {
        if existing.is_dir {
            for child in incoming.children {
                merge_doc_node(existing, child);
            }
            sort_doc_node_children(existing);
        }
        return;
    }
    parent.children.push(incoming);
}

fn sort_doc_node_children(node: &mut FsDocNode) {
    for child in &mut node.children {
        sort_doc_node_children(child);
    }
    node.children.sort_by(|left, right| {
        right
            .is_dir
            .cmp(&left.is_dir)
            .then(left.name.cmp(&right.name))
    });
}

fn count_doc_files(node: &FsDocNode) -> usize {
    if !node.is_dir {
        return 1;
    }
    node.children.iter().map(count_doc_files).sum()
}

fn persisted_collapsed_dirs(state: &WorkspaceState) -> HashSet<String> {
    state.docs_tree_collapsed_dirs.iter().cloned().collect()
}

fn flatten_doc_tree(
    root: &FsDocNode,
    collapsed_dirs: &HashSet<String>,
    workdir: &str,
) -> Vec<DocFlatRow> {
    let mut rows = Vec::new();
    for child in &root.children {
        flatten_doc_node(child, 0, collapsed_dirs, workdir, &mut rows);
    }
    rows
}

fn flatten_doc_node(
    node: &FsDocNode,
    depth: usize,
    collapsed_dirs: &HashSet<String>,
    workdir: &str,
    rows: &mut Vec<DocFlatRow>,
) {
    if node.is_dir {
        let compact = compact_dir_chain(node);
        let collapse_key = docs_tree_collapse_key(&compact.path, workdir);
        let expanded = !collapsed_dirs.contains(&collapse_key);
        rows.push(DocFlatRow {
            name: doc_dir_display(node, compact),
            path: compact.path.clone(),
            collapse_key,
            depth,
            is_dir: true,
            expanded,
        });
        if !expanded {
            return;
        }
        for child in &compact.children {
            flatten_doc_node(child, depth + 1, collapsed_dirs, workdir, rows);
        }
        return;
    }

    rows.push(DocFlatRow {
        name: node.name.clone(),
        path: node.path.clone(),
        collapse_key: String::new(),
        depth,
        is_dir: false,
        expanded: false,
    });
}

fn compact_dir_chain(mut node: &FsDocNode) -> &FsDocNode {
    while node.is_dir && node.children.len() == 1 && node.children[0].is_dir {
        node = &node.children[0];
    }
    node
}

fn doc_dir_display(start: &FsDocNode, compact: &FsDocNode) -> String {
    if start.path == compact.path {
        return start.name.clone();
    }

    let start_path = normalize_display_separators(&start.path);
    let compact_path = normalize_display_separators(&compact.path);
    let prefix = format!("{}/", start_path.trim_end_matches('/'));
    let suffix = compact_path.strip_prefix(&prefix).unwrap_or(&compact.name);
    format!("{}/{}", start.name, suffix.trim_matches('/'))
}

fn normalize_display_separators(path: &str) -> String {
    path.replace('\\', "/")
}

fn docs_tree_collapse_key(path: &str, workdir: &str) -> String {
    // Collapse state is stored relative to the workspace so it survives host path changes.
    let path = normalize_display_separators(path);
    let workdir = normalize_display_separators(workdir)
        .trim_end_matches('/')
        .to_string();
    let key = if !workdir.is_empty() {
        if path == workdir {
            ".".to_string()
        } else {
            let prefix = format!("{workdir}/");
            path.strip_prefix(&prefix)
                .unwrap_or(&path)
                .trim_start_matches('/')
                .to_string()
        }
    } else {
        path.trim_start_matches("./")
            .trim_start_matches('/')
            .to_string()
    };
    let key = key.trim_matches('/').to_string();
    if key.is_empty() { ".".to_string() } else { key }
}

#[cfg(test)]
mod tests {
    use super::{
        count_doc_files, doc_dir_display, docs_tree_collapse_key, docs_tree_error_message,
        flatten_doc_tree, merge_doc_tree, request_docs_tree_page,
    };
    use std::collections::HashSet;
    use zedra_rpc::proto::{FsDocNode, FsDocsTreeError};
    use zedra_session::SessionHandle;

    fn dir(name: &str, path: &str, children: Vec<FsDocNode>) -> FsDocNode {
        FsDocNode {
            name: name.to_string(),
            path: path.to_string(),
            is_dir: true,
            size: 0,
            children,
        }
    }

    fn file(name: &str, path: &str) -> FsDocNode {
        FsDocNode {
            name: name.to_string(),
            path: path.to_string(),
            is_dir: false,
            size: 1,
            children: Vec::new(),
        }
    }

    #[test]
    fn docs_tree_compacts_single_child_directory_chains() {
        let root = dir(
            "repo",
            "/repo",
            vec![dir(
                "vendor",
                "/repo/vendor",
                vec![dir(
                    "zed",
                    "/repo/vendor/zed",
                    vec![dir(
                        "docs",
                        "/repo/vendor/zed/docs",
                        vec![file("guide.md", "/repo/vendor/zed/docs/guide.md")],
                    )],
                )],
            )],
        );

        let rows = flatten_doc_tree(&root, &HashSet::new(), "/repo");

        assert_eq!(rows[0].name, "vendor/zed/docs");
        assert!(rows[0].is_dir);
        assert_eq!(rows[1].name, "guide.md");
        assert_eq!(rows[1].depth, 1);
    }

    #[test]
    fn docs_tree_nested_directory_labels_do_not_repeat_parent_path() {
        let root = dir(
            "repo",
            "/repo",
            vec![dir(
                "crates",
                "/repo/crates",
                vec![
                    dir(
                        "zedra",
                        "/repo/crates/zedra",
                        vec![file("README.md", "/repo/crates/zedra/README.md")],
                    ),
                    file("README.md", "/repo/crates/README.md"),
                ],
            )],
        );

        let rows = flatten_doc_tree(&root, &HashSet::new(), "/repo");

        assert_eq!(rows[0].name, "crates");
        assert_eq!(rows[1].name, "zedra");
        assert_eq!(rows[2].name, "README.md");
        assert_eq!(rows[3].name, "README.md");
    }

    #[test]
    fn docs_tree_collapses_directory_subtrees_by_path() {
        let root = dir(
            "repo",
            "/repo",
            vec![dir(
                "crates",
                "/repo/crates",
                vec![
                    dir(
                        "zedra",
                        "/repo/crates/zedra",
                        vec![file("README.md", "/repo/crates/zedra/README.md")],
                    ),
                    file("README.md", "/repo/crates/README.md"),
                ],
            )],
        );
        let collapsed = HashSet::from(["crates".to_string()]);

        let rows = flatten_doc_tree(&root, &collapsed, "/repo");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "crates");
        assert!(!rows[0].expanded);
    }

    #[test]
    fn docs_tree_collapse_keys_are_workspace_relative() {
        assert_eq!(
            docs_tree_collapse_key("/repo/crates/zedra", "/repo"),
            "crates/zedra"
        );
        assert_eq!(
            docs_tree_collapse_key(r"C:\repo\vendor\zed\docs", r"C:\repo"),
            "vendor/zed/docs"
        );
    }

    #[test]
    fn docs_tree_merge_adds_new_page_nodes_without_duplicates() {
        let mut existing = dir(
            "repo",
            "/repo",
            vec![dir(
                "docs",
                "/repo/docs",
                vec![file("one.md", "/repo/docs/one.md")],
            )],
        );
        let incoming = dir(
            "repo",
            "/repo",
            vec![dir(
                "docs",
                "/repo/docs",
                vec![
                    file("one.md", "/repo/docs/one.md"),
                    file("two.md", "/repo/docs/two.md"),
                ],
            )],
        );

        merge_doc_tree(&mut existing, incoming);

        assert_eq!(count_doc_files(&existing), 2);
        assert_eq!(existing.children[0].children[0].name, "one.md");
        assert_eq!(existing.children[0].children[1].name, "two.md");
    }

    #[test]
    fn doc_dir_display_compacts_nested_paths_without_trailing_slash() {
        let start = dir("vendor", "/repo/vendor", Vec::new());
        let compact = dir("docs", "/repo/vendor/zed/docs", Vec::new());
        assert_eq!(doc_dir_display(&start, &compact), "vendor/zed/docs");
        assert_eq!(doc_dir_display(&start, &start), "vendor");
    }

    #[test]
    fn docs_tree_maps_cache_and_unsupported_errors_to_rebuildable_messages() {
        assert_eq!(
            docs_tree_error_message(FsDocsTreeError::CacheMiss),
            "Docs tree cache expired. Rebuild to refresh."
        );
        assert_eq!(
            docs_tree_error_message(FsDocsTreeError::StaleSnapshot),
            "Docs tree cache expired. Rebuild to refresh."
        );
        assert!(
            docs_tree_error_message(FsDocsTreeError::Unsupported).contains("newer host"),
            "unsupported hosts should not leave the UI in a building state"
        );
    }

    #[test]
    fn docs_tree_request_helper_does_not_require_caller_tokio_runtime() {
        let result = futures::executor::block_on(request_docs_tree_page(
            SessionHandle::new(),
            0,
            true,
            None,
        ));

        assert!(result.unwrap_err().to_string().contains("not connected"));
    }
}
