use zedra_rpc::proto::{AgentInstalledListResult, InstalledAgentEntry};

use crate::agent;

pub fn list_installed_agents() -> AgentInstalledListResult {
    // Join each probe by hand so one panicking actor is dropped from the list
    // instead of unwinding the whole scope. Join order preserves registry order.
    let agents = std::thread::scope(|scope| {
        let handles: Vec<_> = agent::actors()
            .iter()
            .map(|actor| {
                scope.spawn(move || {
                    let launch_cmd = resolve_program(actor.programs());
                    InstalledAgentEntry {
                        slug: actor.slug().to_string(),
                        display_name: actor.display_name().to_string(),
                        icon_name: actor.icon_name().to_string(),
                        available: launch_cmd.is_some(),
                        version: None,
                        launch_cmd,
                    }
                })
            })
            .collect();
        handles
            .into_iter()
            .zip(agent::actors())
            .filter_map(|(handle, actor)| match handle.join() {
                Ok(entry) => Some(entry),
                Err(_) => {
                    tracing::warn!("installed agent probe panicked for `{}`", actor.slug());
                    None
                }
            })
            .collect()
    });

    AgentInstalledListResult {
        agents,
        error: None,
    }
}

/// Resolve the launch command like `AgentActor::cli_available`, so availability matches.
fn resolve_program(programs: &[&str]) -> Option<String> {
    programs
        .iter()
        .find(|program| agent::utils::command_on_path(program))
        .map(|program| program.to_string())
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

    // Every actor's icon_name is a bare slug that must resolve to a real
    // `assets/icons/<slug>.svg`; a typo would otherwise ship a missing native
    // icon (no runtime fallback in UIKit). Source of truth is the zedra crate.
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
