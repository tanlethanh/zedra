// Host identity management for ZTP/2
//
// Loads or generates a persistent Curve25519 keypair for the host.
// The keypair is the host's long-term identity — it persists across
// restarts and is included in QR codes for client pairing.

use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;

use zedra_identity::{DeviceId, Keypair};

/// Host identity: persistent keypair + derived device ID.
pub struct HostIdentity {
    pub keypair: Keypair,
    pub device_id: DeviceId,
}

impl HostIdentity {
    /// Load or generate the host's persistent identity.
    ///
    /// Key is stored at `~/.config/zedra-host/identity.key`.
    pub fn load_or_generate() -> Result<Self> {
        let key_path = identity_key_path()?;
        let keypair = Keypair::load_or_generate(&key_path)?;
        let device_id = DeviceId::from_public_key(&keypair.public_key_bytes());

        tracing::info!("Host identity: {}", device_id);
        tracing::debug!("Identity key: {}", key_path.display());

        Ok(Self { keypair, device_id })
    }

    /// Get the 32-byte public key for inclusion in QR codes and handshakes.
    pub fn public_key_bytes(&self) -> [u8; 32] {
        self.keypair.public_key_bytes()
    }

    /// Get the 32-byte secret key for Noise handshakes.
    pub fn secret_key_bytes(&self) -> [u8; 32] {
        self.keypair.secret_key_bytes()
    }
}

/// Get the identity key file path (~/.config/zedra-host/identity.key).
fn identity_key_path() -> Result<PathBuf> {
    let config = crate::store::config_dir()?;
    Ok(config.join("identity.key"))
}

/// Shared host identity, passed through the daemon.
pub type SharedIdentity = Arc<HostIdentity>;
