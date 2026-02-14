// Integration tests for zedra-host
// Tests host key generation and fingerprinting (used by QR pairing)

use russh_keys::{ssh_key, HashAlg};

#[tokio::test]
async fn test_host_key_generation() {
    // Test that we can generate and load Ed25519 keys
    let key = ssh_key::PrivateKey::random(&mut rand::thread_rng(), ssh_key::Algorithm::Ed25519)
        .expect("Failed to generate Ed25519 key");

    // Verify it can be serialized to OpenSSH format
    let openssh = key
        .to_openssh(ssh_key::LineEnding::LF)
        .expect("Failed to serialize key");

    assert!(!openssh.is_empty());
    assert!(openssh.contains("OPENSSH PRIVATE KEY"));

    // Verify roundtrip
    let loaded =
        ssh_key::PrivateKey::from_openssh(openssh.as_bytes()).expect("Failed to parse key back");

    let orig_fp = key.public_key().fingerprint(HashAlg::Sha256);
    let loaded_fp = loaded.public_key().fingerprint(HashAlg::Sha256);
    assert_eq!(orig_fp.to_string(), loaded_fp.to_string());
}

#[tokio::test]
async fn test_host_key_fingerprint_format() {
    let key =
        ssh_key::PrivateKey::random(&mut rand::thread_rng(), ssh_key::Algorithm::Ed25519).unwrap();

    let fingerprint = key.public_key().fingerprint(HashAlg::Sha256).to_string();

    // SHA256 fingerprints look like "SHA256:base64data"
    assert!(
        fingerprint.starts_with("SHA256:"),
        "Fingerprint should start with SHA256:, got: {}",
        fingerprint
    );
}
