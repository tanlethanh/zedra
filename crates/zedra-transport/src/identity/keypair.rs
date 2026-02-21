use anyhow::{Context, Result};
use iroh::SecretKey;
use std::path::Path;

/// A persistent Ed25519 keypair for device identity.
///
/// Wraps iroh's `SecretKey` (Ed25519 via ed25519-dalek) for use as both the
/// device identity and the iroh Endpoint secret key. The 32-byte secret is
/// stored on disk in the same raw format as the previous X25519 keys.
///
/// Note: Migrating from X25519 to Ed25519 means existing keys produce a
/// different public key (Edwards vs Montgomery form). Devices that were
/// paired with the old key format will need to re-pair.
pub struct Keypair {
    secret: SecretKey,
    secret_bytes: [u8; 32],
}

impl Keypair {
    /// Generate a new random keypair.
    pub fn generate() -> Self {
        let secret = SecretKey::generate(&mut rand::rng());
        let secret_bytes = secret.to_bytes();
        Self {
            secret,
            secret_bytes,
        }
    }

    /// Load a keypair from disk.
    ///
    /// File format: 32 bytes of raw secret key.
    pub fn load(secret_path: &Path) -> Result<Self> {
        let data = std::fs::read(secret_path)
            .with_context(|| format!("failed to read keypair from {}", secret_path.display()))?;
        if data.len() != 32 {
            anyhow::bail!(
                "invalid keypair file: expected 32 bytes, got {}",
                data.len()
            );
        }
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&data);
        let secret = SecretKey::from_bytes(&bytes);
        Ok(Self {
            secret,
            secret_bytes: bytes,
        })
    }

    /// Save the keypair to disk.
    ///
    /// Creates parent directories if they do not exist.
    /// On Unix, sets file permissions to 0o600 (owner read/write only).
    pub fn save(&self, secret_path: &Path) -> Result<()> {
        if let Some(parent) = secret_path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create parent directory {}",
                    parent.display()
                )
            })?;
        }
        std::fs::write(secret_path, &self.secret_bytes)
            .with_context(|| format!("failed to write keypair to {}", secret_path.display()))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(secret_path, perms).with_context(|| {
                format!(
                    "failed to set permissions on {}",
                    secret_path.display()
                )
            })?;
        }

        Ok(())
    }

    /// Load or generate a keypair.
    ///
    /// If a file exists at `secret_path`, loads the keypair from it.
    /// Otherwise, generates a new keypair and saves it.
    pub fn load_or_generate(secret_path: &Path) -> Result<Self> {
        if secret_path.exists() {
            Self::load(secret_path)
        } else {
            let kp = Self::generate();
            kp.save(secret_path)?;
            Ok(kp)
        }
    }

    /// Get the Ed25519 public key as 32 raw bytes.
    pub fn public_key_bytes(&self) -> [u8; 32] {
        *self.secret.public().as_bytes()
    }

    /// Get the secret key as 32 raw bytes.
    pub fn secret_key_bytes(&self) -> [u8; 32] {
        self.secret_bytes
    }

    /// Get the iroh `SecretKey` (for use with iroh Endpoint builder).
    pub fn iroh_secret_key(&self) -> &SecretKey {
        &self.secret
    }

    /// Get the iroh `PublicKey` / `EndpointId`.
    pub fn iroh_public_key(&self) -> iroh::PublicKey {
        self.secret.public()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn generate_produces_valid_keypair() {
        let kp = Keypair::generate();
        assert_ne!(kp.public_key_bytes(), [0u8; 32]);
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("secret.key");

        let kp1 = Keypair::generate();
        kp1.save(&path).unwrap();

        let kp2 = Keypair::load(&path).unwrap();
        assert_eq!(kp1.public_key_bytes(), kp2.public_key_bytes());
        assert_eq!(kp1.secret_key_bytes(), kp2.secret_key_bytes());
    }

    #[test]
    fn load_or_generate_creates_new() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("subdir/secret.key");

        assert!(!path.exists());
        let kp = Keypair::load_or_generate(&path).unwrap();
        assert!(path.exists());
        assert_ne!(kp.public_key_bytes(), [0u8; 32]);
    }

    #[test]
    fn load_or_generate_loads_existing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("secret.key");

        let kp1 = Keypair::generate();
        kp1.save(&path).unwrap();

        let kp2 = Keypair::load_or_generate(&path).unwrap();
        assert_eq!(kp1.public_key_bytes(), kp2.public_key_bytes());
        assert_eq!(kp1.secret_key_bytes(), kp2.secret_key_bytes());
    }

    #[test]
    fn iroh_key_roundtrip() {
        let kp = Keypair::generate();
        let pk = kp.iroh_public_key();
        assert_eq!(*pk.as_bytes(), kp.public_key_bytes());
    }
}
