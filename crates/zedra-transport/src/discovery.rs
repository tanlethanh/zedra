use anyhow::Result;
use std::time::Duration;
use tokio::task::JoinSet;
use tokio::time::timeout;

use crate::providers::TransportProvider;
use zedra_rpc::Transport;

/// Result of a successful provider connection, carrying priority for selection.
struct ConnectResult {
    priority: u32,
    name: String,
    transport: Box<dyn Transport>,
}

/// Run the discovery chain: try all providers concurrently, return the
/// highest-priority (lowest number) transport that connects successfully.
///
/// Strategy:
/// 1. Spawn all providers concurrently.
/// 2. Wait up to 500ms for a high-priority (LAN/Tailscale) transport.
/// 3. If none connected in that window, wait for any provider up to 10s total.
/// 4. Among all successful connections, pick the one with lowest priority value.
pub async fn discover(
    mut providers: Vec<Box<dyn TransportProvider>>,
) -> Result<(Box<dyn Transport>, String)> {
    if providers.is_empty() {
        anyhow::bail!("no transport providers configured");
    }

    // Sort by priority so we can reason about preference order
    providers.sort_by_key(|p| p.priority());

    let mut join_set = JoinSet::new();

    for provider in providers {
        let name = provider.name().to_string();
        let priority = provider.priority();
        join_set.spawn(async move {
            let result = provider.connect().await;
            (priority, name, result)
        });
    }

    let mut best: Option<ConnectResult> = None;
    let mut errors: Vec<String> = Vec::new();

    // Phase 1: Wait up to 500ms for high-priority transports (LAN/Tailscale)
    let fast_deadline = Duration::from_millis(500);
    let fast_phase = async {
        while let Some(result) = join_set.join_next().await {
            match result {
                Ok((priority, name, Ok(transport))) => {
                    log::info!("discovery: {} connected (priority {})", name, priority);
                    let dominated = best.as_ref().is_some_and(|b| b.priority <= priority);
                    if !dominated {
                        best = Some(ConnectResult {
                            priority,
                            name,
                            transport,
                        });
                    }
                    // If we got a priority-0 (LAN) connection, no need to wait further
                    if best.as_ref().is_some_and(|b| b.priority == 0) {
                        return;
                    }
                }
                Ok((_priority, name, Err(e))) => {
                    log::debug!("discovery: {} failed: {}", name, e);
                    errors.push(format!("{}: {}", name, e));
                }
                Err(e) => {
                    errors.push(format!("task panic: {}", e));
                }
            }
        }
    };
    let _ = timeout(fast_deadline, fast_phase).await;

    // If we have a good transport from the fast phase, use it
    if let Some(ref b) = best {
        if b.priority <= 1 {
            log::info!("discovery: using {} (fast phase)", b.name);
            // Abort remaining tasks
            join_set.abort_all();
            let result = best.unwrap();
            return Ok((result.transport, result.name));
        }
    }

    // Phase 2: Wait for remaining providers up to 10s total
    let slow_deadline = Duration::from_secs(10);
    let slow_phase = async {
        while let Some(result) = join_set.join_next().await {
            match result {
                Ok((priority, name, Ok(transport))) => {
                    log::info!("discovery: {} connected (priority {})", name, priority);
                    let dominated = best.as_ref().is_some_and(|b| b.priority <= priority);
                    if !dominated {
                        best = Some(ConnectResult {
                            priority,
                            name,
                            transport,
                        });
                    }
                    // Got a direct connection, no need to wait for relay
                    if best.as_ref().is_some_and(|b| b.priority <= 1) {
                        return;
                    }
                }
                Ok((_priority, name, Err(e))) => {
                    log::debug!("discovery: {} failed: {}", name, e);
                    errors.push(format!("{}: {}", name, e));
                }
                Err(e) => {
                    errors.push(format!("task panic: {}", e));
                }
            }
        }
    };
    let _ = timeout(slow_deadline, slow_phase).await;

    join_set.abort_all();

    match best {
        Some(result) => {
            log::info!("discovery: using {}", result.name);
            Ok((result.transport, result.name))
        }
        None => {
            let detail = errors.join("; ");
            anyhow::bail!("discovery chain exhausted: all transport providers failed: {}", detail)
        }
    }
}
