use std::path::{Path, PathBuf};
use std::sync::Mutex;
use zedra_rpc::proto::{AgentInstalledListResult, InstalledAgentEntry};

struct InstalledAgentSpec {
    slug: &'static str,
    display_name: &'static str,
    icon_name: &'static str,
    programs: &'static [&'static str],
}

const INSTALLED_AGENT_SPECS: &[InstalledAgentSpec] = &[
    InstalledAgentSpec {
        slug: "claude",
        display_name: "Claude Code",
        icon_name: "claude",
        programs: &["claude"],
    },
    InstalledAgentSpec {
        slug: "codex",
        display_name: "Codex",
        icon_name: "openai",
        programs: &["codex"],
    },
    InstalledAgentSpec {
        slug: "opencode",
        display_name: "OpenCode",
        icon_name: "opencode",
        programs: &["opencode"],
    },
    InstalledAgentSpec {
        slug: "amp",
        display_name: "Amp",
        icon_name: "amp",
        programs: &["amp"],
    },
    InstalledAgentSpec {
        slug: "cline",
        display_name: "Cline",
        icon_name: "cline",
        programs: &["cline"],
    },
    InstalledAgentSpec {
        slug: "cursor",
        display_name: "Cursor Agent",
        icon_name: "cursor",
        programs: &["cursor-agent", "cursor"],
    },
    InstalledAgentSpec {
        slug: "copilot",
        display_name: "GitHub Copilot",
        icon_name: "githubcopilot",
        programs: &["copilot", "github-copilot-cli"],
    },
    InstalledAgentSpec {
        slug: "gemini",
        display_name: "Gemini",
        icon_name: "gemini",
        programs: &["gemini"],
    },
    InstalledAgentSpec {
        slug: "goose",
        display_name: "Goose",
        icon_name: "goose",
        programs: &["goose"],
    },
    InstalledAgentSpec {
        slug: "hermes",
        display_name: "Hermes Agent",
        icon_name: "hermesagent",
        programs: &["hermes", "hermes-agent"],
    },
    InstalledAgentSpec {
        slug: "junie",
        display_name: "Junie",
        icon_name: "junie",
        programs: &["junie"],
    },
    InstalledAgentSpec {
        slug: "kilocode",
        display_name: "Kilo Code",
        icon_name: "kilocode",
        programs: &["kilocode"],
    },
    InstalledAgentSpec {
        slug: "openclaw",
        display_name: "OpenClaw",
        icon_name: "openclaw",
        programs: &["openclaw"],
    },
    InstalledAgentSpec {
        slug: "openhands",
        display_name: "OpenHands",
        icon_name: "openhands",
        programs: &["openhands"],
    },
    InstalledAgentSpec {
        slug: "pi",
        display_name: "Pi",
        icon_name: "pi",
        programs: &["pi"],
    },
    InstalledAgentSpec {
        slug: "qoder",
        display_name: "Qoder",
        icon_name: "qoder",
        programs: &["qoder"],
    },
    InstalledAgentSpec {
        slug: "qwen",
        display_name: "Qwen Code",
        icon_name: "qwen",
        programs: &["qwen"],
    },
    InstalledAgentSpec {
        slug: "trae",
        display_name: "Trae Agent",
        icon_name: "trae",
        programs: &["trae"],
    },
    InstalledAgentSpec {
        slug: "zencoder",
        display_name: "Zencoder",
        icon_name: "zencoder",
        programs: &["zencoder"],
    },
];

pub fn list_installed_agents() -> AgentInstalledListResult {
    let agents = Mutex::new(Vec::with_capacity(INSTALLED_AGENT_SPECS.len()));
    std::thread::scope(|scope| {
        for spec in INSTALLED_AGENT_SPECS {
            let agents = &agents;
            scope.spawn(move || {
                let launch_cmd = resolve_program(spec.programs);
                let entry = InstalledAgentEntry {
                    slug: spec.slug.to_string(),
                    display_name: spec.display_name.to_string(),
                    icon_name: spec.icon_name.to_string(),
                    available: launch_cmd.is_some(),
                    version: None,
                    launch_cmd,
                };
                agents
                    .lock()
                    .expect("installed agent probe lock")
                    .push(entry);
            });
        }
    });

    let mut agents = agents.into_inner().expect("installed agent probe lock");
    agents.sort_by_key(|entry| {
        INSTALLED_AGENT_SPECS
            .iter()
            .position(|spec| spec.slug == entry.slug)
            .unwrap_or(usize::MAX)
    });

    AgentInstalledListResult {
        agents,
        error: None,
    }
}

fn resolve_program(programs: &[&str]) -> Option<String> {
    programs
        .iter()
        .find(|program| program_on_path(program))
        .map(|program| program.to_string())
}

fn program_on_path(program: &str) -> bool {
    if program.contains('/') {
        return is_executable_file(Path::new(program));
    }
    path_lookup(program).is_some()
}

fn path_lookup(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(program);
        if is_executable_file(&candidate) {
            return Some(candidate);
        }
    }
    None
}

/// A PATH entry only counts as an installed agent CLI if it is an executable
/// regular file; a non-executable file of the same name (a stray note, a
/// partial download) must not mark the agent as available.
fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_agent_list_includes_all_supported_slugs() {
        let result = list_installed_agents();
        assert_eq!(result.agents.len(), INSTALLED_AGENT_SPECS.len());
        assert!(result
            .agents
            .iter()
            .any(|agent| agent.slug == "claude" && agent.icon_name == "claude"));
    }

    // Every shipped icon_name must map to a real `assets/icons/<slug>.svg`; a typo
    // in any rename would otherwise ship a missing native icon. Source of truth lives
    // in the zedra crate, alongside this workspace.
    #[test]
    fn every_icon_name_has_a_source_svg() {
        let icons_dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../zedra/assets/icons");
        for spec in INSTALLED_AGENT_SPECS {
            let svg = icons_dir.join(format!("{}.svg", spec.icon_name));
            assert!(
                svg.is_file(),
                "icon_name `{}` (agent `{}`) has no source svg at {}",
                spec.icon_name,
                spec.slug,
                svg.display()
            );
        }
    }
}
