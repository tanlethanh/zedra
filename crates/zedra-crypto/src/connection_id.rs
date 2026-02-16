use rand::Rng;

/// A 32-byte Connection ID that uniquely identifies a session across transport changes.
/// Sent in plaintext on each frame so the receiver can route to the correct session
/// before decryption.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConnectionId([u8; 32]);

impl ConnectionId {
    /// Generate a random Connection ID.
    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill(&mut bytes);
        ConnectionId(bytes)
    }

    /// Create from raw bytes.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        ConnectionId(bytes)
    }

    /// Get the raw bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl std::fmt::Display for ConnectionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Show first 8 bytes as hex for logging
        for b in &self.0[..8] {
            write!(f, "{:02x}", b)?;
        }
        write!(f, "...")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_unique() {
        let id1 = ConnectionId::generate();
        let id2 = ConnectionId::generate();
        assert_ne!(id1, id2, "two generated IDs should differ");
    }

    #[test]
    fn from_bytes_roundtrip() {
        let mut bytes = [0u8; 32];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = i as u8;
        }
        let id = ConnectionId::from_bytes(bytes);
        assert_eq!(id.as_bytes(), &bytes);
    }

    #[test]
    fn display_format() {
        let bytes = [0xab; 32];
        let id = ConnectionId::from_bytes(bytes);
        let display = format!("{}", id);
        assert_eq!(display, "abababababababab...");
    }
}
