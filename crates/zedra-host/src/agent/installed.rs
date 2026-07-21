use zedra_rpc::proto::{AgentInstalledListResult, InstalledAgentEntry};

use crate::agent;

pub fn list_installed_agents() -> AgentInstalledListResult {
    // Probe behind catch_unwind so one panicking actor is dropped from the
    // list instead of taking down the whole scan.
    let agents = agent::enabled_actors()
        .into_iter()
        .filter_map(|actor| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let launch_cmd =
                    crate::global_config::agent_launch_cmd(actor.slug(), actor.resolved_program());
                InstalledAgentEntry {
                    slug: actor.slug().to_string(),
                    display_name: actor.display_name().to_string(),
                    icon_name: actor.icon_name().to_string(),
                    available: launch_cmd.is_some(),
                    version: None,
                    launch_cmd,
                    web_client: actor.has_web_client(),
                }
            }))
            .map_err(|_| tracing::warn!("installed agent probe panicked for `{}`", actor.slug()))
            .ok()
        })
        .collect();

    AgentInstalledListResult {
        agents,
        error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_agent_list_includes_all_supported_slugs() {
        let result = list_installed_agents();
        assert_eq!(result.agents.len(), agent::actors().len());
        assert!(result
            .agents
            .iter()
            .any(|agent| agent.slug == "claude" && agent.icon_name == "claude"));
    }

    // Every icon_name must resolve to a real `assets/icons/<slug>.svg`; a typo
    // ships a missing native icon (no UIKit runtime fallback).
    #[test]
    fn every_icon_name_has_a_source_svg() {
        let icons_dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../zedra/assets/icons");
        for actor in agent::actors() {
            let svg = icons_dir.join(format!("{}.svg", actor.icon_name()));
            assert!(
                svg.is_file(),
                "icon_name `{}` (agent `{}`) has no source svg at {}",
                actor.icon_name(),
                actor.slug(),
                svg.display()
            );
        }
    }
}
