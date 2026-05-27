//! Opt-in LSP control plane for the Zedra host.
//!
//! Ownership boundary: this crate owns *policy* (which languages are enabled
//! for a workspace, what binary serves them, what the resource caps are) and
//! *bookkeeping* (status snapshots, persistence). It does **not** spawn
//! language-server processes yet — the actual process supervisor lands in a
//! follow-up commit. Until then, `LspManager` exposes the same status surface
//! the runtime supervisor will use, with all servers in `Idle` state.
//!
//! Decoupling: `zedra-host` depends on this crate, but no other crate does.
//! Removing the dependency must cleanly disable LSP without touching unrelated
//! code paths.

pub mod guard;
pub mod manager;
pub mod persistence;
pub mod policy;
pub mod server;
pub mod supervisor;

pub use guard::{GUARD_DEFAULTS, GuardConfig};
pub use manager::LspManager;
pub use policy::{language_binary, supported_languages};
pub use server::LspServer;
pub use supervisor::Supervisor;
