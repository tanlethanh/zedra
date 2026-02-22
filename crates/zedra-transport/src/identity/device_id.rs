use sha2::{Digest, Sha256};

/// A human-readable device identifier derived from an Ed25519 public key.
///
/// Format: 8 groups of 7 characters, base32-encoded (RFC 4648, no padding),
/// separated by hyphens.
///
/// Example: `RLKQ4WE-GLLHZT5-7QFG3G2-VFI3HTG-XFQTPNL-BNVHJ6Q-WDHYQFP-XWIQTAH`
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct DeviceId(String);

impl DeviceId {
    /// Create a `DeviceId` from a 32-byte Ed25519 public key.
    ///
    /// The key is SHA-256 hashed. Since SHA-256 produces only 32 bytes but
    /// we need 35 bytes for 56 base32 characters (8 groups of 7), we extend
    /// by hashing the first hash and appending 3 bytes.
    pub fn from_public_key(public_key: &[u8; 32]) -> Self {
        let hash1 = Sha256::digest(public_key);
        let hash2 = Sha256::digest(&hash1);
        // Concatenate hash1 (32 bytes) + first 3 bytes of hash2 = 35 bytes
        let mut material = [0u8; 35];
        material[..32].copy_from_slice(&hash1);
        material[32..35].copy_from_slice(&hash2[..3]);
        // 35 bytes -> ceil(35*8/5) = 56 base32 characters -> 8 groups of 7
        let encoded = base32::encode(base32::Alphabet::Rfc4648 { padding: false }, &material);
        let chunks: Vec<&str> = encoded
            .as_bytes()
            .chunks(7)
            .map(|c| std::str::from_utf8(c).unwrap_or(""))
            .take(8)
            .collect();
        DeviceId(chunks.join("-"))
    }

    /// Parse a `DeviceId` from its string representation.
    ///
    /// Validates the format: 8 groups of 7 ASCII alphanumeric characters
    /// separated by hyphens.
    pub fn parse(s: &str) -> Result<Self, anyhow::Error> {
        let upper = s.to_uppercase();
        let parts: Vec<&str> = upper.split('-').collect();
        if parts.len() != 8 {
            anyhow::bail!(
                "invalid device ID format: expected 8 groups, got {}",
                parts.len()
            );
        }
        for (i, part) in parts.iter().enumerate() {
            if part.len() != 7 {
                anyhow::bail!(
                    "invalid device ID format: group {} has {} chars, expected 7",
                    i,
                    part.len()
                );
            }
            if !part.chars().all(|c| c.is_ascii_alphanumeric()) {
                anyhow::bail!(
                    "invalid device ID format: group {} contains non-alphanumeric characters",
                    i
                );
            }
        }
        Ok(DeviceId(upper))
    }

    /// Get the short form (first two groups, e.g. `RLKQ4WE-GLLHZT5`).
    pub fn short(&self) -> &str {
        let end = 15.min(self.0.len());
        &self.0[..end]
    }

    /// Get the full device ID string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for DeviceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> [u8; 32] {
        let mut key = [0u8; 32];
        for (i, b) in key.iter_mut().enumerate() {
            *b = i as u8;
        }
        key
    }

    #[test]
    fn from_public_key_format() {
        let id = DeviceId::from_public_key(&test_key());
        let s = id.as_str();
        let parts: Vec<&str> = s.split('-').collect();
        assert_eq!(parts.len(), 8, "expected 8 groups, got: {}", s);
        for (i, part) in parts.iter().enumerate() {
            assert_eq!(
                part.len(),
                7,
                "group {} has {} chars: {}",
                i,
                part.len(),
                part
            );
            assert!(
                part.chars().all(|c| c.is_ascii_alphanumeric()),
                "group {} contains non-alphanumeric chars: {}",
                i,
                part
            );
        }
    }

    #[test]
    fn same_key_same_id() {
        let key = test_key();
        let id1 = DeviceId::from_public_key(&key);
        let id2 = DeviceId::from_public_key(&key);
        assert_eq!(id1, id2);
    }

    #[test]
    fn different_keys_different_ids() {
        let key1 = test_key();
        let mut key2 = test_key();
        key2[0] = 0xFF;
        let id1 = DeviceId::from_public_key(&key1);
        let id2 = DeviceId::from_public_key(&key2);
        assert_ne!(id1, id2);
    }

    #[test]
    fn parse_roundtrip() {
        let key = test_key();
        let id1 = DeviceId::from_public_key(&key);
        let parsed = DeviceId::parse(id1.as_str()).unwrap();
        assert_eq!(id1, parsed);
    }

    #[test]
    fn short_form() {
        let id = DeviceId::from_public_key(&test_key());
        let short = id.short();
        let full = id.as_str();
        // Short should be first 15 chars: "XXXXXXX-XXXXXXX"
        assert_eq!(short.len(), 15);
        assert_eq!(short, &full[..15]);
        assert_eq!(short.matches('-').count(), 1);
    }

    #[test]
    fn parse_rejects_invalid() {
        assert!(DeviceId::parse("too-short").is_err());
        assert!(DeviceId::parse("AAAAAAA-BBBBBBB").is_err()); // only 2 groups
        assert!(
            DeviceId::parse("AAAAAA-BBBBBBB-CCCCCCC-DDDDDDD-EEEEEEE-FFFFFFF-GGGGGGG-HHHHHHH")
                .is_err()
        ); // group 0 is 6 chars
    }
}
