//! Workspace state and persistence.
//!
//! [WorkspaceState] is a shareable handle to [WorkspaceStateInner]. Clone is cheap (Arc).
//! All reads go through methods so the inner can be shared across views/threads without blocking.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

use crate::platform_bridge;

/// Shared list of workspace states. Views hold an Arc and read via `.iter()` (no lock, non-blocking).
pub type SharedWorkspaceStates = Arc<Vec<WorkspaceState>>;

const STORE_DIR: &str = "zedra";
const STORE_FILE: &str = "workspaces.json";

/// Root file format (serializes inner to avoid Arc in JSON).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct StoreFile {
    workspaces: Vec<WorkspaceStateInner>,
}

/// Inner data; not used directly. All access via [WorkspaceState] methods.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WorkspaceStateInner {
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
    pub workspace_index: Option<usize>,
    #[serde(skip)]
    pub connect_phase: Option<zedra_session::ConnectPhase>,
    #[serde(skip)]
    pub terminal_count: usize,
    #[serde(skip)]
    pub terminal_ids: Vec<String>,
    #[serde(skip)]
    pub active_terminal_id: Option<String>,
}

/// Shareable workspace state. Clone copies the Arc only. Read via methods (non-blocking).
#[derive(Clone, Debug)]
pub struct WorkspaceState(Arc<WorkspaceStateInner>);

impl Default for WorkspaceState {
    fn default() -> Self {
        Self(Arc::new(WorkspaceStateInner::default()))
    }
}

impl Serialize for WorkspaceState {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for WorkspaceState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        WorkspaceStateInner::deserialize(deserializer).map(|i| Self(Arc::new(i)))
    }
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
    #[inline]
    fn inner(&self) -> &WorkspaceStateInner {
        &self.0
    }

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
                    state
                        .workspaces
                        .into_iter()
                        .map(|i| Self(Arc::new(i)))
                        .collect()
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
            workspaces: workspaces.iter().map(|s| s.inner().clone()).collect(),
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

    pub fn remove(endpoint_addr: &str) {
        let mut workspaces = Self::load();
        let before = workspaces.len();
        workspaces.retain(|w| w.endpoint_addr() != endpoint_addr);
        if workspaces.len() != before {
            Self::save_all(&workspaces);
        }
    }

    pub fn upsert(entry: Self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut workspaces = Self::load();
        let entry_addr = entry.endpoint_addr();
        if let Some(idx) = workspaces
            .iter()
            .position(|w| w.endpoint_addr() == entry_addr)
        {
            let mut i = workspaces[idx].inner().clone();
            // Always update session_id (set before connect attempt).
            // Only overwrite display fields when non-empty — the handle is empty
            // during the connecting phase, so we preserve saved info until the
            // connection succeeds and the server populates these fields.
            i.session_id = entry.inner().session_id.clone();
            if !entry.inner().strip_path.is_empty() {
                i.strip_path = entry.inner().strip_path.clone();
            }
            if !entry.inner().hostname.is_empty() {
                i.hostname = entry.inner().hostname.clone();
            }
            if !entry.inner().project_name.is_empty() {
                i.project_name = entry.inner().project_name.clone();
            }
            if !entry.inner().workdir.is_empty() {
                i.workdir = entry.inner().workdir.clone();
            }
            if !entry.inner().homedir.is_empty() {
                i.homedir = entry.inner().homedir.clone();
            }
            i.updated_at = now;
            workspaces[idx] = Self(Arc::new(i));
        } else {
            let mut e = entry.inner().clone();
            e.updated_at = now;
            if e.created_at == 0 {
                e.created_at = now;
            }
            workspaces.push(Self(Arc::new(e)));
        }
        Self::save_all(&workspaces);
    }

    pub(crate) fn update_inner<F>(s: Self, f: F) -> Self
    where
        F: FnOnce(&mut WorkspaceStateInner),
    {
        let mut i = (*s.0).clone();
        f(&mut i);
        Self(Arc::new(i))
    }

    pub fn from_session(
        handle: &zedra_session::SessionHandle,
        session_state: &zedra_session::SessionState,
    ) -> Option<Self> {
        let addr = handle.endpoint_addr()?;
        let encoded = zedra_rpc::pairing::encode_endpoint_addr(&addr).ok()?;
        let snap = session_state.get().snapshot;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Some(Self(Arc::new(WorkspaceStateInner {
            endpoint_addr: encoded,
            session_id: handle.session_id().unwrap_or_default(),
            strip_path: snap.strip_path,
            project_name: snap.project_name,
            workdir: snap.workdir,
            homedir: snap.homedir,
            hostname: snap.hostname,
            created_at: now,
            updated_at: now,
            workspace_index: None,
            connect_phase: None,
            terminal_count: 0,
            terminal_ids: Vec::new(),
            active_terminal_id: None,
        })))
    }

    pub fn display_name(&self) -> &str {
        let p = &self.0.project_name;
        if p.is_empty() { "Workspace" } else { p }
    }

    pub fn workspace_index(&self) -> Option<usize> {
        self.0.workspace_index
    }

    pub fn connect_phase(&self) -> Option<&zedra_session::ConnectPhase> {
        self.0.connect_phase.as_ref()
    }

    pub fn terminal_count(&self) -> usize {
        self.0.terminal_count
    }

    pub fn terminal_ids(&self) -> &[String] {
        &self.0.terminal_ids
    }

    pub fn active_terminal_id(&self) -> Option<&String> {
        self.0.active_terminal_id.as_ref()
    }

    pub fn endpoint_addr(&self) -> &str {
        &self.0.endpoint_addr
    }

    pub fn session_id(&self) -> &str {
        &self.0.session_id
    }

    pub fn project_name(&self) -> &str {
        &self.0.project_name
    }

    pub fn strip_path(&self) -> &str {
        &self.0.strip_path
    }

    pub fn hostname(&self) -> &str {
        &self.0.hostname
    }

    pub fn workdir(&self) -> &str {
        &self.0.workdir
    }

    pub fn homedir(&self) -> &str {
        &self.0.homedir
    }
}
