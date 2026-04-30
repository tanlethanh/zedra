// net_monitor: Background network diagnostics for zedra-host.
//
// Continuously watches the iroh endpoint for network condition changes
// and logs them. Only high-level state changes (network changed, network
// binding OK) are printed to stderr; all details go to tracing.

use std::collections::BTreeSet;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[allow(unused)]
fn ts() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let s = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!(
        "{:02}:{:02}:{:02}",
        (s % 86400) / 3600,
        (s % 3600) / 60,
        s % 60
    )
}

/// Snapshot of net_report state for change detection.
#[derive(Debug, Clone)]
struct ReportSnapshot {
    global_v4: Option<Ipv4Addr>,
    global_v6: Option<Ipv6Addr>,
    symmetric_nat: Option<bool>,
    has_udp: bool,
    captive_portal: Option<bool>,
}

impl ReportSnapshot {
    fn empty() -> Self {
        Self {
            global_v4: None,
            global_v6: None,
            symmetric_nat: None,
            has_udp: false,
            captive_portal: None,
        }
    }
}

/// Classify likely network type from the public IP address heuristics.
#[allow(unused)]
fn classify_network(addrs: &BTreeSet<SocketAddr>, relay_only: bool) -> &'static str {
    if relay_only {
        return "relay-only";
    }
    if addrs.is_empty() {
        return "unknown";
    }
    for addr in addrs {
        match addr.ip() {
            std::net::IpAddr::V4(v4) => {
                let octets = v4.octets();
                if octets[0] == 100 && (octets[1] & 0xC0) == 64 {
                    return "tailscale/CGNAT";
                }
                if octets[0] == 10 {
                    return "vpn/private";
                }
                if octets[0] == 172 && (16..=31).contains(&octets[1]) {
                    return "vpn/private";
                }
                if octets[0] == 192 && octets[1] == 168 {
                    return "LAN";
                }
            }
            std::net::IpAddr::V6(_) => {}
        }
    }
    "direct"
}

/// Spawn background tasks that watch for network changes and log them.
pub fn spawn_net_monitor(endpoint: &iroh::Endpoint) {
    let recovery_in_flight = Arc::new(AtomicBool::new(false));
    spawn_addr_watcher(endpoint, recovery_in_flight.clone());
    spawn_report_watcher(endpoint, recovery_in_flight);
}

fn spawn_addr_watcher(endpoint: &iroh::Endpoint, recovery_in_flight: Arc<AtomicBool>) {
    use iroh::Watcher;

    let mut watcher = endpoint.watch_addr();
    let endpoint_clone = endpoint.clone();

    tokio::spawn(async move {
        let initial = watcher.get();
        let mut prev_relays: BTreeSet<String> =
            initial.relay_urls().map(|u| u.to_string()).collect();
        let mut prev_addrs: BTreeSet<SocketAddr> = initial.ip_addrs().copied().collect();

        tracing::info!(
            "net_monitor: addr watcher started, relays={:?} addrs={:?}",
            prev_relays,
            prev_addrs
        );

        loop {
            if watcher.updated().await.is_err() {
                tracing::info!("net_monitor: addr watcher closed");
                break;
            }
            let addr = watcher.get();
            let new_relays: BTreeSet<String> = addr.relay_urls().map(|u| u.to_string()).collect();
            let new_addrs: BTreeSet<SocketAddr> = addr.ip_addrs().copied().collect();

            let relays_changed = new_relays != prev_relays;
            let addrs_changed = new_addrs != prev_addrs;

            if !relays_changed && !addrs_changed {
                continue;
            }

            trigger_endpoint_recovery(
                endpoint_clone.clone(),
                recovery_in_flight.clone(),
                "addr_changed",
            );

            // Log details to tracing
            if relays_changed {
                let added: Vec<_> = new_relays.difference(&prev_relays).collect();
                let removed: Vec<_> = prev_relays.difference(&new_relays).collect();
                tracing::info!(
                    "net_monitor: relay changed, added={:?} removed={:?}",
                    added,
                    removed
                );
                if new_relays.is_empty() {
                    tracing::warn!("net_monitor: no relay connection");
                }
            }
            if addrs_changed {
                let added: Vec<_> = new_addrs.difference(&prev_addrs).collect();
                let removed: Vec<_> = prev_addrs.difference(&new_addrs).collect();
                tracing::info!(
                    "net_monitor: addrs changed, added={:?} removed={:?}",
                    added,
                    removed
                );
            }

            // DNS/pkarr re-registration check
            tracing::info!(
                "net_monitor: pkarr re-registering, relays={:?} addrs={:?}",
                new_relays,
                new_addrs,
            );
            let ep = endpoint_clone.clone();
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                let current = ep.addr();
                let relay_count = current.relay_urls().count();
                let addr_count = current.ip_addrs().count();
                if relay_count == 0 && addr_count == 0 {
                    tracing::warn!("net_monitor: endpoint has no addresses 5s after change");
                } else {
                    tracing::info!(
                        "net_monitor: re-registration OK, relays={} addrs={}",
                        relay_count,
                        addr_count
                    );
                }
            });

            prev_relays = new_relays;
            prev_addrs = new_addrs;
        }
    });
}

fn spawn_report_watcher(endpoint: &iroh::Endpoint, recovery_in_flight: Arc<AtomicBool>) {
    use iroh::Watcher;

    let mut watcher = endpoint.net_report();
    let endpoint_clone = endpoint.clone();

    tokio::spawn(async move {
        let mut prev = ReportSnapshot::empty();
        // Capture initial state — first report is already logged by create_endpoint
        let initial = watcher.get();
        if let Some(ref r) = initial {
            prev.global_v4 = r.global_v4.map(|a| *a.ip());
            prev.global_v6 = r.global_v6.map(|a| *a.ip());
            prev.symmetric_nat = r.mapping_varies_by_dest();
            prev.has_udp = r.has_udp();
            prev.captive_portal = r.captive_portal;
        }

        loop {
            if watcher.updated().await.is_err() {
                tracing::info!("net_monitor: report watcher closed");
                break;
            }
            let report = match watcher.get() {
                Some(r) => r,
                None => continue,
            };

            let new_v4 = report.global_v4.map(|a| *a.ip());
            let new_v6 = report.global_v6.map(|a| *a.ip());
            let new_sym = report.mapping_varies_by_dest();
            let new_udp = report.has_udp();
            let new_captive = report.captive_portal;

            let anything_changed = new_v4 != prev.global_v4
                || new_v6 != prev.global_v6
                || new_sym != prev.symmetric_nat
                || new_udp != prev.has_udp
                || new_captive != prev.captive_portal;

            if !anything_changed {
                continue;
            }

            // All details to tracing
            if new_v4 != prev.global_v4 {
                tracing::info!("net_monitor: IPv4 {:?} → {:?}", prev.global_v4, new_v4);
            }
            if new_v6 != prev.global_v6 {
                tracing::info!("net_monitor: IPv6 {:?} → {:?}", prev.global_v6, new_v6);
            }
            if new_sym != prev.symmetric_nat {
                tracing::info!(
                    "net_monitor: symmetric_nat {:?} → {:?}",
                    prev.symmetric_nat,
                    new_sym
                );
            }
            if new_udp != prev.has_udp {
                tracing::info!("net_monitor: udp {} → {}", prev.has_udp, new_udp);
            }
            if new_captive != prev.captive_portal {
                tracing::info!(
                    "net_monitor: captive_portal {:?} → {:?}",
                    prev.captive_portal,
                    new_captive
                );
            }

            // User-facing: only print if the IP actually changed (= real network switch)
            let ip_changed = new_v4 != prev.global_v4 || new_v6 != prev.global_v6;
            if ip_changed {
                trigger_endpoint_recovery(
                    endpoint_clone.clone(),
                    recovery_in_flight.clone(),
                    "public_ip_changed",
                );

                let addr_str = new_v4
                    .map(|a| a.to_string())
                    .or_else(|| new_v6.map(|a| a.to_string()))
                    .unwrap_or_else(|| "none".to_string());
                let nat_hint = match new_sym {
                    Some(true) => " (symmetric NAT)",
                    _ => "",
                };
                tracing::info!("net_monitor: public IP changed to {addr_str}{nat_hint}",);
            }

            prev.global_v4 = new_v4;
            prev.global_v6 = new_v6;
            prev.symmetric_nat = new_sym;
            prev.has_udp = new_udp;
            prev.captive_portal = new_captive;
        }
    });
}

fn trigger_endpoint_recovery(
    endpoint: iroh::Endpoint,
    recovery_in_flight: Arc<AtomicBool>,
    reason: &'static str,
) {
    if recovery_in_flight
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        tracing::debug!("net_monitor: recovery already running, skip {}", reason);
        return;
    }

    tokio::spawn(async move {
        tracing::info!("net_monitor: starting endpoint recovery ({})", reason);
        endpoint.network_change().await;
        endpoint.dns_resolver().reset().await;
        match tokio::time::timeout(std::time::Duration::from_secs(8), endpoint.online()).await {
            Ok(()) => {
                let addr = endpoint.addr();
                tracing::info!(
                    "net_monitor: endpoint recovery complete ({}), relays={} addrs={}",
                    reason,
                    addr.relay_urls().count(),
                    addr.ip_addrs().count()
                );
            }
            Err(_) => {
                tracing::warn!("net_monitor: endpoint recovery timed out ({})", reason);
            }
        }

        recovery_in_flight.store(false, Ordering::Release);
    });
}
