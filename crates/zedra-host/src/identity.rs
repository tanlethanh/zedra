// Host identity: persistent Ed25519 keypair for iroh endpoint.
//
// The 32-byte secret key is stored at ~/.config/zedra-host/identity.key
// and used directly as the iroh Endpoint secret key.

use anyhow::{Context, Result};
use std::path::PathBuf;
use std::sync::Arc;

/// Host identity: a persistent iroh SecretKey.
pub struct HostIdentity {
    secret: iroh::SecretKey,
}

impl HostIdentity {
    /// Load or generate the host's persistent identity.
    pub fn load_or_generate() -> Result<Self> {
        let key_path = identity_key_path()?;

        let secret = if key_path.exists() {
            let data = std::fs::read(&key_path)
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
            std::fs::write(&key_path, secret.to_bytes())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))?;
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

/// Get the identity key file path (~/.config/zedra-host/identity.key).
fn identity_key_path() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("dev", "zedra", "zedra-host")
        .ok_or_else(|| anyhow::anyhow!("Could not determine config directory"))?;
    let config = dirs.config_dir().to_path_buf();
    std::fs::create_dir_all(&config)?;
    Ok(config.join("identity.key"))
}

/// Shared host identity, passed through the daemon.
pub type SharedIdentity = Arc<HostIdentity>;
