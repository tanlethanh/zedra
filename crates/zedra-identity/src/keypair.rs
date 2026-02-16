use anyhow::{Context, Result};
use rand::RngCore;
use std::path::Path;
use x25519_dalek::{PublicKey, StaticSecret};

/// A persistent Curve25519 keypair for device identity.
///
/// Because `StaticSecret` does not implement `Clone` or expose its raw bytes
/// after construction, we keep the original 32-byte secret alongside the
/// `StaticSecret` so we can serialize it back to disk.
pub struct Keypair {
    secret: StaticSecret,
    secret_bytes: [u8; 32],
    pub public: PublicKey,
}

impl Keypair {
    /// Generate a new random keypair.
    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        let secret = StaticSecret::from(bytes);
        let public = PublicKey::from(&secret);
        Self {
            secret,
            secret_bytes: bytes,
            public,
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
        let secret = StaticSecret::from(bytes);
        let public = PublicKey::from(&secret);
        Ok(Self {
            secret,
            secret_bytes: bytes,
            public,
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

    /// Get the public key as 32 raw bytes.
    pub fn public_key_bytes(&self) -> [u8; 32] {
        self.public.to_bytes()
    }

    /// Get the secret key as 32 raw bytes.
    pub fn secret_key_bytes(&self) -> [u8; 32] {
        self.secret_bytes
    }

    /// Borrow the `StaticSecret` (e.g. for Diffie-Hellman).
    pub fn secret(&self) -> &StaticSecret {
        &self.secret
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
}
