// zedra-host library — re-exports for integration tests

pub mod agent;
pub mod api;
pub mod client;
pub mod delta;
pub mod docs_tree;
pub mod fs;
#[cfg(all(feature = "telemetry", not(feature = "no-telemetry")))]
pub mod ga4;
#[cfg(any(not(feature = "telemetry"), feature = "no-telemetry"))]
#[path = "ga4_stub.rs"]
pub mod ga4;
pub mod git;
pub mod host_info;
pub mod identity;
pub mod iroh_listener;
pub mod metrics;
pub mod net_monitor;
pub mod paths;
pub mod pty;
pub mod qr;
pub mod rpc_daemon;
pub mod session_registry;
pub mod sqlite_readonly;
pub mod telemetry;
pub mod utils;
pub mod version_check;
pub mod web_client;
pub mod workspace_lock;
