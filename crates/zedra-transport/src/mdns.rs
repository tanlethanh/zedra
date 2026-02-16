// mDNS/UDP Local Discovery
//
// Broadcast-based local network discovery inspired by Syncthing's
// Local Discovery Protocol v4. Host broadcasts presence on the local
// network; client listens for announcements from trusted hosts.
//
// Protocol:
// - UDP broadcast to 255.255.255.255:21027 (IPv4)
// - Packet: 5-byte magic "ZEDRA" + msgpack/JSON payload
// - Broadcast interval: 30 seconds
// - Payload contains device_id, addresses, sessions

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

/// UDP port for local discovery broadcasts.
pub const DISCOVERY_PORT: u16 = 21027;

/// Magic bytes prefix for discovery packets.
const MAGIC: &[u8; 5] = b"ZEDRA";

/// How often the host broadcasts its presence.
const ANNOUNCE_INTERVAL: Duration = Duration::from_secs(30);

/// How long a discovered host entry stays valid without a new announcement.
const ENTRY_TTL: Duration = Duration::from_secs(90);

/// Discovery announcement payload.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Announcement {
    /// Protocol version.
    pub v: u32,
    /// Device ID (Syncthing-style chunked hash of public key).
    pub device_id: String,
    /// Addresses this host is reachable at (ip:port format).
    pub addresses: Vec<String>,
    /// Session IDs currently hosted.
    pub sessions: Vec<String>,
}

/// Encode an announcement into a UDP packet (MAGIC + JSON).
pub fn encode_announcement(ann: &Announcement) -> Vec<u8> {
    let json = serde_json::to_vec(ann).unwrap_or_default();
    let mut packet = Vec::with_capacity(MAGIC.len() + json.len());
    packet.extend_from_slice(MAGIC);
    packet.extend_from_slice(&json);
    packet
}

/// Decode a UDP packet into an announcement. Returns None if invalid.
pub fn decode_announcement(data: &[u8]) -> Option<Announcement> {
    if data.len() < MAGIC.len() {
        return None;
    }
    if &data[..MAGIC.len()] != MAGIC {
        return None;
    }
    serde_json::from_slice(&data[MAGIC.len()..]).ok()
}

/// A discovered host on the local network.
#[derive(Debug, Clone)]
pub struct DiscoveredHost {
    /// Device ID of the discovered host.
    pub device_id: String,
    /// Addresses the host announced.
    pub addresses: Vec<String>,
    /// Session IDs on this host.
    pub sessions: Vec<String>,
    /// When this entry was last refreshed.
    pub last_seen: Instant,
    /// Source address of the UDP packet.
    pub source_addr: SocketAddr,
}

/// Cache of discovered hosts, keyed by device_id.
#[derive(Debug, Default)]
pub struct DiscoveryCache {
    hosts: HashMap<String, DiscoveredHost>,
}

impl DiscoveryCache {
    pub fn new() -> Self {
        Self {
            hosts: HashMap::new(),
        }
    }

    /// Update the cache with a new announcement from the given source.
    pub fn update(&mut self, ann: &Announcement, source: SocketAddr) {
        self.hosts.insert(
            ann.device_id.clone(),
            DiscoveredHost {
                device_id: ann.device_id.clone(),
                addresses: ann.addresses.clone(),
                sessions: ann.sessions.clone(),
                last_seen: Instant::now(),
                source_addr: source,
            },
        );
    }

    /// Look up a host by device_id. Returns None if not found or expired.
    pub fn lookup(&self, device_id: &str) -> Option<&DiscoveredHost> {
        self.hosts.get(device_id).filter(|h| h.last_seen.elapsed() < ENTRY_TTL)
    }

    /// Get all addresses for a device_id. Returns empty vec if not found.
    pub fn addresses_for(&self, device_id: &str) -> Vec<String> {
        self.lookup(device_id)
            .map(|h| h.addresses.clone())
            .unwrap_or_default()
    }

    /// Remove expired entries.
    pub fn evict_expired(&mut self) {
        self.hosts.retain(|_, h| h.last_seen.elapsed() < ENTRY_TTL);
    }
}

/// Run the host announcer loop. Broadcasts presence every 30 seconds.
///
/// This should be spawned as a background task on the host daemon.
/// Non-fatal: if broadcasting fails, it logs and retries next interval.
pub async fn run_announcer(announcement: Announcement) {
    let socket = match tokio::net::UdpSocket::bind("0.0.0.0:0").await {
        Ok(s) => s,
        Err(e) => {
            log::warn!("mdns: failed to bind announcer socket: {}", e);
            return;
        }
    };

    if let Err(e) = socket.set_broadcast(true) {
        log::warn!("mdns: failed to enable broadcast: {}", e);
        return;
    }

    let packet = encode_announcement(&announcement);
    let broadcast_addr: SocketAddr = ([255, 255, 255, 255], DISCOVERY_PORT).into();

    log::info!(
        "mdns: starting announcer (device_id: {}, {} addresses)",
        announcement.device_id,
        announcement.addresses.len()
    );

    let mut interval = tokio::time::interval(ANNOUNCE_INTERVAL);

    loop {
        interval.tick().await;

        match socket.send_to(&packet, broadcast_addr).await {
            Ok(n) => {
                log::debug!("mdns: broadcast {} bytes", n);
            }
            Err(e) => {
                log::debug!("mdns: broadcast failed: {} (will retry)", e);
            }
        }
    }
}

/// Run the discovery listener. Listens for announcements and updates the cache.
///
/// This should be spawned as a background task on the client.
/// The cache is shared via Arc<Mutex<DiscoveryCache>>.
pub async fn run_listener(cache: std::sync::Arc<std::sync::Mutex<DiscoveryCache>>) {
    let socket = match tokio::net::UdpSocket::bind(("0.0.0.0", DISCOVERY_PORT)).await {
        Ok(s) => s,
        Err(e) => {
            log::warn!("mdns: failed to bind listener on port {}: {}", DISCOVERY_PORT, e);
            return;
        }
    };

    log::info!("mdns: listening for announcements on port {}", DISCOVERY_PORT);

    let mut buf = vec![0u8; 4096];

    loop {
        match socket.recv_from(&mut buf).await {
            Ok((n, src)) => {
                if let Some(ann) = decode_announcement(&buf[..n]) {
                    log::debug!(
                        "mdns: announcement from {} (device: {}, addrs: {:?})",
                        src,
                        ann.device_id,
                        ann.addresses
                    );
                    if let Ok(mut c) = cache.lock() {
                        c.update(&ann, src);
                    }
                }
            }
            Err(e) => {
                log::debug!("mdns: recv error: {} (continuing)", e);
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

/// One-shot discovery: listen for announcements for a given device_id
/// with a timeout. Returns discovered addresses or empty vec.
pub async fn discover_host(device_id: &str, timeout_duration: Duration) -> Vec<String> {
    // Try to bind to the discovery port to hear existing announcements.
    // Falls back to an ephemeral port if the discovery port is in use.
    let socket = match tokio::net::UdpSocket::bind(("0.0.0.0", DISCOVERY_PORT)).await {
        Ok(s) => s,
        Err(_) => match tokio::net::UdpSocket::bind(("0.0.0.0", 0)).await {
            Ok(s) => s,
            Err(_) => return vec![],
        },
    };

    let mut buf = vec![0u8; 4096];
    let deadline = tokio::time::Instant::now() + timeout_duration;

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return vec![];
        }

        match tokio::time::timeout(remaining, socket.recv_from(&mut buf)).await {
            Ok(Ok((n, _src))) => {
                if let Some(ann) = decode_announcement(&buf[..n]) {
                    if ann.device_id == device_id {
                        return ann.addresses;
                    }
                }
            }
            Ok(Err(_)) | Err(_) => {
                return vec![];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_roundtrip() {
        let ann = Announcement {
            v: 1,
            device_id: "TEST-DEVICE-ID".to_string(),
            addresses: vec!["192.168.1.100:2123".to_string()],
            sessions: vec!["session-1".to_string()],
        };

        let packet = encode_announcement(&ann);
        assert!(packet.starts_with(b"ZEDRA"));

        let decoded = decode_announcement(&packet).unwrap();
        assert_eq!(decoded.device_id, "TEST-DEVICE-ID");
        assert_eq!(decoded.addresses, vec!["192.168.1.100:2123"]);
        assert_eq!(decoded.sessions, vec!["session-1"]);
        assert_eq!(decoded.v, 1);
    }

    #[test]
    fn decode_invalid_magic() {
        let data = b"WRONG{\"v\":1}";
        assert!(decode_announcement(data).is_none());
    }

    #[test]
    fn decode_too_short() {
        assert!(decode_announcement(b"ZED").is_none());
    }

    #[test]
    fn decode_invalid_json() {
        let mut packet = Vec::new();
        packet.extend_from_slice(MAGIC);
        packet.extend_from_slice(b"not json");
        assert!(decode_announcement(&packet).is_none());
    }

    #[test]
    fn cache_update_and_lookup() {
        let mut cache = DiscoveryCache::new();
        let ann = Announcement {
            v: 1,
            device_id: "HOST-A".to_string(),
            addresses: vec!["10.0.0.1:2123".to_string()],
            sessions: vec![],
        };

        let src: SocketAddr = "192.168.1.5:12345".parse().unwrap();
        cache.update(&ann, src);

        let host = cache.lookup("HOST-A").unwrap();
        assert_eq!(host.addresses, vec!["10.0.0.1:2123"]);
        assert_eq!(host.source_addr, src);

        assert!(cache.lookup("HOST-B").is_none());
    }

    #[test]
    fn cache_addresses_for() {
        let mut cache = DiscoveryCache::new();
        let ann = Announcement {
            v: 1,
            device_id: "HOST-C".to_string(),
            addresses: vec!["10.0.0.2:2123".to_string(), "10.0.0.3:2123".to_string()],
            sessions: vec![],
        };

        let src: SocketAddr = "192.168.1.10:5555".parse().unwrap();
        cache.update(&ann, src);

        let addrs = cache.addresses_for("HOST-C");
        assert_eq!(addrs.len(), 2);

        let missing = cache.addresses_for("UNKNOWN");
        assert!(missing.is_empty());
    }

    #[test]
    fn cache_update_replaces_stale() {
        let mut cache = DiscoveryCache::new();
        let src: SocketAddr = "192.168.1.1:1234".parse().unwrap();

        // First announcement
        let ann1 = Announcement {
            v: 1,
            device_id: "HOST-D".to_string(),
            addresses: vec!["10.0.0.1:2123".to_string()],
            sessions: vec![],
        };
        cache.update(&ann1, src);

        // Second announcement with updated address
        let ann2 = Announcement {
            v: 1,
            device_id: "HOST-D".to_string(),
            addresses: vec!["10.0.0.99:2123".to_string()],
            sessions: vec![],
        };
        cache.update(&ann2, src);

        let addrs = cache.addresses_for("HOST-D");
        assert_eq!(addrs, vec!["10.0.0.99:2123"]);
    }

    #[test]
    fn cache_evict_expired() {
        let mut cache = DiscoveryCache::new();
        let src: SocketAddr = "192.168.1.1:1234".parse().unwrap();

        let ann = Announcement {
            v: 1,
            device_id: "HOST-E".to_string(),
            addresses: vec!["10.0.0.1:2123".to_string()],
            sessions: vec![],
        };
        cache.update(&ann, src);

        // Manually expire by setting last_seen to the past
        if let Some(host) = cache.hosts.get_mut("HOST-E") {
            host.last_seen = Instant::now() - Duration::from_secs(200);
        }

        cache.evict_expired();
        assert!(cache.lookup("HOST-E").is_none());
    }
}
