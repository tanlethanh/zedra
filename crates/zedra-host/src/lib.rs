// zedra-host library — re-exports for integration tests

pub mod agent;
pub mod agent_cache;
pub mod agent_claude;
#[cfg(unix)]
mod agent_claude_probe;
pub mod agent_codex;
pub mod agent_hermes;
pub mod agent_hook_recv;
pub mod agent_installed;
pub mod agent_opencode;
pub mod agent_pi;
pub mod agent_setup;
pub mod agent_utils;
pub mod api;
pub mod client;
pub mod delta;
pub mod docs_tree;
pub mod fs;
#[cfg(feature = "telemetry")]
pub mod ga4;
#[cfg(not(feature = "telemetry"))]
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
pub mod workspace_lock;
