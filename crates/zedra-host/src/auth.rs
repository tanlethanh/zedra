// Authentication logic for zedra-host
// Supports pairing token auth and password fallback

use anyhow::Result;
use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use std::sync::Mutex;

use crate::store;

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

/// Set the fallback password (hashed with argon2)
pub fn set_password(password: &str) -> Result<()> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("Failed to hash password: {}", e))?;

    let mut store_data = store::load_store()?;
    store_data.password_hash = Some(hash.to_string());
    store::save_store(&store_data)?;

    Ok(())
}

/// Verify a password against the stored hash
pub fn verify_password(password: &str) -> Result<bool> {
    let store_data = store::load_store()?;
    let hash_str = match store_data.password_hash {
        Some(h) => h,
        None => return Ok(false), // No password set
    };

    let parsed_hash =
        PasswordHash::new(&hash_str).map_err(|e| anyhow::anyhow!("Invalid hash: {}", e))?;
    let argon2 = Argon2::default();
    Ok(argon2
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok())
}

/// Authenticate a user
/// Returns true if authentication succeeds
pub fn authenticate(username: &str, password: &str) -> Result<bool> {
    match username {
        "zedra-pair" => {
            // Pairing token authentication
            Ok(validate_pairing_token(password))
        }
        "zedra" | _ => {
            // Password authentication
            verify_password(password)
        }
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

    #[test]
    fn test_authenticate_pair_user_with_valid_token() {
        let token = create_pairing_token();
        let result = authenticate("zedra-pair", &token).unwrap();
        assert!(result);
    }

    #[test]
    fn test_authenticate_pair_user_with_invalid_token() {
        let result = authenticate("zedra-pair", "invalid").unwrap();
        assert!(!result);
    }
}
