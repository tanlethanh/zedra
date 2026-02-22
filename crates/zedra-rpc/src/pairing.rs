// QR code pairing: encode/decode iroh EndpointAddr for QR transfer.
//
// Format: postcard binary → base64-url string. No JSON, no custom wrapper.
// Hostname and other metadata are discovered post-connection via GetSessionInfo.

use anyhow::Result;

/// Encode an iroh EndpointAddr to a compact string for QR codes.
pub fn encode_endpoint_addr(addr: &iroh::EndpointAddr) -> Result<String> {
    let bytes = postcard::to_allocvec(addr)?;
    Ok(base64_url::encode(&bytes))
}

/// Decode an iroh EndpointAddr from a QR code string.
pub fn decode_endpoint_addr(s: &str) -> Result<iroh::EndpointAddr> {
    let bytes =
        base64_url::decode(s).map_err(|e| anyhow::anyhow!("invalid base64-url: {}", e))?;
    let addr: iroh::EndpointAddr = postcard::from_bytes(&bytes)?;
    Ok(addr)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> iroh::SecretKey {
        iroh::SecretKey::from([42u8; 32])
    }

    #[test]
    fn roundtrip() {
        let addr = iroh::EndpointAddr::from(test_key().public());

        let encoded = encode_endpoint_addr(&addr).unwrap();
        let decoded = decode_endpoint_addr(&encoded).unwrap();
        assert_eq!(addr, decoded);
    }

    #[test]
    fn roundtrip_with_relay_and_addrs() {
        let addr = iroh::EndpointAddr::from(test_key().public())
            .with_relay_url("https://relay.example.com".parse().unwrap())
            .with_ip_addr("192.168.1.100:12345".parse().unwrap());

        let encoded = encode_endpoint_addr(&addr).unwrap();
        let decoded = decode_endpoint_addr(&encoded).unwrap();
        assert_eq!(addr, decoded);
    }

    #[test]
    fn decode_invalid_fails() {
        assert!(decode_endpoint_addr("not-valid!!!").is_err());
    }
}
