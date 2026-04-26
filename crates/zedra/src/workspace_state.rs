use gpui::{Context, EventEmitter};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use tracing::*;
use zedra_rpc::proto::HostInfoSnapshot;

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
    pub host_info: Option<HostInfoSnapshot>,
}

/// PartialEq implementation for WorkspaceState.
/// Compare all durable fields to prevent unnecessary updates.
impl PartialEq for WorkspaceState {
    fn eq(&self, other: &Self) -> bool {
        self.endpoint_addr == other.endpoint_addr
            && self.session_id == other.session_id
            && self.strip_path == other.strip_path
            && self.project_name == other.project_name
            && self.workdir == other.workdir
            && self.homedir == other.homedir
            && self.hostname == other.hostname
            && self.created_at == other.created_at
            && self.updated_at == other.updated_at
    }
}

static WORKSPACE_STORE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn workspace_store_lock() -> &'static Mutex<()> {
    WORKSPACE_STORE_LOCK.get_or_init(|| Mutex::new(()))
}

fn store_path() -> Option<PathBuf> {
    let data_dir = platform_bridge::bridge().data_directory()?;
    let dir = PathBuf::from(data_dir).join(STORE_DIR);
    if !dir.exists() {
        if let Err(e) = std::fs::create_dir_all(&dir) {
            error!(dir = ?dir, err = %e, "Failed to create directory: {e}");
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

        cx.emit(WorkspaceStateEvent::StateChanged);
    }

    pub fn emit_sync_complete(&self, cx: &mut Context<Self>) {
        cx.emit(WorkspaceStateEvent::SyncComplete);
    }

    pub fn update_host_info(&mut self, host_info: HostInfoSnapshot, cx: &mut Context<Self>) {
        self.host_info = Some(host_info);
        cx.emit(WorkspaceStateEvent::HostInfoChanged);
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
            return Err(format!("No store file yet at {:?}", path));
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
