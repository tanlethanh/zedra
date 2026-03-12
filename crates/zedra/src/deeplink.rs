/// General deeplink module for handling `zedra://` URLs from system intents.
///
/// Both QR scanner results and system URL intents (tapped links, NFC tags, etc.)
/// are routed through this module via `parse()` + `enqueue()`.
///
/// URL scheme: `zedra://<action>[/<payload>][?key=value&...]`
///
/// Supported actions:
///   - `pair/<ticket>`   — pair with a host (same payload as QR)
///   - `connect/<addr>`  — reconnect to a known endpoint
///
/// Legacy: `zedra://zedra<base32>` is treated as a `Pair` action for backward
/// compatibility with existing QR code URLs.
use anyhow::{Result, anyhow};

use crate::pending::PendingSlot;

#[derive(Debug)]
pub enum DeeplinkAction {
    /// Pair with a new host via a pairing ticket (same as QR scan).
    Pair(zedra_rpc::ZedraPairingTicket),
    /// Reconnect to a previously known endpoint.
    Connect {
        endpoint_addr: String,
        session_id: Option<String>,
    },
}

static PENDING_DEEPLINK: PendingSlot<DeeplinkAction> = PendingSlot::new();

/// Parse a `zedra://` URL into a typed action.
pub fn parse(url: &str) -> Result<DeeplinkAction> {
    let body = url
        .strip_prefix("zedra://")
        .ok_or_else(|| anyhow!("not a zedra:// URL"))?;

    // Legacy QR format: zedra://zedra<base32 ticket>
    if body.starts_with("zedra") {
        let ticket = zedra_rpc::ZedraPairingTicket::decode(body)?;
        return Ok(DeeplinkAction::Pair(ticket));
    }

    // Split path and query string
    let (path, query) = match body.find('?') {
        Some(i) => (&body[..i], Some(&body[i + 1..])),
        None => (body, None),
    };

    let segments: Vec<&str> = path.split('/').collect();
    let action = segments.first().ok_or_else(|| anyhow!("empty deeplink path"))?;

    match *action {
        "pair" => {
            let payload = segments.get(1).ok_or_else(|| anyhow!("pair: missing ticket payload"))?;
            let ticket = zedra_rpc::ZedraPairingTicket::decode(payload)?;
            Ok(DeeplinkAction::Pair(ticket))
        }
        "connect" => {
            let addr = segments
                .get(1)
                .ok_or_else(|| anyhow!("connect: missing endpoint address"))?
                .to_string();
            let session_id = parse_query_param(query, "session");
            Ok(DeeplinkAction::Connect {
                endpoint_addr: addr,
                session_id,
            })
        }
        other => Err(anyhow!("unknown deeplink action: {}", other)),
    }
}

/// Enqueue a parsed deeplink action for consumption on the next render frame.
pub fn enqueue(action: DeeplinkAction) {
    PENDING_DEEPLINK.set(action);
    zedra_session::signal_terminal_data();
}

/// Take the pending deeplink action (called from `ZedraApp::render`).
pub fn take_pending() -> Option<DeeplinkAction> {
    PENDING_DEEPLINK.take()
}

fn parse_query_param(query: Option<&str>, key: &str) -> Option<String> {
    let query = query?;
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            if k == key {
                return Some(v.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_legacy_qr_url() {
        // Build a real ticket, encode it, and wrap in zedra:// URL
        let key = iroh::SecretKey::from([42u8; 32]);
        let ticket = zedra_rpc::ZedraPairingTicket {
            endpoint_id: key.public(),
            handshake_key: [7u8; 32],
            session_id: "test-sess".to_string(),
        };
        let url = ticket.to_qr_url().unwrap();
        let action = parse(&url).unwrap();
        assert!(matches!(action, DeeplinkAction::Pair(_)));
    }

    #[test]
    fn parse_pair_action() {
        let key = iroh::SecretKey::from([42u8; 32]);
        let ticket = zedra_rpc::ZedraPairingTicket {
            endpoint_id: key.public(),
            handshake_key: [7u8; 32],
            session_id: "test-sess".to_string(),
        };
        let encoded = ticket.encode().unwrap();
        let url = format!("zedra://pair/{}", encoded);
        let action = parse(&url).unwrap();
        assert!(matches!(action, DeeplinkAction::Pair(_)));
    }

    #[test]
    fn parse_connect_action() {
        let url = "zedra://connect/abc123def?session=my-session";
        let action = parse(url).unwrap();
        match action {
            DeeplinkAction::Connect {
                endpoint_addr,
                session_id,
            } => {
                assert_eq!(endpoint_addr, "abc123def");
                assert_eq!(session_id, Some("my-session".to_string()));
            }
            _ => panic!("expected Connect"),
        }
    }

    #[test]
    fn parse_connect_no_session() {
        let url = "zedra://connect/abc123def";
        let action = parse(url).unwrap();
        match action {
            DeeplinkAction::Connect {
                session_id, ..
            } => {
                assert_eq!(session_id, None);
            }
            _ => panic!("expected Connect"),
        }
    }

    #[test]
    fn parse_unknown_action_fails() {
        assert!(parse("zedra://unknown/foo").is_err());
    }

    #[test]
    fn parse_not_zedra_scheme_fails() {
        assert!(parse("https://example.com").is_err());
    }
}
