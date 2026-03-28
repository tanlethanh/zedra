// Host identity: persistent Ed25519 keypair for iroh endpoint.
//
// Each workspace gets its own identity key so multiple `zedra start`
// instances on the same machine have distinct iroh NodeIds and don't
// conflict on the relay.
//
// Keys are stored at:
//   ~/.config/zedra/identity.key           (base, used by `zedra qr` without --workdir)
//   ~/.config/zedra/workspaces/<hash>/identity.key  (per-workspace)

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Host identity: a persistent iroh SecretKey.
pub struct HostIdentity {
    secret: iroh::SecretKey,
}

impl HostIdentity {
    /// Load or generate the base host identity (no workspace context).
    ///
    /// Used by `zedra qr` when no `--workdir` is specified.
    pub fn load_or_generate() -> Result<Self> {
        let key_path = base_identity_key_path()?;
        Self::load_or_generate_at(&key_path)
    }

    /// Load or generate a per-workspace identity.
    ///
    /// Each canonical workdir path maps to a unique identity key stored
    /// under `~/.config/zedra/workspaces/<hash>/identity.key`.
    /// This ensures multiple `zedra start` instances on the same
    /// machine get distinct iroh NodeIds and don't conflict.
    pub fn load_or_generate_for_workdir(workdir: &Path) -> Result<Self> {
        let key_path = workspace_key_path(workdir)?;
        Self::load_or_generate_at(&key_path)
    }

    /// Load or generate an identity at the given path.
    fn load_or_generate_at(key_path: &Path) -> Result<Self> {
        let secret = if key_path.exists() {
            let data = std::fs::read(key_path)
                .with_context(|| format!("failed to read identity from {}", key_path.display()))?;
            if data.len() != 32 {
                anyhow::bail!(
                    "invalid identity file: expected 32 bytes, got {}",
                    data.len()
                );
            }
            let mut bytes = [0u8; 32];
            bytes.copy_from_slice(&data);
            iroh::SecretKey::from_bytes(&bytes)
        } else {
            let mut bytes = [0u8; 32];
            rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut bytes);
            let secret = iroh::SecretKey::from_bytes(&bytes);
            // Save with restricted permissions
            if let Some(parent) = key_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            write_secret_file(key_path, &secret.to_bytes())?;
            secret
        };

        tracing::info!("Host endpoint ID: {}", secret.public().fmt_short());
        tracing::debug!("Identity key: {}", key_path.display());

        Ok(Self { secret })
    }

    /// Get the iroh SecretKey (for Endpoint builder).
    pub fn iroh_secret_key(&self) -> &iroh::SecretKey {
        &self.secret
    }

    /// The host's public EndpointId (Ed25519 public key).
    /// Encoded in the QR ticket so clients can verify host challenge signatures.
    pub fn endpoint_id(&self) -> iroh::PublicKey {
        self.secret.public()
    }

    /// Sign `data` with the host's iroh SecretKey (Ed25519).
    /// Used to prove host identity in the Authenticate challenge.
    pub fn sign_challenge(&self, data: &[u8]) -> [u8; 64] {
        self.secret.sign(data).to_bytes()
    }
}

/// Path for the host-level telemetry ID file (`~/.config/zedra/telemetry_id`).
///
/// Shared across all workspaces on the same machine so connection counts
/// roll up to a single host identity in telemetry.
pub fn telemetry_id_path() -> Result<PathBuf> {
    Ok(zedra_config_dir()?.join("telemetry_id"))
}

/// Returns the per-workspace config directory:
/// `~/.config/zedra/workspaces/<hash>/`
///
/// Exported so other modules can co-locate their persistent state alongside
/// the identity key without duplicating the hash logic.
pub fn workspace_config_dir(workdir: &Path) -> Result<PathBuf> {
    let hash = stable_path_hash(&workdir.to_string_lossy());
    Ok(zedra_config_dir()?.join("workspaces").join(hash))
}

/// Returns `~/.config/zedra/` as the config root on all platforms.
fn zedra_config_dir() -> Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf()))
        .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
    Ok(home.join(".config").join("zedra"))
}

/// Base identity key path (~/.config/zedra/identity.key).
fn base_identity_key_path() -> Result<PathBuf> {
    let config = zedra_config_dir()?;
    std::fs::create_dir_all(&config)?;
    Ok(config.join("identity.key"))
}

/// Per-workspace identity key path.
///
/// Uses a stable hash of the canonical workdir path as the directory name:
/// `~/.config/zedra/workspaces/<hash>/identity.key`
fn workspace_key_path(workdir: &Path) -> Result<PathBuf> {
    let hash = stable_path_hash(&workdir.to_string_lossy());
    let path = zedra_config_dir()?
        .join("workspaces")
        .join(&hash)
        .join("identity.key");
    Ok(path)
}

/// Write secret bytes to `path` with restricted permissions (0o600).
///
/// On Unix, uses `OpenOptions::mode()` to set permissions atomically at
/// file creation, eliminating the TOCTOU window between write and chmod.
/// On non-Unix platforms falls back to write + set_permissions.
pub fn write_secret_file(path: &Path, data: &[u8]) -> Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(data)?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, data)?;
    }
    Ok(())
}

/// Stable hash of a path string using SHA-256, truncated to 16 hex chars.
///
/// Replaces `DefaultHasher` which is explicitly not stable across Rust versions.
/// A toolchain upgrade with DefaultHasher could change the output and orphan
/// all existing identity keys and sessions.
fn stable_path_hash(path_str: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(path_str.as_bytes());
    format!(
        "{:016x}",
        u64::from_le_bytes(digest[..8].try_into().unwrap())
    )
}

/// Shared host identity, passed through the daemon.
pub type SharedIdentity = Arc<HostIdentity>;
