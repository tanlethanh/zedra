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
            std::fs::write(key_path, secret.to_bytes())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(key_path, std::fs::Permissions::from_mode(0o600))?;
            }
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
    let workdir_str = workdir.to_string_lossy();
    let hash = {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        workdir_str.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    };
    let path = zedra_config_dir()?
        .join("workspaces")
        .join(&hash)
        .join("identity.key");
    Ok(path)
}

/// Shared host identity, passed through the daemon.
pub type SharedIdentity = Arc<HostIdentity>;
