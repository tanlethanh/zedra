use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A trusted peer (either a client trusted by host, or a host trusted by client).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustedPeer {
    /// Human-readable device ID (8 groups of 7 chars).
    pub device_id: String,
    /// Base64url-encoded 32-byte Curve25519 public key.
    pub public_key: String,
    /// User-provided or auto-generated peer name.
    pub name: String,
    /// When this peer was first paired.
    pub paired_at: DateTime<Utc>,
    /// When this peer was last seen online.
    pub last_seen: Option<DateTime<Utc>>,
    /// For hosts: optional coordination server URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coord_url: Option<String>,
    /// Whether this peer has been revoked.
    #[serde(default)]
    pub revoked: bool,
}

/// Persistent trust store backed by a JSON file.
///
/// Manages the set of known and trusted peers. Changes are written to disk
/// via the `save()` method.
pub struct TrustStore {
    path: PathBuf,
    peers: Vec<TrustedPeer>,
}

impl TrustStore {
    /// Load or create a trust store at the given path.
    ///
    /// If the file exists, deserializes the JSON contents.
    /// If the file does not exist, starts with an empty peer list.
    pub fn load(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let peers = if path.exists() {
            let data = std::fs::read_to_string(&path)
                .with_context(|| format!("failed to read trust store from {}", path.display()))?;
            serde_json::from_str(&data)
                .with_context(|| format!("failed to parse trust store at {}", path.display()))?
        } else {
            Vec::new()
        };
        Ok(Self { path, peers })
    }

    /// Save the current state to disk.
    ///
    /// Creates parent directories if they do not exist.
    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create parent directory {}",
                    parent.display()
                )
            })?;
        }
        let json = serde_json::to_string_pretty(&self.peers)
            .context("failed to serialize trust store")?;
        std::fs::write(&self.path, json)
            .with_context(|| format!("failed to write trust store to {}", self.path.display()))?;
        Ok(())
    }

    /// Add a new trusted peer.
    ///
    /// Returns an error if a peer with the same `public_key` already exists.
    pub fn add_peer(&mut self, peer: TrustedPeer) -> Result<()> {
        if self
            .peers
            .iter()
            .any(|p| p.public_key == peer.public_key)
        {
            anyhow::bail!(
                "peer with public key {} already exists",
                peer.public_key
            );
        }
        self.peers.push(peer);
        Ok(())
    }

    /// Look up a peer by their base64url-encoded public key.
    pub fn find_by_public_key(&self, public_key: &str) -> Option<&TrustedPeer> {
        self.peers.iter().find(|p| p.public_key == public_key)
    }

    /// Look up a peer by device ID.
    pub fn find_by_device_id(&self, device_id: &str) -> Option<&TrustedPeer> {
        self.peers.iter().find(|p| p.device_id == device_id)
    }

    /// Update `last_seen` to now for the peer with the given public key.
    pub fn touch_peer(&mut self, public_key: &str) {
        if let Some(peer) = self.peers.iter_mut().find(|p| p.public_key == public_key) {
            peer.last_seen = Some(Utc::now());
        }
    }

    /// Revoke a peer by device ID.
    ///
    /// Returns `true` if the peer was found and revoked, `false` if not found.
    pub fn revoke_peer(&mut self, device_id: &str) -> bool {
        if let Some(peer) = self.peers.iter_mut().find(|p| p.device_id == device_id) {
            peer.revoked = true;
            true
        } else {
            false
        }
    }

    /// Check if a base64url-encoded public key belongs to a trusted (non-revoked) peer.
    pub fn is_trusted(&self, public_key: &str) -> bool {
        self.peers
            .iter()
            .any(|p| p.public_key == public_key && !p.revoked)
    }

    /// List all peers.
    pub fn peers(&self) -> &[TrustedPeer] {
        &self.peers
    }

    /// Get the default trust store path for the host daemon.
    ///
    /// Uses the platform-specific config directory:
    /// - Linux: `~/.config/zedra-host/trust.json`
    /// - macOS: `~/Library/Application Support/dev.zedra.zedra-host/trust.json`
    pub fn default_host_path() -> Result<PathBuf> {
        let dirs = directories::ProjectDirs::from("dev", "zedra", "zedra-host")
            .ok_or_else(|| anyhow::anyhow!("no config directory found for this platform"))?;
        Ok(dirs.config_dir().join("trust.json"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use tempfile::TempDir;

    fn make_peer(name: &str, public_key: &str, device_id: &str) -> TrustedPeer {
        TrustedPeer {
            device_id: device_id.to_string(),
            public_key: public_key.to_string(),
            name: name.to_string(),
            paired_at: Utc::now(),
            last_seen: None,
            coord_url: None,
            revoked: false,
        }
    }

    #[test]
    fn add_and_find_peer() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("trust.json");
        let mut store = TrustStore::load(&path).unwrap();

        let peer = make_peer("laptop", "abc123key", "AAAAAAA-BBBBBBB-CCCCCCC-DDDDDDD-EEEEEEE-FFFFFFF-GGGGGGG-HHHHHHH");
        store.add_peer(peer).unwrap();

        let found = store.find_by_public_key("abc123key");
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "laptop");
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("trust.json");

        {
            let mut store = TrustStore::load(&path).unwrap();
            store
                .add_peer(make_peer("laptop", "key1", "ID1AAAA-ID1BBBB-ID1CCCC-ID1DDDD-ID1EEEE-ID1FFFF-ID1GGGG-ID1HHHH"))
                .unwrap();
            store
                .add_peer(make_peer("phone", "key2", "ID2AAAA-ID2BBBB-ID2CCCC-ID2DDDD-ID2EEEE-ID2FFFF-ID2GGGG-ID2HHHH"))
                .unwrap();
            store.save().unwrap();
        }

        let store = TrustStore::load(&path).unwrap();
        assert_eq!(store.peers().len(), 2);
        assert!(store.find_by_public_key("key1").is_some());
        assert!(store.find_by_public_key("key2").is_some());
        assert_eq!(store.find_by_public_key("key1").unwrap().name, "laptop");
        assert_eq!(store.find_by_public_key("key2").unwrap().name, "phone");
    }

    #[test]
    fn revoke_peer() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("trust.json");
        let mut store = TrustStore::load(&path).unwrap();

        let device_id = "AAAAAAA-BBBBBBB-CCCCCCC-DDDDDDD-EEEEEEE-FFFFFFF-GGGGGGG-HHHHHHH";
        store
            .add_peer(make_peer("laptop", "key1", device_id))
            .unwrap();

        assert!(store.is_trusted("key1"));
        assert!(store.revoke_peer(device_id));
        assert!(!store.is_trusted("key1"));

        // The peer still exists, just revoked
        let peer = store.find_by_public_key("key1").unwrap();
        assert!(peer.revoked);
    }

    #[test]
    fn find_by_device_id() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("trust.json");
        let mut store = TrustStore::load(&path).unwrap();

        let device_id = "AAAAAAA-BBBBBBB-CCCCCCC-DDDDDDD-EEEEEEE-FFFFFFF-GGGGGGG-HHHHHHH";
        store
            .add_peer(make_peer("laptop", "key1", device_id))
            .unwrap();

        let found = store.find_by_device_id(device_id);
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "laptop");

        assert!(store.find_by_device_id("NONEXISTENT").is_none());
    }

    #[test]
    fn duplicate_peer_rejected() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("trust.json");
        let mut store = TrustStore::load(&path).unwrap();

        store
            .add_peer(make_peer("laptop", "samekey", "ID1AAAA-ID1BBBB-ID1CCCC-ID1DDDD-ID1EEEE-ID1FFFF-ID1GGGG-ID1HHHH"))
            .unwrap();

        let result = store.add_peer(make_peer("phone", "samekey", "ID2AAAA-ID2BBBB-ID2CCCC-ID2DDDD-ID2EEEE-ID2FFFF-ID2GGGG-ID2HHHH"));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("already exists")
        );
    }
}
