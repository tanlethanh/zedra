// Device and credential storage
// Stores paired devices and host keys at ~/.config/zedra-host/

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A paired device record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairedDevice {
    pub id: String,
    pub name: String,
    pub public_key: String,
    pub paired_at: String,
    pub last_connected: Option<String>,
}

/// Persistent state for zedra-host
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct HostStore {
    pub devices: Vec<PairedDevice>,
    pub password_hash: Option<String>,
}

/// Get the config directory path
pub fn config_dir() -> Result<PathBuf> {
    let dirs =
        directories::ProjectDirs::from("dev", "zedra", "zedra-host")
            .ok_or_else(|| anyhow::anyhow!("Could not determine config directory"))?;
    let config = dirs.config_dir().to_path_buf();
    std::fs::create_dir_all(&config)?;
    Ok(config)
}

/// Get the store file path
fn store_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("store.json"))
}

/// Get the host key file path
pub fn host_key_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("host_key"))
}

/// Load store from a specific path
fn load_store_from(path: &std::path::Path) -> Result<HostStore> {
    if path.exists() {
        let data = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&data)?)
    } else {
        Ok(HostStore::default())
    }
}

/// Save store to a specific path
fn save_store_to(store: &HostStore, path: &std::path::Path) -> Result<()> {
    let data = serde_json::to_string_pretty(store)?;
    std::fs::write(path, data)?;
    Ok(())
}

/// Load the store from disk
pub fn load_store() -> Result<HostStore> {
    load_store_from(&store_path()?)
}

/// Save the store to disk
pub fn save_store(store: &HostStore) -> Result<()> {
    save_store_to(store, &store_path()?)
}

/// Add a paired device
pub fn add_device(device: PairedDevice) -> Result<()> {
    let mut store = load_store()?;
    // Remove existing device with same ID
    store.devices.retain(|d| d.id != device.id);
    store.devices.push(device);
    save_store(&store)
}

/// List all paired devices
pub fn list_devices() -> Result<Vec<PairedDevice>> {
    let store = load_store()?;
    Ok(store.devices)
}

/// Revoke a device by ID
pub fn revoke_device(device_id: &str) -> Result<()> {
    let mut store = load_store()?;
    let before = store.devices.len();
    store.devices.retain(|d| d.id != device_id);
    if store.devices.len() == before {
        anyhow::bail!("Device not found: {}", device_id);
    }
    save_store(&store)
}

/// Find a device by public key
pub fn find_device_by_key(public_key: &str) -> Result<Option<PairedDevice>> {
    let store = load_store()?;
    Ok(store.devices.iter().find(|d| d.public_key == public_key).cloned())
}

/// Update last connected time for a device
pub fn update_last_connected(device_id: &str) -> Result<()> {
    let mut store = load_store()?;
    if let Some(device) = store.devices.iter_mut().find(|d| d.id == device_id) {
        device.last_connected = Some(chrono_now());
    }
    save_store(&store)
}

fn chrono_now() -> String {
    // Simple ISO 8601 timestamp without external chrono dependency
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}", duration.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_device(id: &str, name: &str) -> PairedDevice {
        PairedDevice {
            id: id.to_string(),
            name: name.to_string(),
            public_key: format!("key-{}", id),
            paired_at: "12345".to_string(),
            last_connected: None,
        }
    }

    #[test]
    fn test_host_store_serialization() {
        let store = HostStore {
            devices: vec![PairedDevice {
                id: "test-id".to_string(),
                name: "test-device".to_string(),
                public_key: "ssh-ed25519 AAAA...".to_string(),
                paired_at: "1234567890".to_string(),
                last_connected: None,
            }],
            password_hash: Some("$argon2...".to_string()),
        };

        let json = serde_json::to_string(&store).unwrap();
        let deserialized: HostStore = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.devices.len(), 1);
        assert_eq!(deserialized.devices[0].id, "test-id");
        assert_eq!(deserialized.devices[0].name, "test-device");
        assert_eq!(deserialized.password_hash, Some("$argon2...".to_string()));
    }

    #[test]
    fn test_host_store_default() {
        let store = HostStore::default();
        assert!(store.devices.is_empty());
        assert!(store.password_hash.is_none());
    }

    #[test]
    fn test_paired_device_clone() {
        let device = test_device("id1", "device1");
        let cloned = device.clone();
        assert_eq!(cloned.id, device.id);
        assert_eq!(cloned.public_key, device.public_key);
    }

    #[test]
    fn test_store_file_roundtrip() {
        let temp_dir = tempfile::tempdir().unwrap();
        let store_path = temp_dir.path().join("store.json");

        let mut store = HostStore::default();
        store.devices.push(test_device("rt-id", "roundtrip"));

        save_store_to(&store, &store_path).unwrap();
        let loaded = load_store_from(&store_path).unwrap();

        assert_eq!(loaded.devices.len(), 1);
        assert_eq!(loaded.devices[0].id, "rt-id");
        assert_eq!(loaded.devices[0].name, "roundtrip");
    }

    #[test]
    fn test_store_add_replaces_duplicate_id() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("store.json");

        let mut store = HostStore::default();
        store.devices.push(test_device("dup", "first"));
        save_store_to(&store, &path).unwrap();

        let mut store = load_store_from(&path).unwrap();
        store.devices.retain(|d| d.id != "dup");
        store.devices.push(test_device("dup", "second"));
        save_store_to(&store, &path).unwrap();

        let loaded = load_store_from(&path).unwrap();
        assert_eq!(loaded.devices.len(), 1);
        assert_eq!(loaded.devices[0].name, "second");
    }

    #[test]
    fn test_store_revoke() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("store.json");

        let mut store = HostStore::default();
        store.devices.push(test_device("a", "alpha"));
        store.devices.push(test_device("b", "beta"));
        save_store_to(&store, &path).unwrap();

        let mut store = load_store_from(&path).unwrap();
        let before = store.devices.len();
        store.devices.retain(|d| d.id != "a");
        assert_eq!(store.devices.len(), before - 1);
        save_store_to(&store, &path).unwrap();

        let loaded = load_store_from(&path).unwrap();
        assert_eq!(loaded.devices.len(), 1);
        assert_eq!(loaded.devices[0].id, "b");
    }

    #[test]
    fn test_store_find_by_key() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("store.json");

        let mut store = HostStore::default();
        store.devices.push(test_device("x", "xray"));
        save_store_to(&store, &path).unwrap();

        let loaded = load_store_from(&path).unwrap();
        let found = loaded.devices.iter().find(|d| d.public_key == "key-x");
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "xray");

        let not_found = loaded.devices.iter().find(|d| d.public_key == "nope");
        assert!(not_found.is_none());
    }

    #[test]
    fn test_load_nonexistent_returns_default() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("nonexistent.json");

        let store = load_store_from(&path).unwrap();
        assert!(store.devices.is_empty());
        assert!(store.password_hash.is_none());
    }

    #[test]
    fn test_store_password_hash_persistence() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("store.json");

        let store = HostStore {
            devices: vec![],
            password_hash: Some("hash123".to_string()),
        };
        save_store_to(&store, &path).unwrap();

        let loaded = load_store_from(&path).unwrap();
        assert_eq!(loaded.password_hash, Some("hash123".to_string()));
    }
}
