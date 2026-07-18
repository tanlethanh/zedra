/// General deeplink module for handling `zedra://` URLs from system intents.
///
/// Both QR scanner results and system URL intents (tapped links, NFC tags, etc.)
/// are routed through this module via `parse()` + `enqueue()`.
///
/// URL scheme: `zedra://<action>?key=value&...`
///
/// Supported actions:
///   - `connect?ticket=<ticket>` — pair/connect with a host via pairing ticket
///   - `open?endpoint_addr=<addr>&terminal_id=<id>` — navigate to a workspace, connecting
///     first if needed. `terminal_id` is optional; when present, navigate to that terminal
///     once the workspace is synced.
use anyhow::{Result, anyhow};

use crate::pending::PendingSlot;

#[derive(Debug)]
pub enum DeeplinkAction {
    /// Connect to a host via a pairing ticket.
    Connect(zedra_rpc::ZedraPairingTicket),
    /// Navigate to a workspace, optionally to a specific terminal within it.
    Open {
        endpoint_addr: String,
        terminal_id: Option<String>,
    },
    /// Debug devtool: drive the web tunnel on the active workspace directly, to
    /// reproduce adapter cases. `zedra://devtool/tunnel?url=<url>&mode=alias&collide=<port>&reset=1`.
    #[cfg(debug_assertions)]
    DebugTunnel {
        url: Option<String>,
        force_alias: bool,
        collide: Option<u16>,
        reset: bool,
    },
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
        "open" => {
            let endpoint_addr = parse_query_param(query, "endpoint_addr")
                .ok_or_else(|| anyhow!("open: missing endpoint_addr parameter"))?;
            if endpoint_addr.is_empty() {
                return Err(anyhow!("open: endpoint_addr must be non-empty"));
            }
            // terminal_id is optional: absent means navigate to the workspace only.
            let terminal_id = parse_query_param(query, "terminal_id").filter(|t| !t.is_empty());
            Ok(DeeplinkAction::Open {
                endpoint_addr,
                terminal_id,
            })
        }
        #[cfg(debug_assertions)]
        "devtool/tunnel" => Ok(DeeplinkAction::DebugTunnel {
            // url is passed unencoded (no `&`); the simple query parser keeps it intact.
            url: parse_query_param(query, "url").filter(|u| !u.is_empty()),
            force_alias: parse_query_param(query, "mode").as_deref() == Some("alias"),
            collide: parse_query_param(query, "collide").and_then(|p| p.parse().ok()),
            reset: parse_query_param(query, "reset").is_some(),
        }),
        other => Err(anyhow!("unknown deeplink action: {}", other)),
    }
}

/// Enqueue a parsed deeplink action for consumption on the next app tick.
/// The app's tick function checks for pending deeplinks periodically.
pub fn enqueue(action: DeeplinkAction) {
    PENDING_DEEPLINK.set(action);
}

/// Take the pending deeplink action (called from `ZedraApp::tick`).
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
            other => panic!("expected Connect, got {other:?}"),
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
