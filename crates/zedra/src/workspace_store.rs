// WorkspaceStore — persists workspace connection data across app restarts.
//
// Stores endpoint addresses and credentials so users can reconnect to
// previously-paired hosts without re-scanning the QR code.
//
// Storage format: JSON file in the app's data directory.
// Works on both iOS (Documents/) and Android (internal files/).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Serialized workspace entry persisted to disk.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersistedWorkspace {
    /// base64-url encoded iroh::EndpointAddr (postcard binary).
    pub endpoint_addr: String,
    /// RPC session ID (assigned by zedra-host, enables PTY resumption).
    pub session_id: Option<String>,
    /// Last known remote working directory (display only).
    pub last_project_path: Option<String>,
    /// Last known remote hostname (display only).
    pub last_hostname: Option<String>,
    /// Unix timestamp (seconds) when this workspace was first created.
    pub created_at: u64,
}

impl PersistedWorkspace {
    /// Returns the project name (last path component of `last_project_path`).
    pub fn project_name(&self) -> Option<&str> {
        self.last_project_path
            .as_deref()
            .and_then(|p| p.rsplit('/').next())
            .filter(|s| !s.is_empty())
    }

    /// Display label in `host@project` format, falling back to whichever part is available.
    pub fn display_name(&self) -> String {
        match (self.last_hostname.as_deref(), self.project_name()) {
            (Some(host), Some(project)) => format!("{}@{}", host, project),
            (Some(host), None) => host.to_string(),
            (None, Some(project)) => project.to_string(),
            (None, None) => "Workspace".to_string(),
        }
    }
}

/// Top-level persisted state.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct PersistedState {
    workspaces: Vec<PersistedWorkspace>,
}

const STORE_DIR: &str = "zedra";
const STORE_FILE: &str = "workspaces.json";

/// Returns the full path to the workspace store file, creating the directory if needed.
fn store_path() -> Option<PathBuf> {
    let data_dir = crate::platform_bridge::bridge().data_directory()?;
    let dir = PathBuf::from(data_dir).join(STORE_DIR);
    if !dir.exists() {
        if let Err(e) = std::fs::create_dir_all(&dir) {
            log::error!("WorkspaceStore: failed to create dir {:?}: {}", dir, e);
            return None;
        }
    }
    Some(dir.join(STORE_FILE))
}

/// Load all persisted workspaces from disk.
pub fn load_workspaces() -> Vec<PersistedWorkspace> {
    let path = match store_path() {
        Some(p) => p,
        None => {
            log::warn!("WorkspaceStore: no data directory available");
            return Vec::new();
        }
    };

    if !path.exists() {
        log::info!("WorkspaceStore: no store file yet at {:?}", path);
        return Vec::new();
    }

    match std::fs::read_to_string(&path) {
        Ok(json) => match serde_json::from_str::<PersistedState>(&json) {
            Ok(state) => {
                log::info!(
                    "WorkspaceStore: loaded {} workspace(s) from {:?}",
                    state.workspaces.len(),
                    path
                );
                state.workspaces
            }
            Err(e) => {
                log::error!("WorkspaceStore: parse error: {}", e);
                Vec::new()
            }
        },
        Err(e) => {
            log::error!("WorkspaceStore: read error: {}", e);
            Vec::new()
        }
    }
}

/// Save workspaces to disk (overwrites existing file).
pub fn save_workspaces(workspaces: &[PersistedWorkspace]) {
    let path = match store_path() {
        Some(p) => p,
        None => {
            log::warn!("WorkspaceStore: no data directory, skipping save");
            return;
        }
    };

    let state = PersistedState {
        workspaces: workspaces.to_vec(),
    };

    match serde_json::to_string_pretty(&state) {
        Ok(json) => match std::fs::write(&path, json.as_bytes()) {
            Ok(()) => {
                log::info!(
                    "WorkspaceStore: saved {} workspace(s) to {:?}",
                    workspaces.len(),
                    path
                );
            }
            Err(e) => {
                log::error!("WorkspaceStore: write error: {}", e);
            }
        },
        Err(e) => {
            log::error!("WorkspaceStore: serialize error: {}", e);
        }
    }
}

/// Remove a workspace by its endpoint address string and persist.
pub fn remove_workspace(endpoint_addr: &str) {
    let mut workspaces = load_workspaces();
    let before = workspaces.len();
    workspaces.retain(|w| w.endpoint_addr != endpoint_addr);
    if workspaces.len() != before {
        save_workspaces(&workspaces);
    }
}

/// Insert or update a workspace entry and persist.
/// Matches by endpoint_addr; updates credentials and metadata if found, inserts if new.
pub fn upsert_workspace(entry: PersistedWorkspace) {
    let mut workspaces = load_workspaces();
    if let Some(existing) = workspaces
        .iter_mut()
        .find(|w| w.endpoint_addr == entry.endpoint_addr)
    {
        existing.session_id = entry.session_id;
        if entry.last_project_path.is_some() {
            existing.last_project_path = entry.last_project_path;
        }
        if entry.last_hostname.is_some() {
            existing.last_hostname = entry.last_hostname;
        }
    } else {
        workspaces.push(entry);
    }
    save_workspaces(&workspaces);
}

/// Snapshot the current state of a SessionHandle into a PersistedWorkspace.
/// Returns None if the handle has no endpoint address stored.
pub fn snapshot_from_handle(handle: &zedra_session::SessionHandle) -> Option<PersistedWorkspace> {
    let addr = handle.endpoint_addr()?;
    let encoded = zedra_rpc::pairing::encode_endpoint_addr(&addr).ok()?;
    let session_id = handle.session_id();

    let (project_path, hostname) = if let Some(session) = handle.session() {
        match session.state() {
            zedra_session::SessionState::Connected {
                workdir, hostname, ..
            } => {
                let wp = if workdir.is_empty() {
                    None
                } else {
                    Some(workdir)
                };
                (wp, Some(hostname))
            }
            _ => (None, None),
        }
    } else {
        (None, None)
    };

    Some(PersistedWorkspace {
        endpoint_addr: encoded,
        session_id,
        last_project_path: project_path,
        last_hostname: hostname,
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    })
}
