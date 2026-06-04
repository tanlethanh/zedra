use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Subcommand};
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use zedra_host::utils;

const PLUGIN_REPO: &str = "tanlethanh/zedra-plugin";
const CLAUDE_MARKETPLACE: &str = "zedra";
const CLAUDE_PLUGIN: &str = "zedra@zedra";
const PLUGIN_RAW_BASE: &str = "https://raw.githubusercontent.com/tanlethanh/zedra-plugin/main";
const SKILL_NAMES: &[&str] = &[
    "zedra-start",
    "zedra-status",
    "zedra-stop",
    "zedra-terminal",
];
const ZEDRA_HOOK_SOURCE: &str = "zedra-agent-hook";
const OPENCODE_HOOK_PLUGIN: &str = "zedra-agent-hooks.mjs";

#[derive(Clone, Copy, Debug, Subcommand)]
pub enum SetupAgent {
    /// Manage the Claude Code plugin from the Zedra marketplace
    Claude {
        #[command(flatten)]
        action: SetupActionArgs,
    },
    /// Manage Codex skills
    Codex {
        #[command(flatten)]
        action: SetupActionArgs,
    },
    /// Manage OpenCode skills in the global OpenCode skill directory
    #[command(name = "opencode", alias = "open-code")]
    OpenCode {
        #[command(flatten)]
        action: SetupActionArgs,
    },
}

pub async fn run(agent: SetupAgent, assume_yes: bool) -> Result<()> {
    match agent {
        SetupAgent::Claude { action } => setup_claude(action.into()),
        SetupAgent::Codex { action } => setup_codex(action.into(), assume_yes).await,
        SetupAgent::OpenCode { action } => setup_opencode(action.into()).await,
    }
}

#[derive(Clone, Copy, Debug, Args)]
pub struct SetupActionArgs {
    /// Remove this agent's Zedra setup
    #[arg(long)]
    remove: bool,
}

impl From<SetupActionArgs> for SetupAction {
    fn from(args: SetupActionArgs) -> Self {
        Self::from_remove_flag(args.remove)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SetupAction {
    Install,
    Remove,
}

impl SetupAction {
    fn from_remove_flag(remove: bool) -> Self {
        if remove {
            Self::Remove
        } else {
            Self::Install
        }
    }
}

fn setup_claude(action: SetupAction) -> Result<()> {
    require_command("claude")?;

    match action {
        SetupAction::Install => setup_claude_install(),
        SetupAction::Remove => setup_claude_remove(),
    }
}

fn setup_claude_install() -> Result<()> {
    println!("Installing Zedra plugin for Claude:");
    if !run_optional_command_step(
        "marketplace",
        "claude",
        &["plugin", "marketplace", "add", PLUGIN_REPO],
    )? {
        println!("Continuing; marketplace may already be configured.");
    }
    run_command_step(
        "plugin",
        "claude",
        &["plugin", "install", "--scope", "user", CLAUDE_PLUGIN],
    )?;
    install_claude_hooks()?;

    println!();
    println!("Claude setup complete.");
    println!("In Claude Code, reload plugins, then start Zedra:");
    print_suggested_command("/reload-plugins");
    print_suggested_command("/zedra-start");
    Ok(())
}

fn setup_claude_remove() -> Result<()> {
    println!("Removing Zedra plugin for Claude:");
    if !run_optional_command_step(
        "plugin",
        "claude",
        &["plugin", "uninstall", "--scope", "user", CLAUDE_PLUGIN],
    )? {
        println!("Continuing; Zedra plugin may already be removed.");
    }
    if !run_optional_command_step(
        "marketplace",
        "claude",
        &["plugin", "marketplace", "remove", CLAUDE_MARKETPLACE],
    )? {
        println!("Continuing; Zedra marketplace may already be removed.");
    }
    remove_claude_hooks()?;

    println!();
    println!("Claude setup removed.");
    println!("In Claude Code, reload plugins to apply the change:");
    print_suggested_command("/reload-plugins");
    Ok(())
}

async fn setup_codex(action: SetupAction, _assume_yes: bool) -> Result<()> {
    match action {
        SetupAction::Install => setup_codex_install().await,
        SetupAction::Remove => setup_codex_remove(),
    }
}

async fn setup_codex_install() -> Result<()> {
    let skills_dir = codex_skills_dir()?;
    install_skills_from_raw("Codex", &skills_dir, "Installing").await?;
    install_codex_hooks()?;

    println!();
    println!("Codex setup complete.");
    println!("In Codex, reload skills if this session is already open, then run:");
    print_suggested_command("$zedra-start");
    Ok(())
}

fn setup_codex_remove() -> Result<()> {
    let skills_dir = codex_skills_dir()?;
    remove_installed_skills("Codex", &skills_dir)?;
    remove_codex_hooks()?;

    println!();
    println!("Codex setup removed.");
    println!("Restart Codex or reload skills to apply the change.");
    Ok(())
}

async fn setup_opencode(action: SetupAction) -> Result<()> {
    match action {
        SetupAction::Install => setup_opencode_install().await,
        SetupAction::Remove => setup_opencode_remove(),
    }
}

async fn setup_opencode_install() -> Result<()> {
    let skills_dir = opencode_skills_dir()?;
    install_skills_from_raw("OpenCode", &skills_dir, "Installing").await?;
    install_opencode_hooks()?;

    println!();
    println!("OpenCode setup complete.");
    println!("In OpenCode, run:");
    print_suggested_command("/zedra-start");
    Ok(())
}

fn setup_opencode_remove() -> Result<()> {
    let skills_dir = opencode_skills_dir()?;
    remove_installed_skills("OpenCode", &skills_dir)?;
    remove_opencode_hooks()?;

    println!();
    println!("OpenCode setup removed.");
    println!("Restart OpenCode or reload skills to apply the change.");
    Ok(())
}

fn print_suggested_command(command: &str) {
    utils::println_command(command);
}

fn print_step_label(label: &str) {
    utils::println_step(label);
}

fn print_detail(detail: &str) {
    utils::println_note(detail);
}

fn print_success_detail(detail: &str) {
    utils::println_success(detail);
}

fn print_error_detail(detail: &str) {
    utils::println_error(detail);
}

fn install_claude_hooks() -> Result<()> {
    let path = home_dir()?.join(".claude").join("settings.json");
    merge_command_hooks(
        &path,
        &[
            ("UserPromptSubmit", None, 2),
            ("PermissionRequest", Some("*"), 2),
            ("Stop", None, 2),
            ("TaskCompleted", None, 2),
            ("SessionEnd", None, 2),
        ],
        "Claude",
    )
}

fn remove_claude_hooks() -> Result<()> {
    remove_command_hooks(&home_dir()?.join(".claude").join("settings.json"), "Claude")
}

fn install_codex_hooks() -> Result<()> {
    let path = home_dir()?.join(".codex").join("hooks.json");
    merge_command_hooks(
        &path,
        &[
            ("UserPromptSubmit", None, 2),
            ("PermissionRequest", Some("*"), 30),
            ("Stop", None, 2),
        ],
        "Codex",
    )
}

fn remove_codex_hooks() -> Result<()> {
    remove_command_hooks(&home_dir()?.join(".codex").join("hooks.json"), "Codex")
}

fn merge_command_hooks(
    path: &Path,
    events: &[(&str, Option<&str>, u64)],
    agent: &str,
) -> Result<()> {
    let mut root = read_json_object(path)?;
    let command = hook_command(agent)?;
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
        if hook_entry_index(entries, agent).is_some() {
            continue;
        }
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
        entries.push(entry);
    }

    write_json_file(path, &root)?;
    print_step_label("hooks");
    print_success_detail(&format!("write {}", path.display()));
    Ok(())
}

fn remove_command_hooks(path: &Path, agent: &str) -> Result<()> {
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
    print_step_label("hooks");
    print_success_detail(&format!("write {}", path.display()));
    Ok(())
}

fn install_opencode_hooks() -> Result<()> {
    let dir = home_dir()?.join(".config").join("opencode");
    install_opencode_hooks_in_dir(&dir, &hook_binary()?)
}

fn install_opencode_hooks_in_dir(dir: &Path, binary: &str) -> Result<()> {
    let plugin_path = dir.join(OPENCODE_HOOK_PLUGIN);
    fs::create_dir_all(&dir)?;
    fs::write(&plugin_path, opencode_hook_plugin(binary)?)?;
    let config_path = dir.join("opencode.jsonc");
    let mut root = read_json_object(&config_path)?;
    let plugins = root
        .as_object_mut()
        .expect("read_json_object returns object")
        .entry("plugin")
        .or_insert_with(|| Value::Array(Vec::new()));
    let plugins = plugins
        .as_array_mut()
        .ok_or_else(|| anyhow!("{} plugin must be a JSON array", config_path.display()))?;
    let plugin_entry = plugin_path.display().to_string();
    if !plugins
        .iter()
        .any(|value| value.as_str() == Some(&plugin_entry))
    {
        plugins.push(Value::String(plugin_entry));
    }
    write_json_file(&config_path, &root)?;
    print_step_label("hooks");
    print_success_detail(&format!("write {}", plugin_path.display()));
    Ok(())
}

fn remove_opencode_hooks() -> Result<()> {
    let dir = home_dir()?.join(".config").join("opencode");
    remove_opencode_hooks_in_dir(&dir)
}

fn remove_opencode_hooks_in_dir(dir: &Path) -> Result<()> {
    let plugin_path = dir.join(OPENCODE_HOOK_PLUGIN);
    let config_path = dir.join("opencode.jsonc");
    if config_path.exists() {
        let mut root = read_json_object(&config_path)?;
        if let Some(plugins) = root.get_mut("plugin").and_then(Value::as_array_mut) {
            let plugin_entry = plugin_path.display().to_string();
            plugins.retain(|value| value.as_str() != Some(&plugin_entry));
        }
        write_json_file(&config_path, &root)?;
    }
    if remove_skill_path(&plugin_path)? {
        print_step_label("hooks");
        print_success_detail(&format!("remove {}", plugin_path.display()));
    }
    Ok(())
}

fn hook_command(agent: &str) -> Result<String> {
    Ok(format!(
        "{} agent-hook {}",
        shell_quote(&hook_binary()?),
        agent
    ))
}

fn hook_binary() -> Result<String> {
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
    entry.get("_source").and_then(Value::as_str) == Some(ZEDRA_HOOK_SOURCE)
        && entry
            .get("hooks")
            .and_then(Value::as_array)
            .is_some_and(|hooks| {
                hooks.iter().any(|hook| {
                    hook.get("command")
                        .and_then(Value::as_str)
                        .is_some_and(|command| command.contains(&format!(" agent-hook {agent}")))
                })
            })
}

fn opencode_hook_plugin(binary: &str) -> Result<String> {
    let binary = serde_json::to_string(binary)?;
    Ok(format!(
        r#"import {{ spawnSync }} from "node:child_process";

const zedra = {binary};

function send(event, payload = {{}}) {{
  spawnSync(zedra, ["agent-hook", "opencode"], {{
    input: JSON.stringify({{ event, ...payload }}),
    stdio: ["pipe", "ignore", "ignore"],
    timeout: 2000,
  }});
}}

export default async function zedraAgentHooks() {{
  return {{
    event: async (input) => send(input.event?.type ?? "event", input),
    "chat.message": async (input) => send("chat.message", input),
    "permission.ask": async (input) => send("permission.ask", input),
    "tool.execute.after": async (input) => {{
      if (String(input.tool ?? "").toLowerCase().includes("selection")) {{
        send("selection", input);
      }}
    }},
  }};
}}
"#
    ))
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

async fn install_skills_from_raw(agent_name: &str, skills_dir: &Path, verb: &str) -> Result<()> {
    let client = reqwest::Client::new();
    let mut installed = 0usize;
    let mut skipped = Vec::new();

    println!("{verb} Zedra skills for {agent_name}:");

    for skill in SKILL_NAMES {
        let url = format!("{PLUGIN_RAW_BASE}/plugins/zedra/skills/{skill}/SKILL.md");
        let target = skills_dir.join(skill).join("SKILL.md");
        print_step_label(skill);

        match download_skill(&client, &url).await {
            Ok(contents) => {
                write_skill_file(&target, &contents)?;
                installed += 1;
                print_success_detail(&format!("write {}", target.display()));
            }
            Err(err) => {
                skipped.push(format!("{skill}: {err}"));
                print_error_detail(&format!("download {url}"));
            }
        }
    }

    if installed == 0 {
        bail!("failed to install any Zedra skills");
    }
    if !skipped.is_empty() {
        println!();
        println!("Some skills were skipped:");
        for item in skipped {
            println!("  {item}");
        }
    }

    Ok(())
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

fn remove_installed_skills(agent_name: &str, skills_dir: &Path) -> Result<()> {
    let mut removed = 0usize;

    println!("Removing Zedra skills for {agent_name}:");

    for skill in SKILL_NAMES {
        let path = skills_dir.join(skill);
        match remove_skill_path(&path) {
            Ok(true) => {
                removed += 1;
                print_step_label(skill);
                print_success_detail(&format!("remove {}", path.display()));
            }
            Ok(false) => {}
            Err(err) => return Err(err),
        }
    }

    if removed == 0 {
        println!("No Zedra skills were installed.");
    }

    Ok(())
}

fn remove_skill_path(path: &Path) -> Result<bool> {
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

fn codex_skills_dir() -> Result<PathBuf> {
    Ok(home_dir()?.join(".agents").join("skills"))
}

fn opencode_skills_dir() -> Result<PathBuf> {
    Ok(home_dir()?.join(".config").join("opencode").join("skills"))
}

fn home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| anyhow!("HOME is not set"))
}

fn require_command(program: &str) -> Result<()> {
    Command::new(program)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .with_context(|| format!("`{program}` was not found in PATH"))?;
    Ok(())
}

fn run_command_step(label: &str, program: &str, args: &[&str]) -> Result<()> {
    if !run_command_step_status(label, program, args, true)? {
        bail!(
            "`{}` exited with a non-zero status",
            display_command(program, args)
        );
    }

    Ok(())
}

fn run_optional_command_step(label: &str, program: &str, args: &[&str]) -> Result<bool> {
    run_command_step_status(label, program, args, false)
}

fn run_command_step_status(
    label: &str,
    program: &str,
    args: &[&str],
    show_error_output: bool,
) -> Result<bool> {
    print_step_label(label);

    let command = shell_command_line(program, args);
    let terminal = utils::stdout_is_terminal();

    if terminal {
        print!("{}", utils::command_text(&command));
        std::io::stdout().flush()?;
    }

    let output = run_command_output(program, args)?;
    let success = output.status.success();

    if terminal {
        print!("\r\x1b[2K");
    }
    if success {
        print_success_detail(&command);
    } else if show_error_output {
        print_error_detail(&command);
        print_command_error_output(&output);
    } else {
        print_detail(&command);
    }

    Ok(success)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_action_from_remove_flag() {
        assert_eq!(SetupAction::from_remove_flag(false), SetupAction::Install);
        assert_eq!(SetupAction::from_remove_flag(true), SetupAction::Remove);
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

        merge_command_hooks(
            &path,
            &[("Stop", None, 2), ("PermissionRequest", Some("*"), 30)],
            "Codex",
        )
        .unwrap();
        merge_command_hooks(
            &path,
            &[("Stop", None, 2), ("PermissionRequest", Some("*"), 30)],
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
            .contains(" agent-hook Codex"));
        assert_eq!(
            root["hooks"]["PermissionRequest"][0]["hooks"][0]["statusMessage"],
            "Waiting for Zedra approval"
        );
    }

    #[test]
    fn command_hook_remove_only_deletes_zedra_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        merge_command_hooks(&path, &[("Stop", None, 2)], "Claude").unwrap();
        let mut root = read_json_object(&path).unwrap();
        root["hooks"]["Stop"].as_array_mut().unwrap().push(json!({
            "hooks": [{
                "type": "command",
                "command": "/usr/bin/true"
            }]
        }));
        write_json_file(&path, &root).unwrap();

        remove_command_hooks(&path, "Claude").unwrap();

        let root = read_json_object(&path).unwrap();
        let stop = root["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 1);
        assert_eq!(stop[0]["hooks"][0]["command"], "/usr/bin/true");
    }

    #[test]
    fn opencode_hook_install_and_remove_updates_plugin_config() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("opencode.jsonc");
        fs::write(
            &config_path,
            serde_json::to_vec_pretty(&json!({
                "plugin": ["existing-plugin"]
            }))
            .unwrap(),
        )
        .unwrap();

        install_opencode_hooks_in_dir(dir.path(), "/tmp/zedra").unwrap();
        install_opencode_hooks_in_dir(dir.path(), "/tmp/zedra").unwrap();

        let plugin_path = dir.path().join(OPENCODE_HOOK_PLUGIN);
        let plugin = fs::read_to_string(&plugin_path).unwrap();
        assert!(plugin.contains(r#"spawnSync(zedra, ["agent-hook", "opencode"]"#));

        let root = read_json_object(&config_path).unwrap();
        let plugins = root["plugin"].as_array().unwrap();
        assert_eq!(
            plugins
                .iter()
                .filter(|value| value.as_str() == Some(plugin_path.to_str().unwrap()))
                .count(),
            1
        );
        assert!(plugins.iter().any(|value| value == "existing-plugin"));

        remove_opencode_hooks_in_dir(dir.path()).unwrap();

        let root = read_json_object(&config_path).unwrap();
        let plugins = root["plugin"].as_array().unwrap();
        assert!(plugins.iter().any(|value| value == "existing-plugin"));
        assert!(plugins
            .iter()
            .all(|value| value.as_str() != Some(plugin_path.to_str().unwrap())));
        assert!(!plugin_path.exists());
    }
}
