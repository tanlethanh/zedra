// global_config.rs — machine-wide user config at `~/.config/zedra/config.yaml`,
// overlaid by a per-workspace `<workdir>/.zedra/config.yaml`. Both are layered
// UNDER env vars and CLI flags: a value here is a default an explicit flag or
// env var overrides. Missing or malformed files fall back to defaults so the
// daemon always starts.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::OnceLock;

use serde::Deserialize;

pub const FILE_NAME: &str = "config.yaml";
/// Per-workspace config lives at `<workdir>/.zedra/config.yaml`.
pub const WORKSPACE_DIR: &str = ".zedra";

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct GlobalConfig {
    pub network: NetworkConfig,
    pub telemetry: TelemetryConfig,
    pub terminal: TerminalConfig,
    pub agents: AgentsConfig,
    pub update: UpdateConfig,
    pub workspace: WorkspaceConfig,
    pub logging: LoggingConfig,
    pub git: GitConfig,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct GitConfig {
    /// How `git status` reports untracked files. `all` (default) lists every
    /// untracked file; `normal` collapses untracked directories; `no` omits them.
    pub untracked: Option<UntrackedFiles>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UntrackedFiles {
    All,
    Normal,
    No,
}

impl GitConfig {
    pub fn untracked_flag(&self) -> &'static str {
        match self.untracked.unwrap_or(UntrackedFiles::All) {
            UntrackedFiles::All => "--untracked-files=all",
            UntrackedFiles::Normal => "--untracked-files=normal",
            UntrackedFiles::No => "--untracked-files=no",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct NetworkConfig {
    /// Default relay URL(s); overridden by `zedra start --relay-url`.
    pub relay_url: Vec<String>,
    /// Force relay-only mode (no hole punching); `--relay-only` also enables it.
    pub relay_only: bool,
    /// Seconds a one-time pairing QR stays valid. Lower it to shrink the window
    /// a leaked QR can be used. Clamped to [30, 3600]; default 600.
    pub pairing_ttl_secs: Option<u64>,
}

impl NetworkConfig {
    /// Clamped pairing TTL, or `default` when unset.
    pub fn pairing_ttl(&self, default: u64) -> u64 {
        self.pairing_ttl_secs
            .map(|v| v.clamp(30, 3600))
            .unwrap_or(default)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct TelemetryConfig {
    /// Disable anonymous telemetry machine-wide.
    pub disabled: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct TerminalConfig {
    /// Shell override; takes precedence over the inherited `$SHELL`.
    pub shell: Option<String>,
    /// Extra environment injected into every terminal (wins over `env_passthrough`).
    pub env: BTreeMap<String, String>,
    /// Host env var names allowed through the sanitizer into terminals.
    /// Opt-in: these are forwarded to a phone-controlled shell.
    pub env_passthrough: Vec<String>,
    /// Reconnect-replay backlog depth (PTY chunks). Clamped to [1000, 500000].
    pub scrollback: Option<usize>,
    /// Max concurrent terminals per session. Clamped to [1, 64].
    pub max_terminals: Option<usize>,
}

impl TerminalConfig {
    pub fn scrollback_limit(&self, default: usize) -> usize {
        self.scrollback
            .map(|v| v.clamp(1000, 500_000))
            .unwrap_or(default)
    }

    pub fn max_terminals_limit(&self, default: usize) -> usize {
        self.max_terminals
            .map(|v| v.clamp(1, 64))
            .unwrap_or(default)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct AgentsConfig {
    /// Max concurrent managed-agent sessions; `ZEDRA_AGENT_SESSION_LIMIT` wins.
    pub session_limit: Option<u32>,
    /// Registry slugs hidden from agent lists and skipped in scans.
    pub disabled: Vec<String>,
    /// Per-agent launch overrides keyed by registry slug (e.g. `hermes`).
    pub overrides: HashMap<String, AgentConfig>,
    /// Default for `--usage-refresh-secs` (live-usage poll cadence; 0 disables).
    pub usage_refresh_secs: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct AgentConfig {
    /// Executable path/name to launch instead of the adapter's default.
    pub bin: Option<String>,
    /// Full launch command run verbatim (e.g. `hermes --tui`); wins over `bin`.
    pub launch_cmd: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct UpdateConfig {
    /// Whether `zedra update` restarts running daemons afterward.
    pub restart: RestartPolicy,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct WorkspaceConfig {
    /// Display name shown on the phone instead of the workdir's folder name.
    pub name: Option<String>,
    /// Label shown for this host instead of the system hostname.
    pub host_label: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct LoggingConfig {
    /// Non-verbose log filter (`error`/`warn`/`info`/`debug`). `--verbose` and
    /// `RUST_LOG` still win. Read from the global file only (set before the
    /// workspace is known).
    pub level: Option<String>,
}

/// Restart-after-update behavior. Read from the global file only — `zedra
/// update` spans every workspace, so it has no single workdir to key on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RestartPolicy {
    /// Prompt when a terminal is attached; default No.
    #[default]
    Ask,
    /// Always restart without prompting.
    Always,
    /// Never restart; print the manual command instead.
    Never,
}

static CACHE: OnceLock<GlobalConfig> = OnceLock::new();

/// Resolve the config for `workdir`, merging `<workdir>/.zedra/config.yaml`
/// over the global file, and cache it process-wide. A daemon serves one
/// workspace, so the first caller sets the value and later calls are no-ops.
pub fn init(workdir: &Path) {
    let _ = CACHE.set(load_merged(Some(workdir)));
}

/// Cached config. Falls back to the global file alone when [`init`] never ran
/// (workspace-agnostic paths such as `zedra update` or `zedra agent installed`).
pub fn get() -> &'static GlobalConfig {
    CACHE.get_or_init(|| load_merged(None))
}

/// Load only the global file, bypassing the cache. Used before the workspace is
/// known (e.g. log-level setup) so an early read can't lock in a global-only
/// cache and defeat per-workspace merging in [`init`].
pub fn global_only() -> GlobalConfig {
    load_merged(None)
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
        self.agents.overrides.get(slug)
    }

    /// True when `slug` is listed under `agents.disabled` (case-insensitive).
    pub fn agent_disabled(&self, slug: &str) -> bool {
        self.agents
            .disabled
            .iter()
            .any(|d| d.eq_ignore_ascii_case(slug))
    }

    /// Overlay `over` (the per-workspace file) onto `self` (global). Each set
    /// value in `over` wins; booleans and list allowlists accumulate. `update`
    /// stays global-only.
    fn merged_with(mut self, over: GlobalConfig) -> GlobalConfig {
        if !over.network.relay_url.is_empty() {
            self.network.relay_url = over.network.relay_url;
        }
        self.network.relay_only |= over.network.relay_only;
        self.network.pairing_ttl_secs = over
            .network
            .pairing_ttl_secs
            .or(self.network.pairing_ttl_secs);
        self.telemetry.disabled |= over.telemetry.disabled;

        self.terminal.shell = over.terminal.shell.or(self.terminal.shell);
        self.terminal.env.extend(over.terminal.env);
        extend_unique(
            &mut self.terminal.env_passthrough,
            over.terminal.env_passthrough,
        );
        self.terminal.scrollback = over.terminal.scrollback.or(self.terminal.scrollback);
        self.terminal.max_terminals = over.terminal.max_terminals.or(self.terminal.max_terminals);

        self.agents.session_limit = over.agents.session_limit.or(self.agents.session_limit);
        self.agents.usage_refresh_secs = over
            .agents
            .usage_refresh_secs
            .or(self.agents.usage_refresh_secs);
        extend_unique(&mut self.agents.disabled, over.agents.disabled);
        for (slug, agent) in over.agents.overrides {
            let base = self.agents.overrides.entry(slug).or_default();
            base.bin = agent.bin.or(base.bin.take());
            base.launch_cmd = agent.launch_cmd.or(base.launch_cmd.take());
        }

        self.workspace.name = over.workspace.name.or(self.workspace.name);
        self.workspace.host_label = over.workspace.host_label.or(self.workspace.host_label);
        self.git.untracked = over.git.untracked.or(self.git.untracked);
        self
    }
}

/// Append items from `extra` not already in `base` (case-insensitive), keeping order.
fn extend_unique(base: &mut Vec<String>, extra: Vec<String>) {
    for item in extra {
        if !base.iter().any(|b| b.eq_ignore_ascii_case(&item)) {
            base.push(item);
        }
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
            "network:\n  relay_url:\n    - https://sg1.relay.zedra.dev\n  relay_only: true\n  pairing_ttl_secs: 60\ntelemetry:\n  disabled: true\nterminal:\n  shell: /bin/zsh\n  env:\n    EDITOR: nvim\n  env_passthrough:\n    - GH_TOKEN\n  scrollback: 10000\n  max_terminals: 8\nagents:\n  session_limit: 12\n  usage_refresh_secs: 900\n  disabled:\n    - pi\n  overrides:\n    hermes:\n      launch_cmd: hermes --tui\nupdate:\n  restart: always\nworkspace:\n  name: My Project\n  host_label: build-box\nlogging:\n  level: info\ngit:\n  untracked: no\n",
        )
        .unwrap();
        assert_eq!(cfg.network.relay_url, vec!["https://sg1.relay.zedra.dev"]);
        assert!(cfg.network.relay_only);
        assert_eq!(cfg.network.pairing_ttl(600), 60);
        assert!(cfg.telemetry.disabled);
        assert_eq!(cfg.terminal.shell.as_deref(), Some("/bin/zsh"));
        assert_eq!(
            cfg.terminal.env.get("EDITOR").map(String::as_str),
            Some("nvim")
        );
        assert_eq!(cfg.terminal.env_passthrough, vec!["GH_TOKEN"]);
        assert_eq!(cfg.terminal.scrollback_limit(50_000), 10_000);
        assert_eq!(cfg.terminal.max_terminals_limit(16), 8);
        assert_eq!(cfg.agents.session_limit, Some(12));
        assert_eq!(cfg.agents.usage_refresh_secs, Some(900));
        assert!(cfg.agent_disabled("Pi"));
        assert_eq!(
            cfg.agent("hermes").and_then(|a| a.launch_cmd.as_deref()),
            Some("hermes --tui")
        );
        assert_eq!(cfg.update.restart, RestartPolicy::Always);
        assert_eq!(cfg.workspace.name.as_deref(), Some("My Project"));
        assert_eq!(cfg.workspace.host_label.as_deref(), Some("build-box"));
        assert_eq!(cfg.logging.level.as_deref(), Some("info"));
        assert_eq!(cfg.git.untracked_flag(), "--untracked-files=no");
    }

    #[test]
    fn accessors_clamp_and_default() {
        // Out-of-range values are clamped; unset falls back to the passed default.
        let cfg = parse(
            "network:\n  pairing_ttl_secs: 5\nterminal:\n  scrollback: 10\n  max_terminals: 999\n",
        )
        .unwrap();
        assert_eq!(cfg.network.pairing_ttl(600), 30); // min 30
        assert_eq!(cfg.terminal.scrollback_limit(50_000), 1000); // min 1000
        assert_eq!(cfg.terminal.max_terminals_limit(16), 64); // max 64

        let empty = GlobalConfig::default();
        assert_eq!(empty.network.pairing_ttl(600), 600);
        assert_eq!(empty.terminal.scrollback_limit(50_000), 50_000);
        assert_eq!(empty.terminal.max_terminals_limit(16), 16);
        assert_eq!(empty.git.untracked_flag(), "--untracked-files=all");
    }

    #[test]
    fn empty_config_is_default() {
        assert_eq!(parse("").unwrap_or_default(), GlobalConfig::default());
        assert_eq!(parse("{}").unwrap(), GlobalConfig::default());
        assert_eq!(GlobalConfig::default().update.restart, RestartPolicy::Ask);
    }

    #[test]
    fn workspace_overrides_global() {
        let global = parse(
            "terminal:\n  shell: /bin/bash\n  env:\n    EDITOR: vi\n    LANG: en_US.UTF-8\nagents:\n  session_limit: 50\n  disabled:\n    - pi\n  overrides:\n    claude:\n      bin: /usr/bin/claude\n    hermes:\n      launch_cmd: hermes\n",
        )
        .unwrap();
        let workspace = parse(
            "telemetry:\n  disabled: true\nterminal:\n  shell: /bin/zsh\n  env:\n    EDITOR: nvim\nagents:\n  disabled:\n    - maki\n  overrides:\n    hermes:\n      launch_cmd: hermes --tui\n",
        )
        .unwrap();
        let merged = global.merged_with(workspace);
        // Workspace wins where set...
        assert!(merged.telemetry.disabled);
        assert_eq!(merged.terminal.shell.as_deref(), Some("/bin/zsh"));
        assert_eq!(
            merged.terminal.env.get("EDITOR").map(String::as_str),
            Some("nvim")
        );
        assert_eq!(
            merged.agent("hermes").and_then(|a| a.launch_cmd.as_deref()),
            Some("hermes --tui")
        );
        // ...global survives where the workspace is silent...
        assert_eq!(
            merged.terminal.env.get("LANG").map(String::as_str),
            Some("en_US.UTF-8")
        );
        assert_eq!(merged.agents.session_limit, Some(50));
        assert_eq!(
            merged.agent("claude").and_then(|a| a.bin.as_deref()),
            Some("/usr/bin/claude")
        );
        // ...and disabled lists accumulate across both files.
        assert!(merged.agent_disabled("pi"));
        assert!(merged.agent_disabled("maki"));
    }

    #[test]
    fn launch_cmd_precedence() {
        let cfg = parse(
            "agents:\n  overrides:\n    a:\n      bin: /x/a\n      launch_cmd: a --flag\n    b:\n      bin: /x/b\n",
        )
        .unwrap();
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
