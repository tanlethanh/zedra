use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;
use zedra_rpc::proto::*;

use crate::agent::home_path;

const SKILL_NAMES: &[&str] = &["zedra-start"];

/// Temporary kill-switch for hooks setup/detection in release builds.
/// Flip to `true` (or remove the gate) when re-enabling.
pub const fn hooks_enabled() -> bool {
    cfg!(debug_assertions)
}

pub fn setup_summary(kind: AgentKind, cli_available: bool, workdir: &Path) -> AgentSetupSummary {
    if !cli_available {
        return AgentSetupSummary {
            state: AgentSetupState::MissingCli,
            skills_installed: false,
            plugin_installed: false,
            hooks_installed: false,
            error: None,
        };
    }

    let mut error = None;
    let (skills_installed, plugin_installed, hooks_installed) = match kind {
        AgentKind::Claude => {
            let status = claude_plugin_status();
            error = status.error;
            (
                false,
                status.plugin_installed,
                hooks_enabled()
                    && (status.hooks_installed || claude_local_hooks_installed(workdir)),
            )
        }
        AgentKind::Codex => {
            let plugin_installed = codex_plugin_installed();
            (
                false,
                plugin_installed,
                hooks_enabled()
                    && (codex_hooks_installed() || codex_local_hooks_installed(workdir)),
            )
        }
        AgentKind::OpenCode => (
            skills_installed_at(&home_path(&[".config", "opencode", "skills"])),
            opencode_plugin_installed(),
            hooks_enabled()
                && (opencode_hooks_installed() || opencode_local_hooks_installed(workdir)),
        ),
        AgentKind::Pi => (false, false, hooks_enabled() && pi_hooks_installed()),
        // Hermes has its own hook system, but Zedra doesn't install into it yet.
        AgentKind::Hermes => (false, false, false),
        // Maki has no external lifecycle-hook mechanism, so Zedra installs
        // nothing; session discovery and resume still work without hooks.
        AgentKind::Maki => (false, false, false),
    };
    let state = if error.is_some() {
        AgentSetupState::Error
    } else if hooks_installed {
        AgentSetupState::HooksReady
    } else if skills_installed || plugin_installed {
        AgentSetupState::SkillsOnly
    } else {
        AgentSetupState::NotConfigured
    };

    AgentSetupSummary {
        state,
        skills_installed,
        plugin_installed,
        hooks_installed,
        error,
    }
}

#[derive(Default)]
pub struct ClaudePluginStatus {
    pub plugin_installed: bool,
    pub hooks_installed: bool,
    pub error: Option<String>,
}

#[derive(Deserialize)]
struct ClaudeInstalledPluginsFile {
    plugins: HashMap<String, Vec<ClaudeInstalledPluginEntry>>,
}

#[derive(Deserialize)]
struct ClaudeInstalledPluginEntry {
    #[serde(rename = "installPath")]
    install_path: String,
}

const CLAUDE_ZEDRA_PLUGIN_ID: &str = "zedra@zedra";

fn claude_plugin_status() -> ClaudePluginStatus {
    let path = home_path(&[".claude", "plugins", "installed_plugins.json"]);
    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return ClaudePluginStatus::default();
        }
        Err(error) => {
            return ClaudePluginStatus {
                error: Some(error.to_string()),
                ..ClaudePluginStatus::default()
            };
        }
    };
    claude_plugin_status_from_installed_plugins(&contents)
}

pub fn claude_plugin_status_from_installed_plugins(contents: &str) -> ClaudePluginStatus {
    let installed: ClaudeInstalledPluginsFile = match serde_json::from_str(contents) {
        Ok(installed) => installed,
        Err(error) => {
            return ClaudePluginStatus {
                error: Some(error.to_string()),
                ..ClaudePluginStatus::default()
            };
        }
    };
    let Some(entry) = installed
        .plugins
        .get(CLAUDE_ZEDRA_PLUGIN_ID)
        .and_then(|entries| entries.first())
    else {
        return ClaudePluginStatus::default();
    };
    let install_path = Path::new(&entry.install_path);
    let hooks_installed = install_path.join("hooks").join("hooks.json").is_file();
    ClaudePluginStatus {
        plugin_installed: true,
        hooks_installed,
        error: None,
    }
}

fn skills_installed_at(base: &Path) -> bool {
    SKILL_NAMES
        .iter()
        .all(|skill| base.join(skill).join("SKILL.md").is_file())
}

fn codex_hooks_installed() -> bool {
    std::fs::read_to_string(home_path(&[".codex", "config.toml"]))
        .map(|contents| contents.contains("zedra") && contents.contains("hook"))
        .unwrap_or(false)
}

fn codex_plugin_installed() -> bool {
    std::fs::read_to_string(home_path(&[".codex", "config.toml"]))
        .map(|contents| codex_plugin_installed_from_config(&contents))
        .unwrap_or(false)
}

pub fn codex_plugin_installed_from_config(contents: &str) -> bool {
    let mut in_plugin_section = false;
    for line in contents.lines().map(str::trim) {
        if line.starts_with('[') {
            in_plugin_section = line == r#"[plugins."zedra@zedra"]"#;
            continue;
        }
        if in_plugin_section && line == "enabled = true" {
            return true;
        }
    }
    false
}

fn claude_local_hooks_installed(workdir: &Path) -> bool {
    hook_file_mentions_zedra(&workdir.join(".claude/settings.local.json"))
}

fn codex_local_hooks_installed(workdir: &Path) -> bool {
    hook_file_mentions_zedra(&workdir.join(".codex/hooks.json"))
}

fn opencode_plugin_installed() -> bool {
    home_path(&[".config", "opencode", "plugins", "zedra-agent-hooks.js"]).is_file()
}

fn opencode_hooks_installed() -> bool {
    opencode_plugin_installed()
}

fn opencode_local_hooks_installed(workdir: &Path) -> bool {
    hook_file_mentions_zedra(&workdir.join(".opencode/plugins/zedra.js"))
}

fn pi_hooks_installed() -> bool {
    hook_file_mentions_zedra(&home_path(&[
        ".pi",
        "agent",
        "extensions",
        "zedra-agent-hooks.ts",
    ]))
}

fn hook_file_mentions_zedra(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .map(|contents| contents.contains("zedra-agent-hook") || contents.contains("agent hook"))
        .unwrap_or(false)
}
