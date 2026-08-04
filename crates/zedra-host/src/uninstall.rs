// `zedra uninstall`: reverses what `install.sh` and `zedra setup` put on this
// machine — running daemons, agent hooks, local state, and the binary itself.

use anyhow::{Context, Result};
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use crate::agent::setup::SetupCliCtx;
use crate::{agent, identity, uploads, utils, workspace_lock};

const STOP_GRACE_SECS: u64 = 5;

pub async fn run(assume_yes: bool) -> Result<()> {
    if !assume_yes && !std::io::stdin().is_terminal() {
        utils::eprintln_error("`zedra uninstall` needs a terminal to confirm. Re-run with --yes.");
        std::process::exit(1);
    }

    utils::eprintln_heading("Zedra Uninstall");
    eprintln!();

    let alive: Vec<_> = workspace_lock::scan_all_instances()
        .into_iter()
        .filter(|(_, _, alive)| *alive)
        .collect();
    let agents: Vec<_> = agent::actors()
        .iter()
        .filter(|actor| actor.supports_setup_cli() && actor.resolved_program().is_some())
        .collect();
    let state_dirs = state_dirs();
    let exe = std::env::current_exe()
        .ok()
        .map(|exe| exe.canonicalize().unwrap_or(exe));

    // Every prompt runs before anything is removed, so a late "no" cannot leave
    // the machine half-uninstalled.
    if !alive.is_empty() {
        utils::eprintln_step(format!("{} running daemon(s):", alive.len()));
        for (_, lock, _) in &alive {
            eprintln!("  pid {}  {}", lock.pid, lock.workdir);
        }
        if !confirm(
            "Stop them? Running terminals will be killed. Required to continue.",
            assume_yes,
        )? {
            utils::eprintln_note("Cancelled.");
            return Ok(());
        }
        eprintln!();
    }

    let remove_hooks = if agents.is_empty() {
        false
    } else {
        utils::eprintln_step("Agent integrations found:");
        for actor in &agents {
            eprintln!("  {}", actor.display_name());
        }
        let answer = confirm("Remove their Zedra plugins and hooks?", assume_yes)?;
        eprintln!();
        answer
    };

    let remove_state = if state_dirs.is_empty() {
        false
    } else {
        utils::eprintln_step("Local Zedra state:");
        for dir in &state_dirs {
            eprintln!("  {}", paths_display(dir));
        }
        let answer = confirm(
            "Delete it? Identity keys are lost and paired devices must scan a new QR.",
            assume_yes,
        )?;
        eprintln!();
        answer
    };

    let remove_binary = match &exe {
        Some(exe) => {
            utils::eprintln_step(format!("Zedra binary: {}", paths_display(exe)));
            let answer = confirm("Delete it?", assume_yes)?;
            eprintln!();
            answer
        }
        None => false,
    };

    if !remove_hooks && !remove_state && !remove_binary && alive.is_empty() {
        utils::eprintln_note("Nothing to uninstall.");
        return Ok(());
    }

    for (_, lock, _) in &alive {
        let workdir = PathBuf::from(&lock.workdir);
        match workspace_lock::kill_and_unlock(&workdir, STOP_GRACE_SECS) {
            Ok(()) => utils::eprintln_success(format!("Stopped daemon pid {}", lock.pid)),
            Err(error) => {
                utils::eprintln_warn(format!("Could not stop daemon pid {}: {error}", lock.pid))
            }
        }
    }

    if remove_hooks {
        let ctx = SetupCliCtx {
            full_bin_path: false,
            quiet: true,
        };
        for actor in &agents {
            if let Err(error) = agent::setup::run(actor.slug(), true, ctx).await {
                utils::eprintln_warn(format!(
                    "{} hook removal failed: {error}",
                    actor.display_name()
                ));
            }
        }
    }

    if remove_state {
        for dir in &state_dirs {
            match std::fs::remove_dir_all(dir) {
                Ok(()) => utils::eprintln_success(format!("Removed {}", paths_display(dir))),
                Err(error) => utils::eprintln_warn(format!(
                    "Could not remove {}: {error}",
                    paths_display(dir)
                )),
            }
        }
    }

    if remove_binary {
        if let Some(exe) = &exe {
            remove_self(exe)?;
        }
    }

    eprintln!();
    if remove_binary {
        utils::eprintln_success("Zedra uninstalled.");
    } else {
        utils::eprintln_success("Done. The zedra binary is still installed.");
    }
    if !remove_state && !state_dirs.is_empty() {
        utils::eprintln_note("Local state kept. Remove it later with:");
        for dir in &state_dirs {
            utils::eprintln_shell_command(format!("rm -rf {}", utils::shell_arg_path(dir)));
        }
    }
    Ok(())
}

/// Host-level directories Zedra creates outside the user's projects.
fn state_dirs() -> Vec<PathBuf> {
    [
        identity::zedra_config_dir().ok(),
        uploads::cache_root().ok(),
    ]
    .into_iter()
    .flatten()
    .filter(|dir| dir.exists())
    .collect()
}

fn paths_display(path: &Path) -> String {
    crate::paths::user_path_string(path)
}

#[cfg(not(windows))]
fn remove_self(exe: &Path) -> Result<()> {
    std::fs::remove_file(exe)
        .with_context(|| format!("failed to remove {}", paths_display(exe)))?;
    utils::eprintln_success(format!("Removed {}", paths_display(exe)));
    Ok(())
}

/// Windows keeps the running image locked, so the binary is renamed aside and
/// the leftover is left for the user (same trick the self-update path uses).
#[cfg(windows)]
fn remove_self(exe: &Path) -> Result<()> {
    let backup = exe.with_extension("old");
    let _ = std::fs::remove_file(&backup);
    std::fs::rename(exe, &backup)
        .with_context(|| format!("failed to move {} aside", paths_display(exe)))?;
    utils::eprintln_success(format!("Removed {}", paths_display(exe)));
    utils::eprintln_note(format!(
        "Delete the leftover after this process exits: {}",
        paths_display(&backup)
    ));
    Ok(())
}

/// Destructive prompt: an empty answer means no.
fn confirm(question: &str, assume_yes: bool) -> Result<bool> {
    if assume_yes {
        return Ok(true);
    }
    eprint!("{question} [y/N] ");
    std::io::stderr().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let input = input.trim();
    Ok(input.eq_ignore_ascii_case("y") || input.eq_ignore_ascii_case("yes"))
}
