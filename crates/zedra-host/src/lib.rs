// zedra-host library — re-exports for integration tests

pub mod account_usage;
pub mod agent;
pub mod agent_cache;
pub mod agent_setup;
pub mod api;
pub mod claude;
#[cfg(unix)]
mod claude_cli_probe;
pub mod client;
pub mod docs_tree;
pub mod fs;
pub mod ga4;
pub mod git;
pub mod host_info;
pub mod identity;
pub mod installed_agents;
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
