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
    pub saved_index: Option<usize>,
    #[serde(skip)]
    pub connect_phase: Option<zedra_session::ConnectPhase>,
    #[serde(skip)]
    pub terminal_count: usize,
    #[serde(skip)]
    pub terminal_ids: Vec<String>,
    #[serde(skip)]
    pub active_terminal_id: Option<String>,
    #[serde(skip)]
    pub endpoint_addr_encoded: Option<String>,
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
            log::error!("WorkspaceState: failed to create dir {:?}: {}", dir, e);
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
                log::warn!("WorkspaceState: no data directory available");
                return Vec::new();
            }
        };
        if !path.exists() {
            log::info!("WorkspaceState: no store file yet at {:?}", path);
            return Vec::new();
        }
        match std::fs::read_to_string(&path) {
            Ok(json) => match serde_json::from_str::<StoreFile>(&json) {
                Ok(state) => {
                    log::info!(
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
                    log::error!("WorkspaceState: parse error: {}", e);
                    Vec::new()
                }
            },
            Err(e) => {
                log::error!("WorkspaceState: read error: {}", e);
                Vec::new()
            }
        }
    }

    fn save_all(workspaces: &[Self]) {
        let path = match store_path() {
            Some(p) => p,
            None => {
                log::warn!("WorkspaceState: no data directory, skipping save");
                return;
            }
        };
        let len = workspaces.len();
        let file = StoreFile {
            workspaces: workspaces.iter().map(|s| s.inner().clone()).collect(),
        };
        match serde_json::to_string_pretty(&file) {
            Ok(json) => match std::fs::write(&path, json.as_bytes()) {
                Ok(()) => log::info!("WorkspaceState: saved {} workspace(s) to {:?}", len, path),
                Err(e) => log::error!("WorkspaceState: write error: {}", e),
            },
            Err(e) => log::error!("WorkspaceState: serialize error: {}", e),
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
        if let Some(idx) = workspaces.iter().position(|w| w.endpoint_addr() == entry_addr) {
            let mut i = workspaces[idx].inner().clone();
            i.session_id = entry.inner().session_id.clone();
            i.strip_path = entry.inner().strip_path.clone();
            i.hostname = entry.inner().hostname.clone();
            i.project_name = entry.inner().project_name.clone();
            i.workdir = entry.inner().workdir.clone();
            i.homedir = entry.inner().homedir.clone();
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

    fn update_inner<F>(s: Self, f: F) -> Self
    where
        F: FnOnce(&mut WorkspaceStateInner),
    {
        let mut i = (*s.0).clone();
        f(&mut i);
        Self(Arc::new(i))
    }

    /// Build from runtime summary (workspace view → home/QA). Persisted fields are minimal.
    pub fn from_summary(
        endpoint_addr: String,
        strip_path: String,
        project_name: String,
        hostname: String,
        workspace_index: usize,
        connect_phase: zedra_session::ConnectPhase,
        terminal_count: usize,
        terminal_ids: Vec<String>,
        active_terminal_id: Option<String>,
        endpoint_addr_encoded: Option<String>,
    ) -> Self {
        Self(Arc::new(WorkspaceStateInner {
            endpoint_addr,
            session_id: String::new(),
            strip_path,
            project_name,
            workdir: String::new(),
            homedir: String::new(),
            hostname,
            created_at: 0,
            updated_at: 0,
            workspace_index: Some(workspace_index),
            saved_index: None,
            connect_phase: Some(connect_phase),
            terminal_count,
            terminal_ids,
            active_terminal_id,
            endpoint_addr_encoded,
        }))
    }

    pub fn from_handle(handle: &zedra_session::SessionHandle) -> Option<Self> {
        let addr = handle.endpoint_addr()?;
        let encoded = zedra_rpc::pairing::encode_endpoint_addr(&addr).ok()?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Some(Self(Arc::new(WorkspaceStateInner {
            endpoint_addr: encoded.clone(),
            session_id: handle.session_id().unwrap_or_default(),
            strip_path: handle.strip_path(),
            project_name: handle.project_name(),
            workdir: handle.workdir(),
            homedir: handle.homedir(),
            hostname: handle.hostname(),
            created_at: now,
            updated_at: now,
            workspace_index: None,
            saved_index: None,
            connect_phase: None,
            terminal_count: 0,
            terminal_ids: Vec::new(),
            active_terminal_id: None,
            endpoint_addr_encoded: Some(encoded),
        })))
    }

    pub fn for_saved_row(self, saved_index: usize) -> Self {
        Self::update_inner(self, |i| i.saved_index = Some(saved_index))
    }

    pub fn display_name(&self) -> &str {
        let p = &self.0.project_name;
        if p.is_empty() {
            "Workspace"
        } else {
            p
        }
    }

    pub fn workspace_index(&self) -> Option<usize> {
        self.0.workspace_index
    }

    pub fn saved_index(&self) -> Option<usize> {
        self.0.saved_index
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

    pub fn endpoint_addr_encoded(&self) -> Option<&str> {
        self.0.endpoint_addr_encoded.as_deref()
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

    pub fn with_saved_index(self, saved_index: usize) -> Self {
        Self::update_inner(self, |i| i.saved_index = Some(saved_index))
    }
}
