// Authentication logic for zedra-host
// Supports OTP-based pairing tokens for Noise_IK handshake

use std::sync::Mutex;

/// Active pairing tokens (in-memory, expire after use)
static PAIRING_TOKENS: Mutex<Vec<PairingToken>> = Mutex::new(Vec::new());

struct PairingToken {
    token: String,
    created_at: std::time::Instant,
}

const TOKEN_EXPIRY_SECS: u64 = 300; // 5 minutes

/// Generate a new pairing token
pub fn create_pairing_token() -> String {
    let mut bytes = [0u8; 32];
    use rand::RngCore;
    rand::thread_rng().fill_bytes(&mut bytes);
    let token = hex::encode(bytes);

    let mut tokens = PAIRING_TOKENS.lock().unwrap();
    // Clean up expired tokens
    tokens.retain(|t| t.created_at.elapsed().as_secs() < TOKEN_EXPIRY_SECS);
    tokens.push(PairingToken {
        token: token.clone(),
        created_at: std::time::Instant::now(),
    });

    token
}

/// Validate and consume a pairing token (single use)
pub fn validate_pairing_token(token: &str) -> bool {
    let mut tokens = PAIRING_TOKENS.lock().unwrap();
    // Clean up expired tokens
    tokens.retain(|t| t.created_at.elapsed().as_secs() < TOKEN_EXPIRY_SECS);

    if let Some(pos) = tokens.iter().position(|t| t.token == token) {
        tokens.remove(pos); // Single use - consume the token
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_pairing_token_returns_hex_string() {
        let token = create_pairing_token();
        assert_eq!(token.len(), 64); // 32 bytes = 64 hex chars
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_pairing_tokens_are_unique() {
        let t1 = create_pairing_token();
        let t2 = create_pairing_token();
        assert_ne!(t1, t2);
    }

    #[test]
    fn test_validate_valid_token() {
        let token = create_pairing_token();
        assert!(validate_pairing_token(&token));
    }

    #[test]
    fn test_token_is_single_use() {
        let token = create_pairing_token();
        assert!(validate_pairing_token(&token));
        assert!(!validate_pairing_token(&token));
    }

    #[test]
    fn test_validate_invalid_token() {
        assert!(!validate_pairing_token("not-a-real-token"));
    }
}
