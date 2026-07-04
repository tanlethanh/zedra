//! Interactive `zedra setup` CLI: registry-driven dispatch to each actor's
//! `setup_cli`, plus the `SetupCliCtx` toolkit those flows use. Actors depend
//! only on the ctx handed to them, never on this module directly.

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use crate::utils;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupAction {
    Install,
    Remove,
}

/// Flags threaded from the `zedra setup` command line into actor setup flows.
#[derive(Clone, Copy, Debug, Default)]
pub struct SetupCliCtx {
    /// Embed the absolute current binary path in hooks instead of bare `zedra`.
    pub full_bin_path: bool,
    /// Pass `--quiet` to installed hook commands.
    pub quiet: bool,
}

const PLUGIN_RAW_BASE: &str = "https://raw.githubusercontent.com/tanlethanh/zedra-plugin/main";
const SKILL_NAMES: &[&str] = &["zedra-start"];
const ZEDRA_HOOK_SOURCE: &str = "zedra-agent-hook";

/// `zedra setup <agent>`: resolve the slug through the actor registry and run
/// that actor's setup flow.
pub async fn run(slug: &str, remove: bool, ctx: SetupCliCtx) -> Result<()> {
    let Some(actor) = super::actor(slug) else {
        bail!(
            "unsupported agent `{slug}`; supported: {}",
            supported_slugs().join(", ")
        );
    };
    if !actor.supports_setup_cli() {
        bail!(
            "{} has no `zedra setup` flow; supported: {}",
            actor.slug(),
            supported_slugs().join(", ")
        );
    }
    let action = if remove {
        SetupAction::Remove
    } else {
        SetupAction::Install
    };
    actor.setup_cli(action, ctx).await
}

/// Bare `zedra setup`: install for every setup-capable actor whose CLI is on
/// PATH.
pub async fn run_all(ctx: SetupCliCtx) -> Result<()> {
    let mut found = false;
    for actor in super::actors() {
        if !actor.supports_setup_cli() || actor.resolved_program().is_none() {
            continue;
        }
        found = true;
        if let Err(error) = actor.setup_cli(SetupAction::Install, ctx).await {
            utils::eprintln_error(format!("{} setup failed: {error}", actor.display_name()));
        }
        ctx.message("");
        ctx.message("");
    }
    if !found {
        ctx.message("No supported agents found on PATH.");
        ctx.message("Run `zedra setup <agent>` to set up a specific agent.");
    }
    Ok(())
}

fn supported_slugs() -> Vec<&'static str> {
    super::actors()
        .iter()
        .filter(|actor| actor.supports_setup_cli())
        .map(|actor| actor.slug())
        .collect()
}

// ---------------------------------------------------------------------------
// Actor-facing setup toolkit
// ---------------------------------------------------------------------------

/// Per-agent data for the shared Claude/Codex marketplace-plugin
/// install/remove script; only these fields differ between the two.
pub(crate) struct PluginSetup<'a> {
    pub display: &'a str,
    pub program: &'a str,
    pub install_args: &'a [&'a str],
    pub uninstall_args: &'a [&'a str],
    pub hooks_path: PathBuf,
    pub events: &'a [(&'a str, Option<&'a str>, u64)],
    pub agent: &'a str,
    /// Product name in the "Start in <…>:" completion line (e.g. "Claude Code").
    pub start_in: &'a str,
    pub start_command: &'a str,
    /// Reload/restart instruction printed after removal.
    pub reload_note: &'a str,
    /// Command suggested after `reload_note`, when the product has one.
    pub reload_command: Option<&'a str>,
}

impl SetupCliCtx {
    /// Marketplace repository for Claude/Codex plugin installs.
    pub(crate) const PLUGIN_REPO: &'static str = "tanlethanh/zedra-plugin";
    /// Marketplace name registered by adding [`Self::PLUGIN_REPO`].
    const PLUGIN_MARKETPLACE: &'static str = "zedra";

    /// Installs the marketplace plugin and merges its command hooks.
    pub(crate) fn install_plugin_and_hooks(&self, spec: &PluginSetup) -> Result<()> {
        self.section(&format!("Setting up {}", spec.display));
        if !self.try_step(
            "marketplace",
            spec.program,
            &["plugin", "marketplace", "add", Self::PLUGIN_REPO],
        )? {
            self.message("Continuing; marketplace may already be configured.");
        }
        self.run_step("plugin", spec.program, spec.install_args)?;
        self.merge_command_hooks(&spec.hooks_path, spec.events, spec.agent)?;
        self.message(&format!(
            "{} setup complete. Start in {}:",
            spec.display, spec.start_in
        ));
        self.suggest_command(spec.start_command);
        Ok(())
    }

    /// Removes the marketplace plugin and its command hooks, then prints the
    /// spec's reload instructions.
    pub(crate) fn remove_plugin_and_hooks(&self, spec: &PluginSetup) -> Result<()> {
        self.message(&format!("Removing Zedra plugin for {}:", spec.display));
        if !self.try_step("plugin", spec.program, spec.uninstall_args)? {
            self.message("Continuing; Zedra plugin may already be removed.");
        }
        if !self.try_step(
            "marketplace",
            spec.program,
            &["plugin", "marketplace", "remove", Self::PLUGIN_MARKETPLACE],
        )? {
            self.message("Continuing; Zedra marketplace may already be removed.");
        }
        self.remove_command_hooks(&spec.hooks_path, spec.agent)?;

        self.message("");
        self.message(&format!("{} setup removed.", spec.display));
        self.message(spec.reload_note);
        if let Some(command) = spec.reload_command {
            self.suggest_command(command);
        }
        Ok(())
    }

    // ---- step output ----

    /// Section heading opening a setup phase.
    pub(crate) fn section(&self, title: &str) {
        utils::println_section(title);
    }

    /// Step label; follow with `detail` lines for its outcome.
    pub(crate) fn step(&self, label: &str) {
        utils::println_step(label);
    }

    /// Dim detail line under the current step (paths written, commands run).
    pub(crate) fn detail(&self, detail: &str) {
        utils::println_dim(detail);
    }

    /// Highlighted command for the user to run next.
    pub(crate) fn suggest_command(&self, command: &str) {
        utils::println_command(command);
    }

    /// Plain stdout line in a setup flow; use `""` for a blank separator.
    pub(crate) fn message(&self, text: &str) {
        println!("{text}");
    }

    // ---- command steps ----

    /// Errors when `program` is not runnable from PATH.
    pub(crate) fn require_command(&self, program: &str) -> Result<()> {
        if !cli_on_path(program) {
            anyhow::bail!("`{program}` was not found in PATH");
        }
        Ok(())
    }

    /// Runs a command as a labeled step; errors on non-zero exit.
    pub(crate) fn run_step(&self, label: &str, program: &str, args: &[&str]) -> Result<()> {
        if !self.run_command_step_status(label, program, args, true)? {
            bail!(
                "`{}` exited with a non-zero status",
                display_command(program, args)
            );
        }

        Ok(())
    }

    /// Runs a command as a labeled step; returns `Ok(false)` on failure
    /// instead of erroring, for steps that may already be satisfied.
    pub(crate) fn try_step(&self, label: &str, program: &str, args: &[&str]) -> Result<bool> {
        self.run_command_step_status(label, program, args, false)
    }

    fn run_command_step_status(
        &self,
        label: &str,
        program: &str,
        args: &[&str],
        show_error_output: bool,
    ) -> Result<bool> {
        self.step(label);

        let command = shell_command_line(program, args);
        let terminal = utils::stdout_is_terminal();

        if terminal {
            print!("{}", utils::dim_text(&command));
            std::io::stdout().flush()?;
        }

        let output = run_command_output(program, args)?;
        let success = output.status.success();

        if terminal {
            print!("\r\x1b[2K");
        }
        if success {
            self.detail(&command);
        } else if show_error_output {
            utils::println_error(&command);
            print_command_error_output(&output);
        } else {
            self.message(&utils::command_text(&command));
        }

        Ok(success)
    }

    // ---- hook command construction ----

    /// Binary embedded in hook scripts: bare `zedra` (survives binary moves)
    /// or the absolute current path under `--full-bin-path`.
    pub(crate) fn hook_binary(&self) -> Result<String> {
        if self.full_bin_path {
            current_zedra_binary()
        } else {
            Ok("zedra".to_string())
        }
    }

    fn hook_command(&self, agent: &str) -> Result<String> {
        let program = if self.full_bin_path {
            crate::utils::shell_arg(&current_zedra_binary()?)
        } else {
            "zedra".to_string()
        };
        let quiet_flag = if self.quiet { " --quiet" } else { "" };
        Ok(format!(
            "{program} agent hook receive --agent {}{quiet_flag}",
            agent_slug(agent),
        ))
    }

    // ---- JSON command-hook files (Claude settings.json / Codex hooks.json) ----

    /// Idempotently upserts Zedra hook entries in a Claude/Codex-style JSON
    /// hooks file, preserving user entries.
    pub(crate) fn merge_command_hooks(
        &self,
        path: &Path,
        events: &[(&str, Option<&str>, u64)],
        agent: &str,
    ) -> Result<()> {
        let mut root = read_json_object(path)?;
        let command = self.hook_command(agent)?;
        let hooks = root
            .as_object_mut()
            .expect("read_json_object returns object")
            .entry("hooks")
            .or_insert_with(|| json!({}));
        let hooks = hooks
            .as_object_mut()
            .ok_or_else(|| anyhow!("{} hooks must be a JSON object", path.display()))?;

        for (event, matcher, timeout) in events {
            let entries = hooks
                .entry((*event).to_string())
                .or_insert_with(|| Value::Array(Vec::new()));
            let entries = entries
                .as_array_mut()
                .ok_or_else(|| anyhow!("{event} hooks must be a JSON array"))?;
            let mut entry = json!({
                "_source": ZEDRA_HOOK_SOURCE,
                "hooks": [{
                    "type": "command",
                    "command": command,
                    "timeout": timeout
                }]
            });
            if let Some(matcher) = matcher {
                entry["matcher"] = Value::String((*matcher).to_string());
                if *event == "PermissionRequest" {
                    entry["hooks"][0]["statusMessage"] =
                        Value::String("Waiting for Zedra approval".to_string());
                }
            }
            if let Some(index) = hook_entry_index(entries, agent) {
                entries[index] = entry;
            } else {
                entries.push(entry);
            }
        }

        write_json_file(path, &root)?;
        self.step("hooks");
        self.detail(&format!(
            "register: {}",
            events
                .iter()
                .map(|(event, _, _)| *event)
                .collect::<Vec<_>>()
                .join(", ")
        ));
        self.detail(&format!("write {}", path.display()));
        Ok(())
    }

    /// Deletes only Zedra-owned entries from a JSON hooks file.
    pub(crate) fn remove_command_hooks(&self, path: &Path, agent: &str) -> Result<()> {
        if !path.exists() {
            return Ok(());
        }
        let mut root = read_json_object(path)?;
        let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) else {
            return Ok(());
        };
        for value in hooks.values_mut() {
            if let Some(entries) = value.as_array_mut() {
                entries.retain(|entry| !is_zedra_hook_entry(entry, agent));
            }
        }
        hooks.retain(|_, value| value.as_array().is_none_or(|entries| !entries.is_empty()));
        write_json_file(path, &root)?;
        self.step("hooks");
        self.detail(&format!("write {}", path.display()));
        Ok(())
    }

    // ---- skills download ----

    /// Downloads Zedra skills from the plugin repo into `skills_dir`.
    pub(crate) async fn install_skills(&self, agent_name: &str, skills_dir: &Path) -> Result<()> {
        self.section(&format!("Setting up {agent_name}"));
        let client = reqwest::Client::new();
        let mut installed = 0usize;
        let mut skipped = Vec::new();

        for skill in SKILL_NAMES {
            let url = format!("{PLUGIN_RAW_BASE}/plugins/zedra/skills/{skill}/SKILL.md");
            let target = skills_dir.join(skill).join("SKILL.md");
            self.step(skill);

            match download_skill(&client, &url).await {
                Ok(contents) => {
                    write_skill_file(&target, &contents)?;
                    installed += 1;
                    self.detail(&format!("write {}", target.display()));
                }
                Err(err) => {
                    skipped.push(format!("{skill}: {err}"));
                    utils::println_error(format!("download {url}"));
                }
            }
        }

        if installed == 0 {
            bail!("failed to install any Zedra skills");
        }
        if !skipped.is_empty() {
            self.message("");
            self.message("Some skills were skipped:");
            for item in skipped {
                self.message(&format!("  {item}"));
            }
        }

        Ok(())
    }

    /// Removes the Zedra skills previously installed under `skills_dir`.
    pub(crate) fn remove_skills(&self, agent_name: &str, skills_dir: &Path) -> Result<()> {
        let mut removed = 0usize;

        self.message(&format!("Removing Zedra skills for {agent_name}:"));

        for skill in SKILL_NAMES {
            let path = skills_dir.join(skill);
            match self.remove_path(&path) {
                Ok(true) => {
                    removed += 1;
                    self.step(skill);
                    self.detail(&format!("remove {}", path.display()));
                }
                Ok(false) => {}
                Err(err) => return Err(err),
            }
        }

        if removed == 0 {
            self.message("No Zedra skills were installed.");
        }

        Ok(())
    }

    /// Removes a file or directory, reporting whether anything existed.
    pub(crate) fn remove_path(&self, path: &Path) -> Result<bool> {
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.is_dir() => {
                fs::remove_dir_all(path)?;
                Ok(true)
            }
            Ok(_) => {
                fs::remove_file(path)?;
                Ok(true)
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(err) => Err(err).with_context(|| format!("remove {}", path.display())),
        }
    }

    /// User home; errors when `$HOME` is unset.
    pub(crate) fn home_dir(&self) -> Result<PathBuf> {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|path| !path.as_os_str().is_empty())
            .ok_or_else(|| anyhow!("HOME is not set"))
    }
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

fn cli_on_path(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

fn run_command_output(program: &str, args: &[&str]) -> Result<Output> {
    Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("failed to run `{}`", display_command(program, args)))
}

fn print_command_error_output(output: &Output) {
    print_command_stream("stdout", &output.stdout);
    print_command_stream("stderr", &output.stderr);
    if output.stdout.is_empty() && output.stderr.is_empty() {
        utils::eprintln_error(format!("status: {}", output.status));
    }
}

fn print_command_stream(label: &str, bytes: &[u8]) {
    if bytes.is_empty() {
        return;
    }

    eprintln!("{label}:");
    eprint!("{}", String::from_utf8_lossy(bytes));
    if !bytes.ends_with(b"\n") {
        eprintln!();
    }
}

fn display_command(program: &str, args: &[&str]) -> String {
    std::iter::once(program)
        .chain(args.iter().copied())
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_command_line(program: &str, args: &[&str]) -> String {
    format!("$ {}", display_command(program, args))
}

fn current_zedra_binary() -> Result<String> {
    Ok(std::env::current_exe()
        .context("resolve current zedra binary")?
        .display()
        .to_string())
}

fn read_json_object(path: &Path) -> Result<Value> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let contents = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let trimmed = contents.trim();
    if trimmed.is_empty() {
        return Ok(json!({}));
    }
    let value: Value =
        serde_json::from_str(trimmed).with_context(|| format!("parse {}", path.display()))?;
    if value.is_object() {
        Ok(value)
    } else {
        bail!("{} must contain a JSON object", path.display());
    }
}

fn write_json_file(path: &Path, value: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

fn hook_entry_index(entries: &[Value], agent: &str) -> Option<usize> {
    entries
        .iter()
        .position(|entry| is_zedra_hook_entry(entry, agent))
}

fn is_zedra_hook_entry(entry: &Value, agent: &str) -> bool {
    // Match by _source (new format) OR by command pattern (old/plugin format).
    // Either is sufficient so re-runs never duplicate entries written without _source.
    let has_source = entry.get("_source").and_then(Value::as_str) == Some(ZEDRA_HOOK_SOURCE);
    let has_command = entry
        .get("hooks")
        .and_then(Value::as_array)
        .is_some_and(|hooks| {
            hooks.iter().any(|hook| {
                hook.get("command")
                    .and_then(Value::as_str)
                    .is_some_and(|command| command_uses_agent(command, agent))
            })
        });
    has_source || has_command
}

fn command_uses_agent(command: &str, agent: &str) -> bool {
    let Some(raw_agent) = command_agent(command) else {
        return false;
    };
    command.contains("agent hook receive --agent") && agent_slug(raw_agent) == agent_slug(agent)
}

fn command_agent(command: &str) -> Option<&str> {
    let mut tokens = command.split_whitespace();
    while let Some(token) = tokens.next() {
        if token == "--agent" {
            return tokens.next();
        }
    }
    None
}

/// Normalizes legacy `Claude`/`Codex` spellings from hook commands written by
/// old releases; current writes always use the registry slug.
fn agent_slug(agent: &str) -> &str {
    match agent {
        "Claude" => "claude",
        "Codex" => "codex",
        _ => agent,
    }
}

async fn download_skill(client: &reqwest::Client, url: &str) -> Result<String> {
    let response = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("download failed: {url}"))?;
    let response = response
        .error_for_status()
        .with_context(|| format!("download failed: {url}"))?;
    let contents = response.text().await?;

    if !contents.starts_with("---\n") {
        bail!("downloaded file is not a Codex skill");
    }

    Ok(contents)
}

fn write_skill_file(path: &Path, contents: &str) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("skill path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent)?;

    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    fs::write(&tmp, contents)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(full_bin_path: bool, quiet: bool) -> SetupCliCtx {
        SetupCliCtx {
            full_bin_path,
            quiet,
        }
    }

    #[test]
    fn command_hook_merge_is_idempotent_and_preserves_existing_hooks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hooks.json");
        fs::write(
            &path,
            serde_json::to_vec_pretty(&json!({
                "hooks": {
                    "Stop": [{
                        "hooks": [{
                            "type": "command",
                            "command": "/usr/bin/true"
                        }]
                    }]
                }
            }))
            .unwrap(),
        )
        .unwrap();

        let setup = ctx(false, true);
        setup
            .merge_command_hooks(
                &path,
                &[
                    ("Stop", None, 2),
                    ("PermissionRequest", Some("*"), 30),
                    ("PostToolUse", Some("*"), 2),
                ],
                "Codex",
            )
            .unwrap();
        let mut root = read_json_object(&path).unwrap();
        root["hooks"]["PermissionRequest"][0]["hooks"][0]["command"] =
            Value::String("/tmp/old-zedra agent hook receive --agent Codex".to_string());
        write_json_file(&path, &root).unwrap();
        setup
            .merge_command_hooks(
                &path,
                &[
                    ("Stop", None, 2),
                    ("PermissionRequest", Some("*"), 30),
                    ("PostToolUse", Some("*"), 2),
                ],
                "Codex",
            )
            .unwrap();

        let root = read_json_object(&path).unwrap();
        let stop = root["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2);
        assert_eq!(
            stop.iter()
                .filter(|entry| is_zedra_hook_entry(entry, "Codex"))
                .count(),
            1
        );
        assert!(root["hooks"]["PermissionRequest"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .contains(" agent hook receive --agent codex"));
        assert_eq!(
            root["hooks"]["PermissionRequest"][0]["hooks"][0]["command"],
            "zedra agent hook receive --agent codex --quiet"
        );
        assert_eq!(
            root["hooks"]["PermissionRequest"][0]["hooks"][0]["statusMessage"],
            "Waiting for Zedra approval"
        );
        assert_eq!(
            root["hooks"]["PostToolUse"][0]["hooks"][0]["command"],
            "zedra agent hook receive --agent codex --quiet"
        );
    }

    #[test]
    fn command_hook_remove_only_deletes_zedra_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let setup = ctx(false, true);
        setup
            .merge_command_hooks(&path, &[("Stop", None, 2)], "Claude")
            .unwrap();
        let mut root = read_json_object(&path).unwrap();
        root["hooks"]["Stop"].as_array_mut().unwrap().push(json!({
            "hooks": [{
                "type": "command",
                "command": "/usr/bin/true"
            }]
        }));
        write_json_file(&path, &root).unwrap();

        setup.remove_command_hooks(&path, "Claude").unwrap();

        let root = read_json_object(&path).unwrap();
        let stop = root["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 1);
        assert_eq!(stop[0]["hooks"][0]["command"], "/usr/bin/true");
    }

    #[test]
    fn hook_command_uses_lowercase_agent_slugs() {
        for (agent, slug) in [("Claude", "claude"), ("codex", "codex")] {
            assert_eq!(
                ctx(false, true).hook_command(agent).unwrap(),
                format!("zedra agent hook receive --agent {slug} --quiet")
            );
            assert_eq!(
                ctx(false, false).hook_command(agent).unwrap(),
                format!("zedra agent hook receive --agent {slug}")
            );
        }
    }

    #[test]
    fn hook_command_can_use_current_binary_path() {
        let command = ctx(true, true).hook_command("Codex").unwrap();
        assert!(command.ends_with(" agent hook receive --agent codex --quiet"));
        assert_ne!(command, "zedra agent hook receive --agent codex --quiet");
    }

    #[test]
    fn every_setup_capable_actor_is_discoverable() {
        let slugs = supported_slugs();
        assert!(!slugs.is_empty(), "no setup-capable actors registered");
        // Every advertised slug must resolve through the same `actor()` lookup
        // `zedra setup <slug>` uses, back to a setup-capable actor.
        for slug in slugs {
            let actor = super::super::actor(slug)
                .unwrap_or_else(|| panic!("slug `{slug}` does not resolve to an actor"));
            assert!(actor.supports_setup_cli(), "`{slug}` lost its setup flow");
            assert_eq!(actor.slug(), slug);
        }
    }
}
