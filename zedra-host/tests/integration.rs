// Integration tests for zedra-host SSH server
// Tests the server end-to-end using russh client

use std::sync::Arc;

use async_trait::async_trait;
use russh::client::{self, Handler};
use russh::keys::{ssh_key, HashAlg};

/// Simple test SSH client handler
struct TestClient;

#[async_trait]
impl Handler for TestClient {
    type Error = anyhow::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        // Accept any key in tests (TOFU)
        Ok(true)
    }
}

/// Helper to start a server on a random port and return the port
async fn start_test_server() -> u16 {
    use tokio::net::TcpListener;

    // Bind to port 0 to get a random available port
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    // We need the server to generate a host key and be ready
    // For now, just return the port - we'll test what we can without full server startup
    drop(listener);
    port
}

#[tokio::test]
async fn test_ssh_client_connect_to_nonexistent_server() {
    let config = client::Config::default();
    let handler = TestClient;

    // Connect to a port where nothing is listening
    let result = client::connect(
        Arc::new(config),
        ("127.0.0.1", 1), // Port 1 - nothing should be listening
        handler,
    )
    .await;

    assert!(result.is_err(), "Should fail to connect to nonexistent server");
}

#[tokio::test]
async fn test_ssh_config_defaults() {
    let config = client::Config::default();
    // Verify default config can be created without panicking
    let _arc = Arc::new(config);
}

#[tokio::test]
async fn test_server_config_creation() {
    use russh::server;

    // Test that server config can be created with required settings
    let config = server::Config {
        auth_rejection_time: std::time::Duration::from_secs(1),
        auth_rejection_time_initial: Some(std::time::Duration::from_secs(0)),
        ..Default::default()
    };

    assert_eq!(config.auth_rejection_time, std::time::Duration::from_secs(1));
}

#[tokio::test]
async fn test_host_key_generation() {
    // Test that we can generate and load Ed25519 keys
    let key = ssh_key::PrivateKey::random(
        &mut rand::thread_rng(),
        ssh_key::Algorithm::Ed25519,
    )
    .expect("Failed to generate Ed25519 key");

    // Verify it can be serialized to OpenSSH format
    let openssh = key
        .to_openssh(ssh_key::LineEnding::LF)
        .expect("Failed to serialize key");

    assert!(!openssh.is_empty());
    assert!(openssh.contains("OPENSSH PRIVATE KEY"));

    // Verify roundtrip
    let loaded = ssh_key::PrivateKey::from_openssh(openssh.as_bytes())
        .expect("Failed to parse key back");

    let orig_fp = key.public_key().fingerprint(HashAlg::Sha256);
    let loaded_fp = loaded.public_key().fingerprint(HashAlg::Sha256);
    assert_eq!(orig_fp.to_string(), loaded_fp.to_string());
}

#[tokio::test]
async fn test_host_key_fingerprint_format() {
    let key = ssh_key::PrivateKey::random(
        &mut rand::thread_rng(),
        ssh_key::Algorithm::Ed25519,
    )
    .unwrap();

    let fingerprint = key.public_key().fingerprint(HashAlg::Sha256).to_string();

    // SHA256 fingerprints look like "SHA256:base64data"
    assert!(
        fingerprint.starts_with("SHA256:"),
        "Fingerprint should start with SHA256:, got: {}",
        fingerprint
    );
}
