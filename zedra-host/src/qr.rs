// QR code generation for device pairing
// Generates a QR code containing connection info + one-time pairing token

use anyhow::Result;
use qrcode::QrCode;
use serde::Serialize;

use crate::auth;
use crate::store;

/// Pairing payload encoded in the QR code
#[derive(Serialize)]
struct PairingPayload {
    /// Protocol version
    v: u32,
    /// Host address (IP or hostname)
    host: String,
    /// SSH port
    port: u16,
    /// One-time pairing token
    token: String,
    /// Host key fingerprint for verification
    fingerprint: String,
    /// Friendly name for this machine
    name: String,
}

/// Generate and display a pairing QR code
pub async fn generate_pairing_qr() -> Result<()> {
    // Get host info
    let hostname = gethostname();
    let ip = get_local_ip().unwrap_or_else(|| "localhost".to_string());
    let port = 2222u16;

    // Get host key fingerprint
    let fingerprint = get_host_fingerprint()?;

    // Generate pairing token
    let token = auth::create_pairing_token();

    // Build payload
    let payload = PairingPayload {
        v: 1,
        host: ip.clone(),
        port,
        token,
        fingerprint,
        name: hostname.clone(),
    };

    let json = serde_json::to_string(&payload)?;
    let encoded = base64_url::encode(&json);
    let uri = format!("zedra://pair?d={}", encoded);

    // Generate QR code
    let code = QrCode::new(uri.as_bytes())?;

    // Render to terminal using Unicode block characters
    let string = render_qr_to_terminal(&code);

    println!();
    println!("  Zedra Host Pairing");
    println!("  ==================");
    println!();
    println!("  Scan this QR code with the Zedra app to pair this device.");
    println!("  Host: {} ({})", hostname, ip);
    println!("  Port: {}", port);
    println!("  Token expires in 5 minutes.");
    println!();
    println!("{}", string);
    println!();
    println!("  Or connect manually:");
    println!("    Host: {}", ip);
    println!("    Port: {}", port);
    println!("    Username: zedra");
    println!();

    Ok(())
}

/// Render QR code to terminal using Unicode block characters
fn render_qr_to_terminal(code: &QrCode) -> String {
    let width = code.width();
    let mut result = String::new();

    // Process two rows at a time using Unicode half blocks
    let mut row = 0;
    while row < width {
        result.push_str("    "); // Left margin
        for col in 0..width {
            let top = code[(col, row)] == qrcode::types::Color::Dark;
            let bottom = if row + 1 < width {
                code[(col, row + 1)] == qrcode::types::Color::Dark
            } else {
                false
            };

            match (top, bottom) {
                (true, true) => result.push('\u{2588}'),   // Full block
                (true, false) => result.push('\u{2580}'),  // Upper half
                (false, true) => result.push('\u{2584}'),  // Lower half
                (false, false) => result.push(' '),         // Empty
            }
        }
        result.push('\n');
        row += 2;
    }

    result
}

/// Get the host key fingerprint
fn get_host_fingerprint() -> Result<String> {
    let key_path = store::host_key_path()?;
    if !key_path.exists() {
        // Generate host key on first run
        generate_host_key(&key_path)?;
    }

    let key_data = std::fs::read(&key_path)?;
    let key = ssh_key::PrivateKey::from_openssh(&key_data)
        .map_err(|e| anyhow::anyhow!("Failed to read host key: {}", e))?;
    let public_key = key.public_key();
    let fingerprint = public_key.fingerprint(ssh_key::HashAlg::Sha256);
    Ok(fingerprint.to_string())
}

/// Generate an Ed25519 host key
fn generate_host_key(path: &std::path::Path) -> Result<()> {
    tracing::info!("Generating host key at {:?}", path);

    let key = ssh_key::PrivateKey::random(&mut rand::thread_rng(), ssh_key::Algorithm::Ed25519)
        .map_err(|e| anyhow::anyhow!("Failed to generate key: {}", e))?;

    let openssh = key
        .to_openssh(ssh_key::LineEnding::LF)
        .map_err(|e| anyhow::anyhow!("Failed to serialize key: {}", e))?;

    std::fs::write(path, openssh.as_bytes())?;

    // Set permissions to 600
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }

    Ok(())
}

/// Get local hostname
fn gethostname() -> String {
    hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Get local IP address
fn get_local_ip() -> Option<String> {
    // Try to find a non-loopback IPv4 address
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    let addr = socket.local_addr().ok()?;
    Some(addr.ip().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_qr_to_terminal() {
        let code = QrCode::new(b"test").unwrap();
        let rendered = render_qr_to_terminal(&code);
        assert!(!rendered.is_empty());
        assert!(rendered.contains('\n'));
        // Should have left margin
        for line in rendered.lines() {
            assert!(line.starts_with("    "));
        }
    }

    #[test]
    fn test_pairing_payload_serialization() {
        let payload = PairingPayload {
            v: 1,
            host: "192.168.1.1".to_string(),
            port: 2222,
            token: "abc123".to_string(),
            fingerprint: "SHA256:xxxx".to_string(),
            name: "my-machine".to_string(),
        };

        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("192.168.1.1"));
        assert!(json.contains("2222"));
        assert!(json.contains("abc123"));
        assert!(json.contains("SHA256:xxxx"));
        assert!(json.contains("my-machine"));

        // Verify it can be encoded/decoded as base64url
        let encoded = base64_url::encode(&json);
        let decoded = base64_url::decode(&encoded).unwrap();
        let decoded_str = String::from_utf8(decoded).unwrap();
        assert_eq!(decoded_str, json);
    }

    #[test]
    fn test_pairing_uri_format() {
        let payload = PairingPayload {
            v: 1,
            host: "10.0.0.1".to_string(),
            port: 2222,
            token: "token".to_string(),
            fingerprint: "fp".to_string(),
            name: "host".to_string(),
        };

        let json = serde_json::to_string(&payload).unwrap();
        let encoded = base64_url::encode(&json);
        let uri = format!("zedra://pair?d={}", encoded);

        assert!(uri.starts_with("zedra://pair?d="));
        // The base64url-encoded data portion should not contain + or /
        let data_part = uri.strip_prefix("zedra://pair?d=").unwrap();
        assert!(!data_part.contains('+'));
        assert!(!data_part.contains('/'));
    }

    #[test]
    fn test_qr_code_from_uri() {
        let uri = "zedra://pair?d=eyJ2IjoxfQ";
        let code = QrCode::new(uri.as_bytes());
        assert!(code.is_ok());
    }

    #[test]
    fn test_gethostname_returns_nonempty() {
        let name = gethostname();
        assert!(!name.is_empty());
    }
}
