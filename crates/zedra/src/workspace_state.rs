use gpui::{Context, EventEmitter};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use tracing::*;
use uuid::Uuid;
use zedra_rpc::proto::{AgentState, HostInfoSnapshot, WebClientInfo, WebClientUpdate};

use zedra_session::*;

use crate::platform_bridge;

const STORE_DIR: &str = "zedra";
const STORE_FILE: &str = "workspaces.json";

#[derive(Clone, Default, Serialize, Deserialize)]
struct WorkspaceStore {
    workspaces: Vec<WorkspaceState>,
}

pub enum WorkspaceStateEvent {
    StateChanged,
    SyncComplete,
    HostInfoChanged,
    TerminalCreated { id: String },
    TerminalOpened { id: String },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum WorkspaceMainView {
    #[default]
    Default,
    File {
        path: String,
    },
    GitDiff {
        path: String,
        section: u8,
    },
    Terminal {
        id: String,
    },
    AgentSessions,
    AgentManage,
    AgentDetail {
        slug: String,
    },
}

impl WorkspaceMainView {
    pub fn file_path(&self) -> Option<&str> {
        match self {
            Self::File { path } => Some(path),
            _ => None,
        }
    }

    pub fn is_file_path(&self, path: &str) -> bool {
        self.file_path().is_some_and(|active| active == path)
    }

    pub fn git_diff(&self) -> Option<(&str, u8)> {
        match self {
            Self::GitDiff { path, section } => Some((path, *section)),
            _ => None,
        }
    }

    pub fn is_git_diff(&self, path: &str, section: u8) -> bool {
        self.git_diff()
            .is_some_and(|(active_path, active_section)| {
                active_path == path && active_section == section
            })
    }

    pub fn terminal_id(&self) -> Option<&str> {
        match self {
            Self::Terminal { id } => Some(id),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkspaceNavigationStack {
    routes: Vec<WorkspaceMainView>,
}

impl WorkspaceNavigationStack {
    pub fn active(&self) -> WorkspaceMainView {
        self.routes.last().cloned().unwrap_or_default()
    }

    pub fn open(&mut self, route: WorkspaceMainView) {
        if self.routes.last() == Some(&route) {
            return;
        }

        self.routes.retain(|entry| entry != &route);
        self.routes.push(route);
    }

    pub fn replace(&mut self, route: WorkspaceMainView) {
        if !self.routes.is_empty() {
            self.routes.pop();
        }
        self.routes.retain(|entry| entry != &route);
        self.routes.push(route);
    }

    pub fn reset(&mut self, route: WorkspaceMainView) {
        self.routes.clear();
        self.routes.push(route);
    }

    pub fn go_back(&mut self) -> Option<WorkspaceMainView> {
        if self.routes.len() <= 1 {
            return None;
        }

        self.routes.pop();
        self.routes.last().cloned()
    }

    pub fn prune_stale_terminals(&mut self, terminal_ids: &[String]) {
        self.routes.retain(|route| {
            route
                .terminal_id()
                .is_none_or(|id| terminal_ids.iter().any(|terminal_id| terminal_id == id))
        });
    }

    pub fn remove_terminal(&mut self, id: &str) {
        self.routes.retain(|route| route.terminal_id() != Some(id));
    }
}

/// A web tunnel opened for this workspace, tracked so the user can reopen it
/// from the session panel. Persisted across app restarts and reconnects.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackedTunnel {
    /// The URL to open (e.g. `http://localhost:5173`).
    pub url: String,
    /// Short label shown in the list (`host:port`).
    pub title: String,
    /// Unix seconds of the last open, for most-recent-first ordering.
    pub last_opened_at: u64,
}

/// Insert or update a tracked tunnel, preserving list position: an existing
/// entry (matched by url) is updated in place, a new one is appended.
fn upsert_web_tunnel(tunnels: &mut Vec<TrackedTunnel>, url: &str, title: &str, now: u64) {
    if let Some(existing) = tunnels.iter_mut().find(|t| t.url == url) {
        existing.title = title.to_string();
        existing.last_opened_at = now;
    } else {
        tunnels.push(TrackedTunnel {
            url: url.to_string(),
            title: title.to_string(),
            last_opened_at: now,
        });
    }
}

fn web_client_card(info: WebClientInfo) -> WebClientCard {
    WebClientCard {
        id: info.id,
        slug: info.slug,
        port: info.port,
        title: info.title,
        state: info.state,
        path: info.path,
    }
}

fn replace_web_client_cards(cards: &mut Vec<WebClientCard>, clients: Vec<WebClientInfo>) {
    *cards = clients.into_iter().map(web_client_card).collect();
}

/// A host-managed agent web-client server (e.g. `opencode serve`) shown as a
/// card. Runtime-only: rebuilt from the host's `WebClientWatch` stream on every
/// connect. Icon and display name resolve from `slug`.
#[derive(Clone, Debug, PartialEq)]
pub struct WebClientCard {
    pub id: String,
    pub slug: String,
    /// Host loopback port; the app tunnels `http://localhost:<port>`.
    pub port: u16,
    pub title: Option<String>,
    pub state: AgentState,
    /// URL path to open on the server, tracking where the user last navigated.
    /// Host-held, so it survives reconnects.
    pub path: String,
}

/// Shareable workspace state. Clone copies the Arc only. Read via methods (non-blocking).
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct WorkspaceState {
    pub endpoint_addr: String,
    pub session_id: String,
    pub strip_path: String,
    pub project_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_name: Option<String>,
    pub workdir: String,
    pub homedir: String,
    pub hostname: String,
    // Workspace-relative docs tree directories hidden by the user.
    #[serde(default)]
    pub docs_tree_collapsed_dirs: Vec<String>,
    // Web tunnels opened for this workspace, in stable open order for quick reopen.
    #[serde(default)]
    pub web_tunnels: Vec<TrackedTunnel>,
    #[serde(default)]
    pub delta_host_pubkey: Option<[u8; 32]>,
    #[serde(default)]
    pub delta_host_node_id: Option<Uuid>,
    pub created_at: u64,
    pub updated_at: u64,

    #[serde(skip)]
    pub connect_phase: Option<ConnectPhase>,
    #[serde(skip)]
    pub active_terminal_id: Option<String>,
    #[serde(skip)]
    pub active_main_view: WorkspaceMainView,
    #[serde(skip)]
    pub main_view_stack: WorkspaceNavigationStack,
    #[serde(skip)]
    pub terminal_ids: Vec<String>,
    #[serde(skip)]
    pub host_info: Option<HostInfoSnapshot>,
    // Host-managed web-client servers (e.g. `opencode serve`), live from the
    // host's `WebClientWatch` stream. Rebuilt per connect, so not persisted.
    #[serde(skip)]
    pub web_clients: Vec<WebClientCard>,
}

#[derive(Clone, PartialEq)]
struct WorkspaceStateSyncSnapshot {
    session_id: String,
    strip_path: String,
    project_name: String,
    workdir: String,
    homedir: String,
    hostname: String,
    connect_phase: Option<ConnectPhase>,
    active_terminal_id: Option<String>,
    terminal_ids: Vec<String>,
    host_info: Option<HostInfoSnapshot>,
    web_clients: Vec<WebClientCard>,
    delta_host_pubkey: Option<[u8; 32]>,
    delta_host_node_id: Option<Uuid>,
}

/// PartialEq implementation for WorkspaceState.
/// Compare all durable fields to prevent unnecessary updates.
impl PartialEq for WorkspaceState {
    fn eq(&self, other: &Self) -> bool {
        self.endpoint_addr == other.endpoint_addr
            && self.session_id == other.session_id
            && self.strip_path == other.strip_path
            && self.project_name == other.project_name
            && self.custom_name == other.custom_name
            && self.workdir == other.workdir
            && self.homedir == other.homedir
            && self.hostname == other.hostname
            && self.docs_tree_collapsed_dirs == other.docs_tree_collapsed_dirs
            && self.web_tunnels == other.web_tunnels
            && self.delta_host_pubkey == other.delta_host_pubkey
            && self.delta_host_node_id == other.delta_host_node_id
            && self.created_at == other.created_at
            && self.updated_at == other.updated_at
    }
}

static WORKSPACE_STORE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn workspace_store_lock() -> &'static Mutex<()> {
    WORKSPACE_STORE_LOCK.get_or_init(|| Mutex::new(()))
}

#[cfg(test)]
static TEST_DATA_DIRECTORY: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

#[cfg(test)]
fn test_data_directory() -> &'static Mutex<Option<PathBuf>> {
    TEST_DATA_DIRECTORY.get_or_init(|| Mutex::new(None))
}

fn data_directory() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(dir) = test_data_directory().lock().ok().and_then(|g| g.clone()) {
        return Some(dir);
    }

    platform_bridge::bridge()
        .data_directory()
        .map(PathBuf::from)
}

fn store_path() -> Option<PathBuf> {
    let dir = data_directory()?.join(STORE_DIR);
    if !dir.exists() {
        if let Err(e) = std::fs::create_dir_all(&dir) {
            error!(dir = ?dir, err = %e, "Failed to create directory: {e}");
            return None;
        }
    }
    Some(dir.join(STORE_FILE))
}

impl WorkspaceState {
    fn sync_snapshot(&self) -> WorkspaceStateSyncSnapshot {
        WorkspaceStateSyncSnapshot {
            session_id: self.session_id.clone(),
            strip_path: self.strip_path.clone(),
            project_name: self.project_name.clone(),
            workdir: self.workdir.clone(),
            homedir: self.homedir.clone(),
            hostname: self.hostname.clone(),
            connect_phase: self.connect_phase.clone(),
            active_terminal_id: self.active_terminal_id.clone(),
            terminal_ids: self.terminal_ids.clone(),
            host_info: self.host_info.clone(),
            web_clients: self.web_clients.clone(),
            delta_host_pubkey: self.delta_host_pubkey,
            delta_host_node_id: self.delta_host_node_id,
        }
    }

    fn clear_runtime_state_for_disconnect(&mut self) {
        self.connect_phase = Some(ConnectPhase::Disconnected);
        self.active_terminal_id = None;
        self.active_main_view = WorkspaceMainView::Default;
        self.main_view_stack.reset(WorkspaceMainView::Default);
        self.terminal_ids.clear();
        self.host_info = None;
        self.web_clients.clear();
    }

    pub fn mark_disconnected(&mut self, cx: &mut Context<Self>) {
        self.clear_runtime_state_for_disconnect();

        cx.emit(WorkspaceStateEvent::StateChanged);
        cx.notify();
    }

    /// The name shown in the UI. Returns `custom_name` if set, otherwise `project_name`.
    pub fn display_name(&self) -> &str {
        self.custom_name
            .as_deref()
            .unwrap_or(self.project_name.as_str())
    }

    pub fn set_custom_name(&mut self, name: Option<String>, cx: &mut Context<Self>) {
        if self.custom_name == name {
            return;
        }
        self.custom_name = name;
        cx.emit(WorkspaceStateEvent::StateChanged);
        cx.notify();
    }

    pub fn sync_from_session(
        &mut self,
        session_handle: &SessionHandle,
        session_state: &SessionState,
        cx: &mut Context<Self>,
    ) {
        if !self.sync_fields_from_session(session_handle, session_state) {
            return;
        }

        cx.emit(WorkspaceStateEvent::StateChanged);
        cx.notify();
    }

    fn sync_fields_from_session(
        &mut self,
        session_handle: &SessionHandle,
        session_state: &SessionState,
    ) -> bool {
        let before = self.sync_snapshot();
        let session_id = session_state.snapshot.session_id.clone();
        self.connect_phase = Some(session_state.phase.clone());
        if matches!(
            session_state.phase,
            ConnectPhase::Disconnected
                | ConnectPhase::Reconnecting { .. }
                | ConnectPhase::Failed(_)
        ) {
            self.web_clients.clear();
        }
        self.terminal_ids = session_handle.terminal_ids().clone();
        if !matches!(
            session_state.phase,
            ConnectPhase::Connected | ConnectPhase::Idle { .. }
        ) {
            self.host_info = None;
        }

        let snap = &session_state.snapshot;
        if !snap.hostname.is_empty() {
            self.hostname = snap.hostname.clone();
        }
        if !snap.workdir.is_empty() {
            self.workdir = snap.workdir.clone();
        }
        if !snap.project_name.is_empty() {
            self.project_name = snap.project_name.clone();
        }
        if !snap.strip_path.is_empty() {
            self.strip_path = snap.strip_path.clone();
        }
        if !snap.homedir.is_empty() {
            self.homedir = snap.homedir.clone();
        }
        if let Some(session_id) = session_id {
            self.session_id = session_id.clone();
        }

        self.sync_snapshot() != before
    }

    pub fn emit_sync_complete(&self, cx: &mut Context<Self>) {
        cx.emit(WorkspaceStateEvent::SyncComplete);
    }

    pub fn update_host_info(&mut self, host_info: HostInfoSnapshot, cx: &mut Context<Self>) {
        self.host_info = Some(host_info);
        cx.emit(WorkspaceStateEvent::HostInfoChanged);
    }

    pub fn set_delta_host_pubkey(&mut self, delta_host_pubkey: [u8; 32], cx: &mut Context<Self>) {
        if self.delta_host_pubkey == Some(delta_host_pubkey) {
            return;
        }
        self.delta_host_pubkey = Some(delta_host_pubkey);
        self.delta_host_node_id = None;
        cx.emit(WorkspaceStateEvent::StateChanged);
        cx.notify();
    }

    pub fn set_delta_host_node_id(&mut self, delta_host_node_id: Uuid, cx: &mut Context<Self>) {
        if self.delta_host_node_id == Some(delta_host_node_id) {
            return;
        }
        self.delta_host_node_id = Some(delta_host_node_id);
        cx.emit(WorkspaceStateEvent::StateChanged);
        cx.notify();
    }

    pub fn set_active_main_view(
        &mut self,
        active_main_view: WorkspaceMainView,
        cx: &mut Context<Self>,
    ) {
        if self.active_main_view == active_main_view {
            return;
        }

        self.active_main_view = active_main_view;
        cx.notify();
    }

    pub fn active_main_view_terminal_id(&self) -> Option<&str> {
        self.active_main_view.terminal_id()
    }

    /// Drop back to the workspace start view. `Default` is the base of the
    /// navigation stack, never a pushed route, so this clears the stack rather
    /// than navigating onto it.
    pub fn reset_to_default(&mut self, cx: &mut Context<Self>) {
        self.main_view_stack.reset(WorkspaceMainView::Default);
        self.set_active_main_view(WorkspaceMainView::Default, cx);
    }

    pub fn navigate(&mut self, route: WorkspaceMainView, cx: &mut Context<Self>) {
        self.main_view_stack.open(route.clone());
        if let WorkspaceMainView::Terminal { ref id } = route {
            self.active_terminal_id = Some(id.clone());
            cx.emit(WorkspaceStateEvent::TerminalOpened { id: id.clone() });
        }
        self.set_active_main_view(route, cx);
    }

    pub fn replace_current_route(&mut self, route: WorkspaceMainView, cx: &mut Context<Self>) {
        self.main_view_stack.replace(route.clone());
        if let WorkspaceMainView::Terminal { ref id } = route {
            self.active_terminal_id = Some(id.clone());
            cx.emit(WorkspaceStateEvent::TerminalOpened { id: id.clone() });
        }
        self.set_active_main_view(route, cx);
    }

    pub fn remove_terminal_route(&mut self, id: &str, cx: &mut Context<Self>) {
        self.main_view_stack.remove_terminal(id);
        cx.notify();
    }

    pub fn go_back(&mut self, cx: &mut Context<Self>) -> Option<WorkspaceMainView> {
        let route = self.main_view_stack.go_back()?;
        if let WorkspaceMainView::Terminal { ref id } = route {
            self.active_terminal_id = Some(id.clone());
            cx.emit(WorkspaceStateEvent::TerminalOpened { id: id.clone() });
        }
        self.set_active_main_view(route.clone(), cx);
        Some(route)
    }

    pub fn set_docs_tree_dir_collapsed(
        &mut self,
        path: String,
        collapsed: bool,
        cx: &mut Context<Self>,
    ) {
        if collapsed {
            if self.docs_tree_collapsed_dirs.iter().any(|p| p == &path) {
                return;
            }
            self.docs_tree_collapsed_dirs.push(path);
            self.docs_tree_collapsed_dirs.sort();
        } else {
            let before = self.docs_tree_collapsed_dirs.len();
            self.docs_tree_collapsed_dirs.retain(|p| p != &path);
            if self.docs_tree_collapsed_dirs.len() == before {
                return;
            }
        }

        cx.emit(WorkspaceStateEvent::StateChanged);
        cx.notify();
    }

    /// Track a web tunnel opened for this workspace. Position is stable: an
    /// existing entry is updated in place, a new one appended — so reopening a
    /// tunnel never reorders the list.
    pub fn record_web_tunnel(&mut self, url: &str, title: &str, cx: &mut Context<Self>) {
        upsert_web_tunnel(&mut self.web_tunnels, url, title, Self::now_u64());
        cx.emit(WorkspaceStateEvent::StateChanged);
        cx.notify();
    }

    /// Apply a live web-client update from the host's `WebClientWatch` stream:
    /// remove on close, otherwise upsert by id (preserving list order).
    pub fn apply_web_client_update(&mut self, update: WebClientUpdate, cx: &mut Context<Self>) {
        if update.closed {
            self.web_clients.retain(|card| card.id != update.id);
        } else if let Some(card) = self.web_clients.iter_mut().find(|c| c.id == update.id) {
            card.slug = update.slug;
            card.port = update.port;
            card.title = update.title;
            card.state = update.state;
            card.path = update.path;
        } else {
            self.web_clients.push(WebClientCard {
                id: update.id,
                slug: update.slug,
                port: update.port,
                title: update.title,
                state: update.state,
                path: update.path,
            });
        }
        cx.notify();
    }

    /// Replace runtime cards from the host's authoritative list after a local
    /// stream lag. The host list is already in stable creation order.
    pub fn replace_web_clients(&mut self, clients: Vec<WebClientInfo>, cx: &mut Context<Self>) {
        replace_web_client_cards(&mut self.web_clients, clients);
        cx.notify();
    }

    /// Forget a tracked web tunnel (user removed it from the list).
    pub fn remove_web_tunnel(&mut self, url: &str, cx: &mut Context<Self>) {
        let before = self.web_tunnels.len();
        self.web_tunnels.retain(|t| t.url != url);
        if self.web_tunnels.len() == before {
            return;
        }
        cx.emit(WorkspaceStateEvent::StateChanged);
        cx.notify();
    }

    /// Load all persisted workspaces from the store.
    pub fn load() -> Result<Vec<Self>, String> {
        let _guard = workspace_store_lock()
            .lock()
            .map_err(|e| format!("Failed to lock workspace store: {e}"))?;
        Ok(WorkspaceStore::load()?.workspaces)
    }

    /// Removes a workspace from the store by its endpoint address.
    pub fn remove_by_endpoint_add(endpoint_addr: &str) -> Result<(), String> {
        let _guard = workspace_store_lock()
            .lock()
            .map_err(|e| format!("Failed to lock workspace store: {e}"))?;
        let mut store = WorkspaceStore::load()?;

        if store.remove_by_endpoint_addr(endpoint_addr) {
            store.save()?
        }

        Ok(())
    }

    pub fn now_u64() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    /// Saves a workspace entry, updating an existing entry if one with the same endpoint_addr already exists.
    pub fn upsert(entry: Self) -> Result<(), String> {
        let _guard = workspace_store_lock()
            .lock()
            .map_err(|e| format!("Failed to lock workspace store: {e}"))?;
        let mut store = WorkspaceStore::load()?;
        store.upsert(entry)?;

        Ok(())
    }
}

impl WorkspaceStore {
    fn load() -> Result<Self, String> {
        let path: PathBuf = match store_path() {
            Some(p) => p,
            None => return Err("No data directory available".to_string()),
        };
        if !path.exists() {
            return Ok(Self::default());
        }
        match std::fs::read_to_string(&path) {
            Ok(json) => match serde_json::from_str::<Self>(&json) {
                Ok(store) => Ok(store),
                Err(e) => Err(format!("Parse error: {e}")),
            },
            Err(e) => Err(format!("Read error: {e}")),
        }
    }

    fn save(&self) -> Result<(), String> {
        let path = match store_path() {
            Some(p) => p,
            None => return Err("No data directory available".to_string()),
        };
        match serde_json::to_string_pretty(self) {
            Ok(json) => match std::fs::write(&path, json.as_bytes()) {
                Ok(_) => Ok(()),
                Err(e) => Err(format!("Write error: {e}")),
            },
            Err(e) => Err(format!("Serialize error: {e}")),
        }
    }

    fn upsert(&mut self, entry: WorkspaceState) -> Result<(), String> {
        let now = WorkspaceState::now_u64();

        let mut changed = false;
        if let Some(idx) = self
            .workspaces
            .iter()
            .position(|w| w.endpoint_addr == entry.endpoint_addr)
        {
            let workspace = self.workspaces[idx].clone();
            if workspace != entry {
                self.workspaces[idx] = entry;
                changed = true;
            }
        } else {
            let mut entry = entry;
            entry.updated_at = now;
            if entry.created_at == 0 {
                entry.created_at = now;
            }
            self.workspaces.push(entry);
            changed = true;
        }

        if changed {
            self.save()?;
        }

        Ok(())
    }

    fn remove_by_endpoint_addr(&mut self, endpoint_addr: &str) -> bool {
        let before = self.workspaces.len();
        self.workspaces.retain(|w| w.endpoint_addr != endpoint_addr);
        self.workspaces.len() != before
    }
}

impl EventEmitter<WorkspaceStateEvent> for WorkspaceState {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    static TEST_STORE_DIRECTORY_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    struct TestDataDirectoryGuard {
        path: PathBuf,
        _lock: MutexGuard<'static, ()>,
    }

    impl Drop for TestDataDirectoryGuard {
        fn drop(&mut self) {
            *test_data_directory().lock().unwrap() = None;
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn set_test_data_directory(name: &str) -> TestDataDirectoryGuard {
        let lock = TEST_STORE_DIRECTORY_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!(
            "zedra-workspace-state-{name}-{}-{nanos}",
            std::process::id()
        ));
        *test_data_directory().lock().unwrap() = Some(path.clone());
        TestDataDirectoryGuard { path, _lock: lock }
    }

    #[test]
    fn active_main_view_projects_file_selection_only_for_file_views() {
        let file_view = WorkspaceMainView::File {
            path: "src/main.rs".into(),
        };
        assert!(file_view.is_file_path("src/main.rs"));
        assert_eq!(file_view.file_path(), Some("src/main.rs"));
        assert_eq!(file_view.git_diff(), None);
        assert_eq!(file_view.terminal_id(), None);

        let git_diff_view = WorkspaceMainView::GitDiff {
            path: "src/main.rs".into(),
            section: 1,
        };
        assert!(!git_diff_view.is_file_path("src/main.rs"));

        let terminal_view = WorkspaceMainView::Terminal {
            id: "terminal-1".into(),
        };
        assert!(!terminal_view.is_file_path("src/main.rs"));
    }

    #[test]
    fn active_main_view_projects_git_diff_selection_by_path_and_section() {
        let git_diff_view = WorkspaceMainView::GitDiff {
            path: "src/main.rs".into(),
            section: 1,
        };

        assert!(git_diff_view.is_git_diff("src/main.rs", 1));
        assert_eq!(git_diff_view.git_diff(), Some(("src/main.rs", 1)));
        assert!(!git_diff_view.is_git_diff("src/main.rs", 0));
        assert!(!git_diff_view.is_git_diff("src/lib.rs", 1));
        assert_eq!(git_diff_view.file_path(), None);
        assert_eq!(git_diff_view.terminal_id(), None);
    }

    #[test]
    fn navigation_stack_moves_existing_route_to_top() {
        let mut stack = WorkspaceNavigationStack::default();
        let terminal = WorkspaceMainView::Terminal {
            id: "terminal-1".into(),
        };
        let file = WorkspaceMainView::File {
            path: "src/main.rs".into(),
        };
        let diff = WorkspaceMainView::GitDiff {
            path: "src/lib.rs".into(),
            section: 1,
        };

        stack.open(terminal.clone());
        stack.open(file.clone());
        stack.open(diff.clone());
        stack.open(terminal.clone());

        assert_eq!(stack.active(), terminal);
        assert_eq!(stack.go_back(), Some(diff));
        assert_eq!(stack.go_back(), Some(file));
        assert_eq!(stack.go_back(), None);
    }

    #[test]
    fn navigation_stack_replace_does_not_grow_history() {
        let mut stack = WorkspaceNavigationStack::default();
        let file = WorkspaceMainView::File {
            path: "src/main.rs".into(),
        };

        stack.open(WorkspaceMainView::Terminal {
            id: "terminal-1".into(),
        });
        stack.open(file.clone());
        stack.replace(WorkspaceMainView::AgentSessions);

        assert_eq!(stack.active(), WorkspaceMainView::AgentSessions);
        assert_eq!(
            stack.go_back(),
            Some(WorkspaceMainView::Terminal {
                id: "terminal-1".into()
            })
        );
        assert_eq!(stack.go_back(), None);
    }

    #[test]
    fn navigation_stack_prunes_stale_terminal_routes() {
        let mut stack = WorkspaceNavigationStack::default();
        let file = WorkspaceMainView::File {
            path: "src/main.rs".into(),
        };
        stack.open(WorkspaceMainView::Terminal {
            id: "terminal-1".into(),
        });
        stack.open(file.clone());
        stack.open(WorkspaceMainView::Terminal {
            id: "terminal-2".into(),
        });

        stack.prune_stale_terminals(&["terminal-1".to_string()]);

        assert_eq!(stack.active(), file);
        assert_eq!(
            stack.go_back(),
            Some(WorkspaceMainView::Terminal {
                id: "terminal-1".into()
            })
        );
    }

    #[test]
    fn navigation_stack_removes_buried_terminal_route() {
        let mut stack = WorkspaceNavigationStack::default();
        let file = WorkspaceMainView::File {
            path: "src/main.rs".into(),
        };
        let diff = WorkspaceMainView::GitDiff {
            path: "src/lib.rs".into(),
            section: 0,
        };

        stack.open(WorkspaceMainView::Terminal {
            id: "terminal-pending".into(),
        });
        stack.open(file.clone());
        stack.open(diff.clone());

        stack.remove_terminal("terminal-pending");

        assert_eq!(stack.active(), diff);
        assert_eq!(stack.go_back(), Some(file));
        assert_eq!(stack.go_back(), None);
    }

    #[test]
    fn active_main_view_terminal_id_ignores_last_active_terminal() {
        let mut state = WorkspaceState::default();
        state.active_terminal_id = Some("terminal-pending".into());
        state.active_main_view = WorkspaceMainView::GitDiff {
            path: "src/lib.rs".into(),
            section: 0,
        };

        assert_eq!(state.active_main_view_terminal_id(), None);
    }

    #[test]
    fn manual_disconnect_sets_disconnected_phase_and_clears_runtime_state() {
        let mut state = WorkspaceState {
            endpoint_addr: "endpoint".into(),
            session_id: "session".into(),
            project_name: "project".into(),
            connect_phase: Some(ConnectPhase::Connected),
            active_terminal_id: Some("terminal-1".into()),
            active_main_view: WorkspaceMainView::Terminal {
                id: "terminal-1".into(),
            },
            terminal_ids: vec!["terminal-1".into(), "terminal-2".into()],
            host_info: Some(HostInfoSnapshot {
                captured_at_ms: 100,
                cpu_usage_percent: 25.0,
                cpu_count: 8,
                memory_used_bytes: 1024,
                memory_total_bytes: 2048,
                swap_used_bytes: 0,
                swap_total_bytes: 0,
                system_uptime_secs: 30,
                batteries: Vec::new(),
            }),
            web_clients: vec![WebClientCard {
                id: "web-1".into(),
                slug: "opencode".into(),
                port: 4096,
                title: Some("Session".into()),
                state: AgentState::Running,
                path: "/session/1".into(),
            }],
            ..Default::default()
        };

        state.clear_runtime_state_for_disconnect();

        assert_eq!(state.endpoint_addr, "endpoint");
        assert_eq!(state.session_id, "session");
        assert_eq!(state.project_name, "project");
        assert_eq!(state.connect_phase, Some(ConnectPhase::Disconnected));
        assert_eq!(state.active_terminal_id, None);
        assert_eq!(state.active_main_view, WorkspaceMainView::Default);
        assert!(state.terminal_ids.is_empty());
        assert_eq!(state.host_info, None);
        assert!(state.web_clients.is_empty());
    }

    #[test]
    fn authoritative_web_client_list_replaces_zombies_and_preserves_order() {
        let mut cards = vec![WebClientCard {
            id: "zombie".into(),
            slug: "opencode".into(),
            port: 4000,
            title: None,
            state: AgentState::Idle,
            path: "/zombie".into(),
        }];
        let clients = vec![
            WebClientInfo {
                id: "first".into(),
                slug: "opencode".into(),
                port: 4096,
                title: Some("First".into()),
                state: AgentState::Running,
                path: "/first".into(),
            },
            WebClientInfo {
                id: "second".into(),
                slug: "opencode".into(),
                port: 4097,
                title: Some("Second".into()),
                state: AgentState::Completed,
                path: "/second".into(),
            },
        ];

        replace_web_client_cards(&mut cards, clients);

        assert_eq!(
            cards
                .iter()
                .map(|card| card.id.as_str())
                .collect::<Vec<_>>(),
            vec!["first", "second"]
        );
    }

    #[tokio::test]
    async fn sync_fields_ignores_network_only_session_snapshot_changes() {
        let session = Session::new(tokio::runtime::Handle::current());
        let mut session_state = SessionState::new();
        session_state.phase = ConnectPhase::Connected;
        session_state.snapshot.session_id = Some("session".into());
        session_state.snapshot.has_ipv4 = true;
        session_state.snapshot.has_ipv6 = true;
        session_state.snapshot.mapping_varies = Some(false);
        session_state.snapshot.relay_latency_ms = Some(12);

        let mut state = WorkspaceState {
            session_id: "session".into(),
            connect_phase: Some(ConnectPhase::Connected),
            ..Default::default()
        };

        assert!(!state.sync_fields_from_session(session.handle(), &session_state));

        session_state.snapshot.relay_latency_ms = Some(30);

        assert!(!state.sync_fields_from_session(session.handle(), &session_state));
    }

    #[tokio::test]
    async fn sync_fields_reports_workspace_phase_changes() {
        let session = Session::new(tokio::runtime::Handle::current());
        let mut session_state = SessionState::new();
        session_state.phase = ConnectPhase::Connected;
        session_state.snapshot.session_id = Some("session".into());

        let mut state = WorkspaceState {
            session_id: "session".into(),
            connect_phase: Some(ConnectPhase::Sync),
            ..Default::default()
        };

        assert!(state.sync_fields_from_session(session.handle(), &session_state));
        assert_eq!(state.connect_phase, Some(ConnectPhase::Connected));
    }

    #[tokio::test]
    async fn reconnect_phase_clears_cards_before_stream_reseeds_them() {
        let session = Session::new(tokio::runtime::Handle::current());
        let mut session_state = SessionState::new();
        session_state.phase = ConnectPhase::Reconnecting {
            attempt: 1,
            reason: ReconnectReason::ConnectionLost,
            next_retry_secs: 0,
        };
        let mut state = WorkspaceState {
            connect_phase: Some(ConnectPhase::Connected),
            web_clients: vec![WebClientCard {
                id: "stale".into(),
                slug: "opencode".into(),
                port: 4096,
                title: None,
                state: AgentState::Idle,
                path: "/stale".into(),
            }],
            ..Default::default()
        };

        assert!(state.sync_fields_from_session(session.handle(), &session_state));
        assert!(state.web_clients.is_empty());
    }

    #[test]
    fn upsert_creates_missing_workspace_store() {
        let _guard = set_test_data_directory("upsert-creates-missing-store");

        WorkspaceState::upsert(WorkspaceState {
            endpoint_addr: "endpoint-a".into(),
            project_name: "Project A".into(),
            ..Default::default()
        })
        .unwrap();

        let loaded = WorkspaceState::load().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].endpoint_addr, "endpoint-a");
        assert_eq!(loaded[0].project_name, "Project A");
    }

    #[test]
    fn upsert_persists_docs_tree_collapsed_dirs() {
        let _guard = set_test_data_directory("upsert-persists-docs-tree-collapsed-dirs");

        WorkspaceState::upsert(WorkspaceState {
            endpoint_addr: "endpoint-a".into(),
            docs_tree_collapsed_dirs: vec!["crates/zedra".into(), "vendor/zed/docs".into()],
            ..Default::default()
        })
        .unwrap();

        let loaded = WorkspaceState::load().unwrap();
        assert_eq!(
            loaded[0].docs_tree_collapsed_dirs,
            vec!["crates/zedra", "vendor/zed/docs"]
        );
    }

    #[test]
    fn upsert_persists_web_tunnels() {
        let _guard = set_test_data_directory("upsert-persists-web-tunnels");

        WorkspaceState::upsert(WorkspaceState {
            endpoint_addr: "endpoint-a".into(),
            web_tunnels: vec![TrackedTunnel {
                url: "http://localhost:5173".into(),
                title: "localhost:5173".into(),
                last_opened_at: 42,
            }],
            ..Default::default()
        })
        .unwrap();

        let loaded = WorkspaceState::load().unwrap();
        assert_eq!(loaded[0].web_tunnels.len(), 1);
        assert_eq!(loaded[0].web_tunnels[0].url, "http://localhost:5173");
    }

    #[test]
    fn upsert_web_tunnel_appends_new_in_open_order() {
        let mut tunnels = Vec::new();
        upsert_web_tunnel(&mut tunnels, "http://localhost:5173", "localhost:5173", 1);
        upsert_web_tunnel(&mut tunnels, "http://localhost:8080", "localhost:8080", 2);
        upsert_web_tunnel(&mut tunnels, "http://localhost:3000", "localhost:3000", 3);

        let urls: Vec<&str> = tunnels.iter().map(|t| t.url.as_str()).collect();
        assert_eq!(
            urls,
            [
                "http://localhost:5173",
                "http://localhost:8080",
                "http://localhost:3000"
            ]
        );
    }

    #[test]
    fn upsert_web_tunnel_reopen_updates_in_place_without_reordering() {
        let mut tunnels = Vec::new();
        upsert_web_tunnel(&mut tunnels, "http://localhost:5173", "old", 1);
        upsert_web_tunnel(&mut tunnels, "http://localhost:8080", "localhost:8080", 2);

        // Reopen the first entry: it keeps index 0, updates title + timestamp.
        upsert_web_tunnel(&mut tunnels, "http://localhost:5173", "new", 9);

        assert_eq!(tunnels.len(), 2);
        assert_eq!(tunnels[0].url, "http://localhost:5173");
        assert_eq!(tunnels[0].title, "new");
        assert_eq!(tunnels[0].last_opened_at, 9);
        assert_eq!(tunnels[1].url, "http://localhost:8080");
    }

    #[test]
    fn upsert_persists_delta_host_binding() {
        let _guard = set_test_data_directory("upsert-persists-delta-host-binding");
        let delta_host_pubkey = [7u8; 32];
        let delta_host_node_id = Uuid::parse_str("33333333-3333-3333-3333-333333333333").unwrap();

        WorkspaceState::upsert(WorkspaceState {
            endpoint_addr: "endpoint-a".into(),
            delta_host_pubkey: Some(delta_host_pubkey),
            delta_host_node_id: Some(delta_host_node_id),
            ..Default::default()
        })
        .unwrap();

        let loaded = WorkspaceState::load().unwrap();
        assert_eq!(loaded[0].delta_host_pubkey, Some(delta_host_pubkey));
        assert_eq!(loaded[0].delta_host_node_id, Some(delta_host_node_id));
    }

    #[test]
    fn remove_by_endpoint_addr_persists_removal() {
        let _guard = set_test_data_directory("remove-persists-removal");
        for endpoint_addr in ["endpoint-a", "endpoint-b"] {
            WorkspaceState::upsert(WorkspaceState {
                endpoint_addr: endpoint_addr.into(),
                ..Default::default()
            })
            .unwrap();
        }

        WorkspaceState::remove_by_endpoint_add("endpoint-a").unwrap();

        let loaded = WorkspaceState::load().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].endpoint_addr, "endpoint-b");
    }
}
