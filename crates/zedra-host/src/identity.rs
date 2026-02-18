// Host identity management for iroh-based transport
//
// Loads or generates a persistent Ed25519 keypair for the host.
// The keypair serves as the host's long-term identity and is used
// directly as the iroh Endpoint secret key.

use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;

use zedra_transport::identity::{DeviceId, Keypair, PublicKey, SecretKey};

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
        tracing::info!("Endpoint ID: {}", keypair.iroh_public_key());
        tracing::debug!("Identity key: {}", key_path.display());

        Ok(Self { keypair, device_id })
    }

    /// Get the 32-byte Ed25519 public key bytes.
    pub fn public_key_bytes(&self) -> [u8; 32] {
        self.keypair.public_key_bytes()
    }

    /// Get the 32-byte secret key bytes.
    pub fn secret_key_bytes(&self) -> [u8; 32] {
        self.keypair.secret_key_bytes()
    }

    /// Get the iroh SecretKey (for Endpoint builder).
    pub fn iroh_secret_key(&self) -> &SecretKey {
        self.keypair.iroh_secret_key()
    }

    /// Get the iroh PublicKey / EndpointId.
    pub fn iroh_endpoint_id(&self) -> PublicKey {
        self.keypair.iroh_public_key()
    }
}

/// Get the identity key file path (~/.config/zedra-host/identity.key).
fn identity_key_path() -> Result<PathBuf> {
    let config = crate::store::config_dir()?;
    Ok(config.join("identity.key"))
}

/// Shared host identity, passed through the daemon.
pub type SharedIdentity = Arc<HostIdentity>;
