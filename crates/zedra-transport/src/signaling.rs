// Coordination Server Signaling
//
// Queries the coordination server for host addresses and exchanges
// connection candidates during reconnection. Enables reconnection
// after IP changes without re-scanning QR.

use anyhow::Result;
use std::time::Duration;

use zedra_relay::coord::{
    CoordClient, ConnectionCandidate, HostLookupResponse, SignalCandidates,
};

/// Timeout for coordination server queries.
const COORD_QUERY_TIMEOUT: Duration = Duration::from_secs(5);

/// Result of a host lookup via the coordination server.
#[derive(Debug, Clone)]
pub struct HostDiscovery {
    /// Whether the host is currently online.
    pub online: bool,
    /// LAN addresses reported by the host.
    pub lan_addresses: Vec<String>,
    /// Tailscale addresses (identified by 100.x.x.x prefix).
    pub tailscale_addresses: Vec<String>,
    /// Relay endpoint URL.
    pub relay_endpoint: String,
    /// All sessions available on the host.
    pub sessions: Vec<SessionInfo>,
}

/// Session info from the coordination server.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub id: String,
    pub name: String,
    pub workdir: String,
}

/// Look up a host's current addresses via the coordination server.
///
/// Returns the host's latest registered addresses, which may differ from
/// the addresses in the original QR code (e.g., after IP change).
pub async fn lookup_host(coord_url: &str, device_id: &str) -> Result<HostDiscovery> {
    let client = CoordClient::new(coord_url);

    let resp: HostLookupResponse =
        tokio::time::timeout(COORD_QUERY_TIMEOUT, client.lookup(device_id))
            .await
            .map_err(|_| anyhow::anyhow!("coord lookup timeout"))??;

    let mut lan_addrs = Vec::new();
    let mut ts_addrs = Vec::new();

    for addr in &resp.addresses {
        match addr.addr_type.as_str() {
            "lan" => lan_addrs.push(addr.addr.clone()),
            "tailscale" => ts_addrs.push(addr.addr.clone()),
            _ => lan_addrs.push(addr.addr.clone()),
        }
    }

    let sessions = resp
        .sessions
        .iter()
        .map(|s| SessionInfo {
            id: s.id.clone(),
            name: s.name.clone(),
            workdir: s.workdir.clone(),
        })
        .collect();

    Ok(HostDiscovery {
        online: resp.online,
        lan_addresses: lan_addrs,
        tailscale_addresses: ts_addrs,
        relay_endpoint: resp.relay_endpoint,
        sessions,
    })
}

/// Send our connection candidates to the host via the coordination server.
///
/// Used during reconnection: the client tells the host what addresses
/// it can be reached at, so the host can initiate a reverse connection.
pub async fn send_candidates(
    coord_url: &str,
    target_device_id: &str,
    from_device_id: &str,
    session_id: Option<&str>,
    candidates: Vec<CandidateInfo>,
) -> Result<()> {
    let client = CoordClient::new(coord_url);

    let signal = SignalCandidates {
        from_device_id: from_device_id.to_string(),
        candidates: candidates
            .into_iter()
            .map(|c| ConnectionCandidate {
                candidate_type: c.candidate_type,
                addr: c.addr,
                url: c.url,
                priority: c.priority,
            })
            .collect(),
        session_id: session_id.map(|s| s.to_string()),
    };

    tokio::time::timeout(COORD_QUERY_TIMEOUT, client.signal(target_device_id, &signal))
        .await
        .map_err(|_| anyhow::anyhow!("coord signal timeout"))??;

    Ok(())
}

/// Drain pending connection candidates from the coordination server.
///
/// Used by the host to check if any client is trying to reach it.
pub async fn drain_candidates(
    coord_url: &str,
    device_id: &str,
) -> Result<Vec<IncomingSignal>> {
    let client = CoordClient::new(coord_url);

    let signals =
        tokio::time::timeout(COORD_QUERY_TIMEOUT, client.drain_signals(device_id))
            .await
            .map_err(|_| anyhow::anyhow!("coord drain timeout"))??;

    Ok(signals
        .into_iter()
        .map(|s| IncomingSignal {
            from_device_id: s.from_device_id,
            session_id: s.session_id,
            candidates: s
                .candidates
                .into_iter()
                .map(|c| CandidateInfo {
                    candidate_type: c.candidate_type,
                    addr: c.addr,
                    url: c.url,
                    priority: c.priority,
                })
                .collect(),
        })
        .collect())
}

/// A connection candidate we can offer or receive.
#[derive(Debug, Clone)]
pub struct CandidateInfo {
    /// Type: "direct-lan", "direct-tailscale", "relay-ws", "relay-http"
    pub candidate_type: String,
    /// Address (for direct candidates), e.g. "192.168.1.100:2123"
    pub addr: Option<String>,
    /// URL (for relay candidates), e.g. "wss://relay.zedra.dev/r/abc"
    pub url: Option<String>,
    /// Priority (lower = preferred)
    pub priority: u32,
}

/// A signal received from another device via the coordination server.
#[derive(Debug, Clone)]
pub struct IncomingSignal {
    pub from_device_id: String,
    pub session_id: Option<String>,
    pub candidates: Vec<CandidateInfo>,
}

/// Extract LAN addresses from a HostDiscovery result.
///
/// Strips port from "ip:port" format to return just IPs,
/// since the port is managed separately by TransportManager.
pub fn extract_ips(addresses: &[String]) -> Vec<String> {
    addresses
        .iter()
        .map(|addr| {
            // Strip port if present (e.g., "192.168.1.100:2123" -> "192.168.1.100")
            if let Some(colon_pos) = addr.rfind(':') {
                // Check if what's after the colon is a port number
                if addr[colon_pos + 1..].parse::<u16>().is_ok() {
                    return addr[..colon_pos].to_string();
                }
            }
            addr.clone()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_ips_strips_ports() {
        let addrs = vec![
            "192.168.1.100:2123".to_string(),
            "10.0.0.5:2123".to_string(),
        ];
        let ips = extract_ips(&addrs);
        assert_eq!(ips, vec!["192.168.1.100", "10.0.0.5"]);
    }

    #[test]
    fn extract_ips_no_port() {
        let addrs = vec!["192.168.1.100".to_string()];
        let ips = extract_ips(&addrs);
        assert_eq!(ips, vec!["192.168.1.100"]);
    }

    #[test]
    fn extract_ips_empty() {
        let addrs: Vec<String> = vec![];
        let ips = extract_ips(&addrs);
        assert!(ips.is_empty());
    }

    #[test]
    fn candidate_info_construction() {
        let c = CandidateInfo {
            candidate_type: "direct-lan".to_string(),
            addr: Some("192.168.1.100:2123".to_string()),
            url: None,
            priority: 0,
        };
        assert_eq!(c.candidate_type, "direct-lan");
        assert_eq!(c.priority, 0);
        assert!(c.addr.is_some());
        assert!(c.url.is_none());
    }

    #[test]
    fn incoming_signal_construction() {
        let signal = IncomingSignal {
            from_device_id: "CLIENT-123".to_string(),
            session_id: Some("session-1".to_string()),
            candidates: vec![CandidateInfo {
                candidate_type: "relay-ws".to_string(),
                addr: None,
                url: Some("wss://relay.example.com/r/abc".to_string()),
                priority: 3,
            }],
        };
        assert_eq!(signal.from_device_id, "CLIENT-123");
        assert_eq!(signal.candidates.len(), 1);
        assert_eq!(signal.candidates[0].priority, 3);
    }
}
