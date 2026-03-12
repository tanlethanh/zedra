/// General deeplink module for handling `zedra://` URLs from system intents.
///
/// Both QR scanner results and system URL intents (tapped links, NFC tags, etc.)
/// are routed through this module via `parse()` + `enqueue()`.
///
/// URL scheme: `zedra://<action>?key=value&...`
///
/// Supported actions:
///   - `connect?ticket=<ticket>` — pair/connect with a host via pairing ticket
use anyhow::{Result, anyhow};

use crate::pending::PendingSlot;

#[derive(Debug)]
pub enum DeeplinkAction {
    /// Connect to a host via a pairing ticket.
    Connect(zedra_rpc::ZedraPairingTicket),
}

static PENDING_DEEPLINK: PendingSlot<DeeplinkAction> = PendingSlot::new();

/// Parse a `zedra://` URL into a typed action.
pub fn parse(url: &str) -> Result<DeeplinkAction> {
    let body = url
        .strip_prefix("zedra://")
        .ok_or_else(|| anyhow!("not a zedra:// URL"))?;

    // Split path and query string
    let (path, query) = match body.find('?') {
        Some(i) => (&body[..i], Some(&body[i + 1..])),
        None => (body, None),
    };

    match path {
        "connect" => {
            let ticket_str = parse_query_param(query, "ticket")
                .ok_or_else(|| anyhow!("connect: missing ticket parameter"))?;
            let ticket = zedra_rpc::ZedraPairingTicket::decode(&ticket_str)?;
            Ok(DeeplinkAction::Connect(ticket))
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
    fn parse_connect_with_ticket() {
        let key = iroh::SecretKey::from([42u8; 32]);
        let ticket = zedra_rpc::ZedraPairingTicket {
            endpoint_id: key.public(),
            handshake_secret: [7u8; 16],
            session_id: "a1b2c3d4".to_string(),
        };
        let url = ticket.to_pairing_url().unwrap();
        assert!(url.starts_with("zedra://connect?ticket="));
        let action = parse(&url).unwrap();
        match action {
            DeeplinkAction::Connect(t) => assert_eq!(t, ticket),
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

    #[test]
    fn parse_connect_missing_ticket_fails() {
        assert!(parse("zedra://connect").is_err());
        assert!(parse("zedra://connect?other=value").is_err());
    }
}
