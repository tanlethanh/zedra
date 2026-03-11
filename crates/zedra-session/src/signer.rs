// Client signing abstraction for PKI authentication.
//
// Decouples the application signing key from the iroh transport key.
// Designed to allow future hardware-backed implementations
// (Android Keystore, iOS Secure Enclave) without changing the API.

use anyhow::Result;
use ed25519_dalek::{Signer, SigningKey};

/// Application-layer signing for client authentication.
///
/// This key represents the device's persistent identity in the Zedra PKI.
/// It is stored in the host's per-session ACL after first pairing, and is
/// used to sign challenge nonces on every subsequent reconnect.
///
/// Separate from the iroh transport key, which rotates per-connection.
pub trait ClientSigner: Send + Sync {
    /// The client's Ed25519 public key (32 bytes).
    /// Stored by the host in authorized_clients and per-session ACL.
    fn pubkey(&self) -> [u8; 32];

    /// Sign `data` with the client's private key.
    /// Returns a 64-byte Ed25519 signature.
    fn sign(&self, data: &[u8]) -> [u8; 64];
}

/// File-backed client signer.
///
/// Stores the raw 32-byte Ed25519 secret key on disk with 0o600 permissions.
/// The key is generated once and reused across app restarts.
pub struct FileClientSigner {
    signing_key: SigningKey,
}

impl FileClientSigner {
    /// Load the client key from `path`, or generate and persist a new one.
    ///
    /// Default path: `~/.config/zedra/client.key`
    /// Android path: `<app_data_dir>/client.key`
    pub fn load_or_generate(path: &std::path::Path) -> Result<Self> {
        let signing_key = if path.exists() {
            let bytes = std::fs::read(path)?;
            if bytes.len() != 32 {
                anyhow::bail!(
                    "invalid client key at {}: expected 32 bytes, got {}",
                    path.display(),
                    bytes.len()
                );
            }
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            SigningKey::from_bytes(&arr)
        } else {
            let signing_key = SigningKey::generate(&mut rand::thread_rng());
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, signing_key.to_bytes())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(
                    path,
                    std::fs::Permissions::from_mode(0o600),
                )?;
            }
            tracing::info!("Generated new client key at {}", path.display());
            signing_key
        };

        tracing::info!(
            "Client pubkey loaded (first 8 bytes: {:?})",
            &signing_key.verifying_key().to_bytes()[..8]
        );
        Ok(Self { signing_key })
    }
}

impl ClientSigner for FileClientSigner {
    fn pubkey(&self) -> [u8; 32] {
        self.signing_key.verifying_key().to_bytes()
    }

    fn sign(&self, data: &[u8]) -> [u8; 64] {
        self.signing_key.sign(data).to_bytes()
    }
}
