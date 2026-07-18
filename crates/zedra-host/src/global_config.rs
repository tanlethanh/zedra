// global_config.rs — machine-wide user config at `~/.config/zedra/config.yaml`.
// Layered UNDER per-workspace `config.yaml` and CLI flags / env vars: a value
// here is a default that an explicit flag or env var overrides. Missing or
// malformed files fall back to defaults so the daemon always starts.

use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

use serde::Deserialize;

pub const FILE_NAME: &str = "config.yaml";
/// Per-workspace config lives at `<workdir>/.zedra/config.yaml`.
pub const WORKSPACE_DIR: &str = ".zedra";

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct GlobalConfig {
    /// Default relay URL(s); overridden by `zedra start --relay-url`.
    pub relay_url: Vec<String>,
    /// Disable anonymous telemetry machine-wide.
    pub no_telemetry: bool,
    /// PTY shell override; takes precedence over the inherited `$SHELL`.
    pub shell: Option<String>,
    /// Max concurrent managed-agent sessions; `ZEDRA_AGENT_SESSION_LIMIT` wins.
    pub agent_session_limit: Option<u32>,
    /// Per-agent overrides keyed by registry slug (e.g. `hermes`).
    pub agents: HashMap<String, AgentConfig>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct AgentConfig {
    /// Executable path/name to launch instead of the adapter's default.
    pub bin: Option<String>,
    /// Full launch command run verbatim (e.g. `hermes --tui`); wins over `bin`.
    pub launch_cmd: Option<String>,
}

static CACHE: OnceLock<GlobalConfig> = OnceLock::new();

/// Resolve the config for `workdir`, merging `<workdir>/.zedra/config.yaml`
/// over the global file, and cache it process-wide. A daemon serves one
/// workspace, so the first caller sets the value and later calls are no-ops.
pub fn init(workdir: &Path) {
    let _ = CACHE.set(load_merged(Some(workdir)));
}

/// Cached config. Falls back to the global file alone when [`init`] never ran
/// (workspace-agnostic paths such as `zedra agent installed`).
pub fn get() -> &'static GlobalConfig {
    CACHE.get_or_init(|| load_merged(None))
}

fn load_merged(workdir: Option<&Path>) -> GlobalConfig {
    let global = crate::identity::zedra_config_dir()
        .ok()
        .map(|dir| load_file(&dir.join(FILE_NAME)))
        .unwrap_or_default();
    match workdir {
        Some(wd) => global.merged_with(load_file(&wd.join(WORKSPACE_DIR).join(FILE_NAME))),
        None => global,
    }
}

fn load_file(path: &Path) -> GlobalConfig {
    match std::fs::read_to_string(path) {
        Ok(contents) => parse(&contents).unwrap_or_else(|e| {
            tracing::warn!(
                "global_config: failed to parse {}, using defaults: {e}",
                path.display()
            );
            GlobalConfig::default()
        }),
        Err(_) => GlobalConfig::default(),
    }
}

fn parse(contents: &str) -> Result<GlobalConfig, serde_yaml::Error> {
    serde_yaml::from_str(contents)
}

impl GlobalConfig {
    pub fn agent(&self, slug: &str) -> Option<&AgentConfig> {
        self.agents.get(slug)
    }

    /// Overlay `over` (the per-workspace file) onto `self` (global). Each set
    /// value in `over` wins; `no_telemetry` sticks once either level sets it.
    fn merged_with(mut self, over: GlobalConfig) -> GlobalConfig {
        if !over.relay_url.is_empty() {
            self.relay_url = over.relay_url;
        }
        self.no_telemetry |= over.no_telemetry;
        self.shell = over.shell.or(self.shell);
        self.agent_session_limit = over.agent_session_limit.or(self.agent_session_limit);
        for (slug, agent) in over.agents {
            let base = self.agents.entry(slug).or_default();
            base.bin = agent.bin.or(base.bin.take());
            base.launch_cmd = agent.launch_cmd.or(base.launch_cmd.take());
        }
        self
    }
}

/// Launch command advertised for `slug`: `launch_cmd` verbatim, else `bin`,
/// else `fallback` (the adapter's resolved program). `None` when the agent has
/// no configured override and no program on PATH.
pub fn agent_launch_cmd(slug: &str, fallback: Option<&str>) -> Option<String> {
    let cfg = get().agent(slug);
    cfg.and_then(|c| c.launch_cmd.clone())
        .or_else(|| cfg.and_then(|c| c.bin.clone()))
        .or_else(|| fallback.map(str::to_string))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_config() {
        let cfg = parse(
            "relay_url:\n  - https://sg1.relay.zedra.dev\nno_telemetry: true\nshell: /bin/zsh\nagent_session_limit: 12\nagents:\n  hermes:\n    launch_cmd: hermes --tui\n  claude:\n    bin: /opt/claude\n",
        )
        .unwrap();
        assert_eq!(cfg.relay_url, vec!["https://sg1.relay.zedra.dev"]);
        assert!(cfg.no_telemetry);
        assert_eq!(cfg.shell.as_deref(), Some("/bin/zsh"));
        assert_eq!(cfg.agent_session_limit, Some(12));
        assert_eq!(
            cfg.agent("hermes").and_then(|a| a.launch_cmd.as_deref()),
            Some("hermes --tui")
        );
        assert_eq!(
            cfg.agent("claude").and_then(|a| a.bin.as_deref()),
            Some("/opt/claude")
        );
    }

    #[test]
    fn empty_config_is_default() {
        assert_eq!(parse("").unwrap_or_default(), GlobalConfig::default());
        assert_eq!(parse("{}").unwrap(), GlobalConfig::default());
    }

    #[test]
    fn workspace_overrides_global() {
        let global = parse(
            "no_telemetry: false\nshell: /bin/bash\nagent_session_limit: 50\nagents:\n  claude:\n    bin: /usr/bin/claude\n  hermes:\n    launch_cmd: hermes\n",
        )
        .unwrap();
        let workspace = parse(
            "no_telemetry: true\nshell: /bin/zsh\nagents:\n  hermes:\n    launch_cmd: hermes --tui\n",
        )
        .unwrap();
        let merged = global.merged_with(workspace);
        // Workspace wins where set...
        assert!(merged.no_telemetry);
        assert_eq!(merged.shell.as_deref(), Some("/bin/zsh"));
        assert_eq!(
            merged.agent("hermes").and_then(|a| a.launch_cmd.as_deref()),
            Some("hermes --tui")
        );
        // ...and global survives where the workspace is silent.
        assert_eq!(merged.agent_session_limit, Some(50));
        assert_eq!(
            merged.agent("claude").and_then(|a| a.bin.as_deref()),
            Some("/usr/bin/claude")
        );
    }

    #[test]
    fn launch_cmd_precedence() {
        let cfg =
            parse("agents:\n  a:\n    bin: /x/a\n    launch_cmd: a --flag\n  b:\n    bin: /x/b\n")
                .unwrap();
        // launch_cmd wins over bin
        assert_eq!(
            cfg.agent("a").and_then(|a| a.launch_cmd.clone()),
            Some("a --flag".to_string())
        );
        assert_eq!(
            cfg.agent("b").and_then(|a| a.bin.clone()),
            Some("/x/b".to_string())
        );
    }
}
