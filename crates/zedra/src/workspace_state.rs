//! Workspace state and persistence.
//!
//! [WorkspaceState] is a shareable handle to [WorkspaceStateInner]. Clone is cheap (Arc).
//! All reads go through methods so the inner can be shared across views/threads without blocking.

use gpui::{Context, EventEmitter};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

use zedra_session::*;

use crate::platform_bridge;

/// Shared list of workspace states. Views hold an Arc and read via `.iter()` (no lock, non-blocking).
pub type SharedWorkspaceStates = Arc<Vec<WorkspaceState>>;

const STORE_DIR: &str = "zedra";
const STORE_FILE: &str = "workspaces.json";

/// Root file format (serializes inner to avoid Arc in JSON).
#[derive(Clone, Default, Serialize, Deserialize)]
struct StoreFile {
    workspaces: Vec<WorkspaceState>,
}

pub enum WorkspaceStateEvent {
    StateChanged,
    SyncComplete,
}

/// Shareable workspace state. Clone copies the Arc only. Read via methods (non-blocking).
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct WorkspaceState {
    pub endpoint_addr: String,
    pub session_id: String,
    pub strip_path: String,
    pub project_name: String,
    pub workdir: String,
    pub homedir: String,
    pub hostname: String,
    pub created_at: u64,
    pub updated_at: u64,

    #[serde(skip)]
    pub connect_phase: Option<ConnectPhase>,
    #[serde(skip)]
    pub active_terminal_id: Option<String>,
    #[serde(skip)]
    pub terminal_ids: Vec<String>,
    #[serde(skip)]
    pub remote_terminals: Vec<RemoteTerminal>,
}

fn store_path() -> Option<PathBuf> {
    let data_dir = platform_bridge::bridge().data_directory()?;
    let dir = PathBuf::from(data_dir).join(STORE_DIR);
    if !dir.exists() {
        if let Err(e) = std::fs::create_dir_all(&dir) {
            tracing::error!(dir = ?dir, err = %e, "store: create dir failed");
            return None;
        }
    }
    Some(dir.join(STORE_FILE))
}

impl WorkspaceState {
    pub fn sync_from_session(
        &mut self,
        session_handle: &SessionHandle,
        session_state: &SessionState,
        cx: &mut Context<Self>,
    ) {
        let session_id = session_state.snapshot.session_id.clone();
        self.connect_phase = Some(session_state.phase.clone());
        self.terminal_ids = session_handle.terminal_ids().clone();
        self.remote_terminals = session_handle.terminals().clone();

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

        cx.emit(WorkspaceStateEvent::StateChanged);
    }

    pub fn remote_terminal(&self, id: &str) -> Option<&RemoteTerminal> {
        self.remote_terminals.iter().find(|t| t.id() == id)
    }

    pub fn emit_sync_complete(&self, cx: &mut Context<Self>) {
        cx.emit(WorkspaceStateEvent::SyncComplete);
    }
}

/// Store implementation for [`WorkspaceState`].
impl WorkspaceState {
    /// Load all persisted workspaces from the store.
    pub fn load() -> Vec<Self> {
        let path = match store_path() {
            Some(p) => p,
            None => {
                tracing::warn!("WorkspaceState: no data directory available");
                return Vec::new();
            }
        };
        if !path.exists() {
            tracing::info!("WorkspaceState: no store file yet at {:?}", path);
            return Vec::new();
        }
        match std::fs::read_to_string(&path) {
            Ok(json) => match serde_json::from_str::<StoreFile>(&json) {
                Ok(state) => {
                    tracing::info!(
                        "WorkspaceState: loaded {} workspace(s) from {:?}",
                        state.workspaces.len(),
                        path
                    );
                    state.workspaces.into_iter().map(|i| i).collect()
                }
                Err(e) => {
                    tracing::error!("WorkspaceState: parse error: {}", e);
                    Vec::new()
                }
            },
            Err(e) => {
                tracing::error!("WorkspaceState: read error: {}", e);
                Vec::new()
            }
        }
    }

    /// Save all workspaces to the store.
    fn save_all(workspaces: &[Self]) {
        let path = match store_path() {
            Some(p) => p,
            None => {
                tracing::warn!("WorkspaceState: no data directory, skipping save");
                return;
            }
        };
        let len = workspaces.len();
        let file = StoreFile {
            workspaces: workspaces.iter().map(|s| s.clone()).collect(),
        };
        match serde_json::to_string_pretty(&file) {
            Ok(json) => match std::fs::write(&path, json.as_bytes()) {
                Ok(()) => {
                    tracing::info!("WorkspaceState: saved {} workspace(s) to {:?}", len, path)
                }
                Err(e) => tracing::error!("WorkspaceState: write error: {}", e),
            },
            Err(e) => tracing::error!("WorkspaceState: serialize error: {}", e),
        }
    }

    /// Removes a workspace from the store by its endpoint address.
    pub fn remove_by_endpoint_add(endpoint_addr: &str) {
        let mut workspaces = Self::load();
        let before = workspaces.len();
        workspaces.retain(|w| w.endpoint_addr != endpoint_addr);
        if workspaces.len() != before {
            Self::save_all(&workspaces);
        }
    }

    pub fn now_u64() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    /// Saves a workspace entry, updating an existing entry if one with the same endpoint_addr already exists.
    pub fn upsert(entry: Self) {
        let mut workspaces = Self::load();
        let now = Self::now_u64();

        if let Some(idx) = workspaces
            .iter()
            .position(|w| w.endpoint_addr == entry.endpoint_addr)
        {
            let mut workspace = workspaces[idx].clone();
            workspace.session_id = entry.session_id.clone();
            if !entry.strip_path.is_empty() {
                workspace.strip_path = entry.strip_path.clone();
            }
            if !entry.hostname.is_empty() {
                workspace.hostname = entry.hostname.clone();
            }
            if !entry.project_name.is_empty() {
                workspace.project_name = entry.project_name.clone();
            }
            if !entry.workdir.is_empty() {
                workspace.workdir = entry.workdir.clone();
            }
            if !entry.homedir.is_empty() {
                workspace.homedir = entry.homedir.clone();
            }
            workspace.updated_at = now;
            workspaces[idx] = workspace;
        } else {
            let mut entry = entry.clone();
            entry.updated_at = now;
            if entry.created_at == 0 {
                entry.created_at = now;
            }
            workspaces.push(entry);
        }

        Self::save_all(&workspaces);
    }

    /// Construct a [`WorkspaceState`] from a [`SessionHandle`] and [`SessionState`].
    pub fn from_session(
        session_handle: &SessionHandle,
        session_state: &SessionState,
    ) -> Option<Self> {
        let addr = session_handle.endpoint_addr()?;
        let encoded = zedra_rpc::pairing::encode_endpoint_addr(&addr).ok()?;
        let snap = session_state.snapshot();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        Some(Self {
            endpoint_addr: encoded,
            session_id: session_handle.session_id().unwrap_or_default(),
            strip_path: snap.strip_path,
            project_name: snap.project_name,
            workdir: snap.workdir,
            homedir: snap.homedir,
            hostname: snap.hostname,
            created_at: now,
            updated_at: now,
            connect_phase: None,
            active_terminal_id: None,
            terminal_ids: Vec::new(),
            remote_terminals: Vec::new(),
        })
    }
}

impl EventEmitter<WorkspaceStateEvent> for WorkspaceState {}
