use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Subcommand};
use serde_json::{Value, json};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use zedra_host::utils;

const PLUGIN_REPO: &str = "tanlethanh/zedra-plugin";
const CLAUDE_MARKETPLACE: &str = "zedra";
const CLAUDE_PLUGIN: &str = "zedra@zedra";
const CODEX_MARKETPLACE: &str = "zedra";
const CODEX_PLUGIN: &str = "zedra@zedra";
const PLUGIN_RAW_BASE: &str = "https://raw.githubusercontent.com/tanlethanh/zedra-plugin/main";
const SKILL_NAMES: &[&str] = &["zedra-start"];
const ZEDRA_HOOK_SOURCE: &str = "zedra-agent-hook";
const OPENCODE_HOOK_PLUGIN: &str = "zedra-agent-hooks.js";
const PI_HOOK_EXTENSION: &str = "zedra-agent-hooks.ts";

#[derive(Clone, Copy, Debug, Subcommand)]
pub enum SetupAgent {
    /// Claude Code — installs the Zedra plugin and lifecycle hooks
    Claude {
        #[command(flatten)]
        action: SetupActionArgs,
    },
    /// Codex — installs the Zedra plugin and lifecycle hooks
    Codex {
        #[command(flatten)]
        action: SetupActionArgs,
    },
    /// OpenCode — installs Zedra skills and the hook plugin
    #[command(name = "opencode", alias = "open-code")]
    OpenCode {
        #[command(flatten)]
        action: SetupActionArgs,
    },
    /// Pi — installs the Zedra lifecycle-hook extension
    Pi {
        #[command(flatten)]
        action: SetupActionArgs,
    },
    /// Hermes — installs the Zedra lifecycle-hook script and patches config.yaml
    Hermes {
        #[command(flatten)]
        action: SetupActionArgs,
    },
}

pub async fn run_all(_assume_yes: bool, full_bin_path: bool, no_quiet: bool) -> Result<()> {
    let quiet = !no_quiet;
    let agents: &[(&str, &str)] = &[
        ("claude", "Claude"),
        ("codex", "Codex"),
        ("opencode", "OpenCode"),
        ("pi", "Pi"),
        ("hermes", "Hermes"),
    ];
    let mut found = false;
    for (program, name) in agents {
        if !cli_on_path(program) {
            continue;
        }
        found = true;
        let result = match *name {
            "Claude" => setup_claude_install(full_bin_path, quiet),
            "Codex" => setup_codex_install(full_bin_path, quiet).await,
            "OpenCode" => setup_opencode_install(full_bin_path, quiet).await,
            "Pi" => setup_pi_install(full_bin_path),
            "Hermes" => setup_hermes_install(full_bin_path),
            _ => unreachable!(),
        };
        if let Err(e) = result {
            utils::eprintln_error(format!("{name} setup failed: {e}"));
        }
        println!();
        println!();
    }
    if !found {
        println!("No supported agents found on PATH.");
        println!("Run `zedra setup <agent>` to set up a specific agent.");
    }
    Ok(())
}

pub async fn run(
    agent: SetupAgent,
    assume_yes: bool,
    full_bin_path: bool,
    no_quiet: bool,
) -> Result<()> {
    let quiet = !no_quiet;
    match agent {
        SetupAgent::Claude { action } => setup_claude(action.into(), full_bin_path, quiet),
        SetupAgent::Codex { action } => {
            setup_codex(action.into(), assume_yes, full_bin_path, quiet).await
        }
        SetupAgent::OpenCode { action } => {
            setup_opencode(action.into(), full_bin_path, quiet).await
        }
        SetupAgent::Pi { action } => setup_pi(action.into(), full_bin_path),
        SetupAgent::Hermes { action } => setup_hermes(action.into(), full_bin_path),
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
        if remove { Self::Remove } else { Self::Install }
    }
}

fn setup_claude(action: SetupAction, full_bin_path: bool, quiet: bool) -> Result<()> {
    require_command("claude")?;

    match action {
        SetupAction::Install => setup_claude_install(full_bin_path, quiet),
        SetupAction::Remove => setup_claude_remove(),
    }
}

fn setup_claude_install(full_bin_path: bool, quiet: bool) -> Result<()> {
    utils::println_section("Setting up Claude");
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
    install_claude_hooks(full_bin_path, quiet)?;
    println!("Claude setup complete. Start in Claude Code:");
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

async fn setup_codex(
    action: SetupAction,
    _assume_yes: bool,
    full_bin_path: bool,
    quiet: bool,
) -> Result<()> {
    match action {
        SetupAction::Install => setup_codex_install(full_bin_path, quiet).await,
        SetupAction::Remove => setup_codex_remove(),
    }
}

async fn setup_codex_install(full_bin_path: bool, quiet: bool) -> Result<()> {
    require_command("codex")?;
    utils::println_section("Setting up Codex");
    if !run_optional_command_step(
        "marketplace",
        "codex",
        &["plugin", "marketplace", "add", PLUGIN_REPO],
    )? {
        println!("Continuing; marketplace may already be configured.");
    }
    run_command_step("plugin", "codex", &["plugin", "add", CODEX_PLUGIN])?;
    install_codex_hooks(full_bin_path, quiet)?;
    println!("Codex setup complete. Start in Codex:");
    print_suggested_command("$zedra-start");
    Ok(())
}

fn setup_codex_remove() -> Result<()> {
    require_command("codex")?;
    println!("Removing Zedra plugin for Codex:");
    if !run_optional_command_step("plugin", "codex", &["plugin", "remove", CODEX_PLUGIN])? {
        println!("Continuing; Zedra plugin may already be removed.");
    }
    if !run_optional_command_step(
        "marketplace",
        "codex",
        &["plugin", "marketplace", "remove", CODEX_MARKETPLACE],
    )? {
        println!("Continuing; Zedra marketplace may already be removed.");
    }
    remove_codex_hooks()?;

    println!();
    println!("Codex setup removed.");
    println!("Restart Codex or reload skills to apply the change.");
    Ok(())
}

async fn setup_opencode(action: SetupAction, full_bin_path: bool, quiet: bool) -> Result<()> {
    match action {
        SetupAction::Install => setup_opencode_install(full_bin_path, quiet).await,
        SetupAction::Remove => setup_opencode_remove(),
    }
}

async fn setup_opencode_install(full_bin_path: bool, quiet: bool) -> Result<()> {
    let skills_dir = opencode_skills_dir()?;
    install_skills_from_raw("OpenCode", &skills_dir).await?;
    install_opencode_hooks(full_bin_path, quiet)?;
    println!("OpenCode setup complete. Start in OpenCode:");
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

fn setup_pi(action: SetupAction, full_bin_path: bool) -> Result<()> {
    match action {
        SetupAction::Install => setup_pi_install(full_bin_path),
        SetupAction::Remove => setup_pi_remove(),
    }
}

fn setup_pi_install(full_bin_path: bool) -> Result<()> {
    utils::println_section("Setting up Pi");
    install_pi_hooks(full_bin_path)?;
    println!("pi setup complete.");
    Ok(())
}

fn setup_pi_remove() -> Result<()> {
    println!("Removing Zedra lifecycle-hook extension for pi:");
    remove_pi_hooks()?;

    println!();
    println!("pi setup removed.");
    println!("Restart any running pi session to apply the change.");
    Ok(())
}

fn setup_hermes(action: SetupAction, full_bin_path: bool) -> Result<()> {
    match action {
        SetupAction::Install => setup_hermes_install(full_bin_path),
        SetupAction::Remove => setup_hermes_remove(),
    }
}

fn setup_hermes_install(full_bin_path: bool) -> Result<()> {
    utils::println_section("Setting up Hermes");
    install_hermes_hooks(full_bin_path)?;
    println!("Hermes setup complete.");
    Ok(())
}

fn setup_hermes_remove() -> Result<()> {
    println!("Removing Zedra lifecycle-hook script for Hermes:");
    remove_hermes_hooks()?;

    println!();
    println!("Hermes setup removed.");
    println!("Restart any running Hermes session to apply the change.");
    Ok(())
}

fn hermes_home() -> Result<PathBuf> {
    Ok(std::env::var_os("HERMES_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().unwrap_or_default().join(".hermes")))
}

fn hermes_hook_script_path() -> Result<PathBuf> {
    Ok(hermes_home()?.join("agent-hooks").join(HERMES_HOOK_SCRIPT))
}

const HERMES_HOOK_SCRIPT: &str = "zedra-agent-hooks.sh";

const HERMES_HOOK_EVENTS: &[&str] = &[
    "on_session_start",
    "pre_approval_request",
    "post_approval_response",
    "post_llm_call",
    "on_session_end",
];

fn install_hermes_hooks(full_bin_path: bool) -> Result<()> {
    let script_path = hermes_hook_script_path()?;
    let binary = hook_binary(full_bin_path)?;
    let script = hermes_hook_script(&binary);
    if let Some(parent) = script_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let script_changed =
        fs::read_to_string(&script_path).map_or(true, |existing| existing != script);
    if script_changed {
        fs::write(&script_path, &script)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms)?;
    }
    print_step_label("hooks");
    print_success_detail(&format!(
        "{} {}",
        if script_changed { "write" } else { "keep" },
        script_path.display()
    ));

    let config_path = hermes_home()?.join("config.yaml");
    let existing = fs::read_to_string(&config_path).unwrap_or_default();
    let patched = patch_hermes_config_hooks(&existing, &script_path);
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let config_changed = patched != existing;
    if config_changed {
        fs::write(&config_path, &patched)?;
    }
    print_step_label("config");
    print_success_detail(&format!(
        "{} {}",
        if config_changed { "write" } else { "keep" },
        config_path.display()
    ));

    Ok(())
}

fn remove_hermes_hooks() -> Result<()> {
    let script_path = hermes_hook_script_path()?;
    if remove_skill_path(&script_path)? {
        print_step_label("hooks");
        print_success_detail(&format!("remove {}", script_path.display()));
    }

    let config_path = hermes_home()?.join("config.yaml");
    if config_path.is_file() {
        let existing = fs::read_to_string(&config_path)?;
        let cleaned = remove_hermes_zedra_hooks(&existing);
        if cleaned != existing {
            fs::write(&config_path, &cleaned)?;
            print_step_label("config");
            print_success_detail(&format!("write {}", config_path.display()));
        }
    }
    Ok(())
}

fn hermes_hook_script(binary: &str) -> String {
    format!(
        r#"#!/bin/sh
# zedra-agent-hook (Hermes lifecycle hook)
# No-op outside a Zedra terminal (ZEDRA_TERMINAL_ID not set by the shell).
[ -z "${{ZEDRA_TERMINAL_ID:-}}" ] && exit 0
CLI="${{ZEDRA_CLI:-}}"
[ -n "$CLI" ] || CLI={binary}
[ -x "$CLI" ] || CLI="zedra"
exec "$CLI" agent hook receive --agent hermes --quiet
"#,
        binary = shell_quote(binary),
    )
}

/// Idempotently patches the `hooks:` block in `~/.hermes/config.yaml` to
/// include Zedra hook entries. Preserves all non-Zedra event keys. On
/// re-run, removes old Zedra event keys and re-inserts fresh entries.
fn patch_hermes_config_hooks(config: &str, script_path: &Path) -> String {
    let script = script_path.display().to_string();
    let lines: Vec<&str> = config.lines().collect();
    let trailing_newline = config.ends_with('\n');

    let hooks_idx = lines.iter().position(|l| is_hooks_key_line(l));

    let Some(hooks_idx) = hooks_idx else {
        let mut out = config.to_string();
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("hooks:\n");
        out.push_str(&zedra_hooks_entries(&script));
        return out;
    };

    let inline_empty = {
        let t = lines[hooks_idx].trim();
        t == "hooks: {}" || t == "hooks:{}"
    };

    let hooks_block_end = lines[hooks_idx + 1..]
        .iter()
        .position(|l| {
            !l.is_empty() && !l.starts_with(' ') && !l.starts_with('\t') && !l.starts_with('#')
        })
        .map(|i| hooks_idx + 1 + i)
        .unwrap_or(lines.len());

    let pre = &lines[..hooks_idx];
    let post = &lines[hooks_block_end..];

    let mut hooks_block = String::from("hooks:\n");
    if !inline_empty {
        let existing_content = &lines[hooks_idx + 1..hooks_block_end];
        let preserved = remove_zedra_event_blocks(existing_content, HERMES_HOOK_EVENTS);
        if !preserved.trim().is_empty() {
            hooks_block.push_str(&preserved);
        }
    }
    hooks_block.push_str(&zedra_hooks_entries(&script));

    let mut out = String::new();
    for l in pre {
        out.push_str(l);
        out.push('\n');
    }
    out.push_str(&hooks_block);
    for l in post {
        out.push_str(l);
        out.push('\n');
    }
    while out.ends_with("\n\n") {
        out.pop();
    }
    if trailing_newline && !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Remove all Zedra-managed event blocks from config.yaml hooks section
/// (used by `--remove`).
fn remove_hermes_zedra_hooks(config: &str) -> String {
    let lines: Vec<&str> = config.lines().collect();
    let trailing_newline = config.ends_with('\n');

    let Some(hooks_idx) = lines.iter().position(|l| is_hooks_key_line(l)) else {
        return config.to_string();
    };

    let hooks_block_end = lines[hooks_idx + 1..]
        .iter()
        .position(|l| {
            !l.is_empty() && !l.starts_with(' ') && !l.starts_with('\t') && !l.starts_with('#')
        })
        .map(|i| hooks_idx + 1 + i)
        .unwrap_or(lines.len());

    let existing_content = &lines[hooks_idx + 1..hooks_block_end];
    let preserved = remove_zedra_event_blocks(existing_content, HERMES_HOOK_EVENTS);

    let mut hooks_block = String::from("hooks:");
    if preserved.trim().is_empty() {
        hooks_block.push_str(" {}\n");
    } else {
        hooks_block.push('\n');
        hooks_block.push_str(&preserved);
    }

    let mut out = String::new();
    for l in &lines[..hooks_idx] {
        out.push_str(l);
        out.push('\n');
    }
    out.push_str(&hooks_block);
    for l in &lines[hooks_block_end..] {
        out.push_str(l);
        out.push('\n');
    }
    while out.ends_with("\n\n") {
        out.pop();
    }
    if trailing_newline && !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn is_hooks_key_line(line: &str) -> bool {
    if line.starts_with(' ') || line.starts_with('\t') {
        return false;
    }
    let t = line.trim();
    t == "hooks: {}"
        || t == "hooks:{}"
        || t == "hooks:"
        || (t.starts_with("hooks:")
            && matches!(t.as_bytes().get(6), Some(b' ') | Some(b'{') | None))
}

fn remove_zedra_event_blocks(lines: &[&str], remove_events: &[&str]) -> String {
    let mut out = String::new();
    let mut skip = false;
    for line in lines {
        if let Some(key) = event_key_at_line(line) {
            skip = remove_events.contains(&key);
        }
        if !skip {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn event_key_at_line<'a>(line: &'a str) -> Option<&'a str> {
    let rest = line.strip_prefix("  ")?;
    if rest.starts_with(' ') {
        return None;
    }
    let name = rest.strip_suffix(':')?;
    if name.is_empty() || name.contains(' ') || name.contains('"') || name.contains('\'') {
        return None;
    }
    Some(name)
}

fn zedra_hooks_entries(script: &str) -> String {
    let script_yaml = script.replace('\\', "\\\\").replace('"', "\\\"");
    let mut out = String::new();
    for event in HERMES_HOOK_EVENTS {
        out.push_str(&format!("  {}:\n", event));
        out.push_str(&format!("    - command: \"{}\"\n", script_yaml));
        out.push_str("      timeout: 5\n");
    }
    out
}

fn pi_extension_path() -> Result<PathBuf> {
    Ok(home_dir()?
        .join(".pi")
        .join("agent")
        .join("extensions")
        .join(PI_HOOK_EXTENSION))
}

fn install_pi_hooks(full_bin_path: bool) -> Result<()> {
    let path = pi_extension_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, pi_hook_extension(&hook_binary(full_bin_path)?)?)?;
    print_step_label("hooks");
    print_success_detail(&format!("write {}", path.display()));
    Ok(())
}

fn remove_pi_hooks() -> Result<()> {
    let path = pi_extension_path()?;
    if remove_skill_path(&path)? {
        print_step_label("hooks");
        print_success_detail(&format!("remove {}", path.display()));
    }
    Ok(())
}

/// pi extension that mirrors the OpenCode plugin: it shells back into the zedra
/// binary on lifecycle events. It is a complete no-op outside a Zedra terminal
/// (no `ZEDRA_TERMINAL_ID`) and for non-interactive pi runs (`ctx.hasUI === false`,
/// i.e. subagents / `-p` / JSON mode). Dispatch is fire-and-forget so a missing
/// or slow zedra binary can never stall the pi agent loop.
///
/// pi's native events are normalized to the Claude-compatible names the host
/// receiver expects: `before_agent_start → UserPromptSubmit`, `agent_end → Stop`.
/// `session_shutdown → Stop` clears a stuck "running" state on Ctrl+C / quit.
fn pi_hook_extension(binary: &str) -> Result<String> {
    let binary = serde_json::to_string(binary)?;
    Ok(format!(
        r#"// zedra-agent-hook (pi lifecycle extension)
import {{ spawn }} from "node:child_process";

const zedra = {binary};

function fire(eventName) {{
  if (!process.env.ZEDRA_TERMINAL_ID) return;
  try {{
    const child = spawn(
      zedra,
      ["agent", "hook", "receive", "--agent", "pi", "--payload", JSON.stringify({{ hook_event_name: eventName }})],
      {{ stdio: ["ignore", "ignore", "ignore"], detached: true }},
    );
    child.on("error", () => {{}});
    child.unref();
  }} catch {{
    // spawn() can throw synchronously (EACCES, ENOENT). Stay silent.
  }}
}}

// Gate on ctx.hasUI === false (not !ctx.hasUI) so pi versions without the field
// still fire; only explicit non-interactive runs are skipped.
const skip = (ctx) => ctx?.hasUI === false;

export default function (pi) {{
  pi.on("before_agent_start", (_event, ctx) => {{ if (!skip(ctx)) fire("UserPromptSubmit"); }});
  pi.on("agent_end", (_event, ctx) => {{ if (!skip(ctx)) fire("Stop"); }});
  pi.on("session_shutdown", (_event, ctx) => {{ if (!skip(ctx)) fire("Stop"); }});
}}
"#
    ))
}

fn print_suggested_command(command: &str) {
    utils::println_command(command);
}

fn print_step_label(label: &str) {
    utils::println_step(label);
}

fn print_detail(detail: &str) {
    println!("{}", utils::command_text(detail));
}

fn print_success_detail(detail: &str) {
    utils::println_dim(detail);
}

fn print_error_detail(detail: &str) {
    utils::println_error(detail);
}

fn install_claude_hooks(full_bin_path: bool, quiet: bool) -> Result<()> {
    let path = home_dir()?.join(".claude").join("settings.json");
    merge_command_hooks(
        &path,
        &[
            ("UserPromptSubmit", None, 2),
            ("PermissionRequest", Some("*"), 2),
            ("PostToolUse", Some("*"), 2),
            ("Stop", None, 2),
        ],
        "Claude",
        full_bin_path,
        quiet,
    )
}

fn remove_claude_hooks() -> Result<()> {
    remove_command_hooks(&home_dir()?.join(".claude").join("settings.json"), "Claude")
}

fn install_codex_hooks(full_bin_path: bool, quiet: bool) -> Result<()> {
    let path = home_dir()?.join(".codex").join("hooks.json");
    merge_command_hooks(
        &path,
        &[
            ("UserPromptSubmit", None, 2),
            ("PermissionRequest", Some("*"), 30),
            ("PostToolUse", Some("*"), 2),
            ("Stop", None, 2),
        ],
        "Codex",
        full_bin_path,
        quiet,
    )
}

fn remove_codex_hooks() -> Result<()> {
    remove_command_hooks(&home_dir()?.join(".codex").join("hooks.json"), "Codex")
}

fn merge_command_hooks(
    path: &Path,
    events: &[(&str, Option<&str>, u64)],
    agent: &str,
    full_bin_path: bool,
    quiet: bool,
) -> Result<()> {
    let mut root = read_json_object(path)?;
    let command = hook_command(agent, full_bin_path, quiet)?;
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
    print_step_label("hooks");
    print_success_detail(&format!(
        "register: {}",
        events
            .iter()
            .map(|(event, _, _)| *event)
            .collect::<Vec<_>>()
            .join(", ")
    ));
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

fn install_opencode_hooks(full_bin_path: bool, quiet: bool) -> Result<()> {
    let dir = home_dir()?.join(".config").join("opencode");
    install_opencode_hooks_in_dir(&dir, &hook_binary(full_bin_path)?, quiet)
}

fn install_opencode_hooks_in_dir(dir: &Path, binary: &str, quiet: bool) -> Result<()> {
    let plugin_path = opencode_hook_plugin_path(dir);
    let content = opencode_hook_plugin(binary, quiet)?;
    if fs::read_to_string(&plugin_path).ok().as_deref() == Some(&content) {
        return Ok(());
    }
    if let Some(parent) = plugin_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&plugin_path, content)?;
    print_step_label("hooks");
    print_success_detail(&format!("write {}", plugin_path.display()));
    Ok(())
}

fn remove_opencode_hooks() -> Result<()> {
    let dir = home_dir()?.join(".config").join("opencode");
    remove_opencode_hooks_in_dir(&dir)
}

fn remove_opencode_hooks_in_dir(dir: &Path) -> Result<()> {
    let plugin_path = opencode_hook_plugin_path(dir);
    if remove_skill_path(&plugin_path)? {
        print_step_label("hooks");
        print_success_detail(&format!("remove {}", plugin_path.display()));
    }
    Ok(())
}

fn opencode_hook_plugin_path(dir: &Path) -> PathBuf {
    dir.join("plugins").join(OPENCODE_HOOK_PLUGIN)
}

fn hook_command(agent: &str, full_bin_path: bool, quiet: bool) -> Result<String> {
    let quiet_flag = if quiet { " --quiet" } else { "" };
    Ok(format!(
        "{} agent hook receive --agent {}{}",
        hook_command_program(full_bin_path)?,
        agent_slug(agent),
        quiet_flag,
    ))
}

fn hook_command_program(full_bin_path: bool) -> Result<String> {
    if full_bin_path {
        Ok(shell_quote(&current_zedra_binary()?))
    } else {
        Ok("zedra".to_string())
    }
}

fn hook_binary(full_bin_path: bool) -> Result<String> {
    if full_bin_path {
        current_zedra_binary()
    } else {
        Ok("zedra".to_string())
    }
}

fn current_zedra_binary() -> Result<String> {
    Ok(std::env::current_exe()
        .context("resolve current zedra binary")?
        .display()
        .to_string())
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
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

fn agent_slug(agent: &str) -> &str {
    match agent {
        "Claude" | "claude" => "claude",
        "Codex" | "codex" => "codex",
        "OpenCode" | "opencode" | "open-code" | "open_code" => "opencode",
        "Pi" | "pi" => "pi",
        "Hermes" | "hermes" => "hermes",
        _ => agent,
    }
}

fn opencode_hook_plugin(binary: &str, quiet: bool) -> Result<String> {
    let binary = serde_json::to_string(binary)?;
    let quiet_arg = if quiet { r#", "--quiet""# } else { "" };
    Ok(format!(
        r#"import {{ spawnSync }} from "node:child_process";

const zedra = {binary};

function send(event, payload = {{}}) {{
  spawnSync(zedra, ["agent", "hook", "receive", "--agent", "opencode"{quiet_arg}, "--payload", JSON.stringify({{ event_name: event, ...payload }})], {{
    stdio: ["ignore", "ignore", "ignore"],
    timeout: 2000,
  }});
}}

export const ZedraAgentHooks = async () => {{
  return {{
    event: async (input) => send(input.event?.type ?? "event", input),
  }};
}}
"#
    ))
}

async fn install_skills_from_raw(agent_name: &str, skills_dir: &Path) -> Result<()> {
    utils::println_section(format!("Setting up {agent_name}"));
    let client = reqwest::Client::new();
    let mut installed = 0usize;
    let mut skipped = Vec::new();

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

fn opencode_skills_dir() -> Result<PathBuf> {
    Ok(home_dir()?.join(".config").join("opencode").join("skills"))
}

fn home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| anyhow!("HOME is not set"))
}

fn cli_on_path(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

fn require_command(program: &str) -> Result<()> {
    if !cli_on_path(program) {
        anyhow::bail!("`{program}` was not found in PATH");
    }
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
        print!("{}", utils::dim_text(&command));
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
            &[
                ("Stop", None, 2),
                ("PermissionRequest", Some("*"), 30),
                ("PostToolUse", Some("*"), 2),
            ],
            "Codex",
            false,
            true,
        )
        .unwrap();
        let mut root = read_json_object(&path).unwrap();
        root["hooks"]["PermissionRequest"][0]["hooks"][0]["command"] =
            Value::String("/tmp/old-zedra agent hook receive --agent Codex".to_string());
        write_json_file(&path, &root).unwrap();
        merge_command_hooks(
            &path,
            &[
                ("Stop", None, 2),
                ("PermissionRequest", Some("*"), 30),
                ("PostToolUse", Some("*"), 2),
            ],
            "Codex",
            false,
            true,
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
        assert!(
            root["hooks"]["PermissionRequest"][0]["hooks"][0]["command"]
                .as_str()
                .unwrap()
                .contains(" agent hook receive --agent codex")
        );
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
        merge_command_hooks(&path, &[("Stop", None, 2)], "Claude", false, true).unwrap();
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
    fn hook_command_uses_lowercase_agent_slugs() {
        for (agent, slug) in [
            ("Claude", "claude"),
            ("Codex", "codex"),
            ("OpenCode", "opencode"),
            ("Pi", "pi"),
            ("Hermes", "hermes"),
        ] {
            assert_eq!(
                hook_command(agent, false, true).unwrap(),
                format!("zedra agent hook receive --agent {slug} --quiet")
            );
            assert_eq!(
                hook_command(agent, false, false).unwrap(),
                format!("zedra agent hook receive --agent {slug}")
            );
        }
    }

    #[test]
    fn hook_command_can_use_current_binary_path() {
        let command = hook_command("Codex", true, true).unwrap();
        assert!(command.ends_with(" agent hook receive --agent codex --quiet"));
        assert_ne!(command, "zedra agent hook receive --agent codex --quiet");
    }

    #[test]
    fn hermes_hook_script_preserves_quoted_binary_path() {
        let script = hermes_hook_script("/tmp/zedra build/zedra");
        assert!(script.contains("CLI=\"${ZEDRA_CLI:-}\""));
        assert!(script.contains("[ -n \"$CLI\" ] || CLI='/tmp/zedra build/zedra'"));
        assert!(!script.contains("CLI=\"${ZEDRA_CLI:-'"));
    }

    #[test]
    fn opencode_hook_install_and_remove_updates_plugin_directory() {
        let dir = tempfile::tempdir().unwrap();

        install_opencode_hooks_in_dir(dir.path(), "/tmp/zedra", true).unwrap();
        install_opencode_hooks_in_dir(dir.path(), "/tmp/zedra", true).unwrap();

        let plugin_path = opencode_hook_plugin_path(dir.path());
        let plugin = fs::read_to_string(&plugin_path).unwrap();
        assert!(plugin.contains(
            r#"spawnSync(zedra, ["agent", "hook", "receive", "--agent", "opencode", "--quiet""#
        ));
        assert!(plugin.contains("JSON.stringify({ event_name: event, ...payload })"));

        remove_opencode_hooks_in_dir(dir.path()).unwrap();

        assert!(!plugin_path.exists());
    }

    #[test]
    fn pi_extension_shells_into_zedra_and_is_detectable() {
        let ext = pi_hook_extension("/tmp/zedra").unwrap();
        // Shells back into the zedra binary with the pi agent slug.
        assert!(ext.contains(r#"["agent", "hook", "receive", "--agent", "pi", "--payload""#));
        // No-op guards: outside Zedra terminals and for non-interactive pi runs.
        assert!(ext.contains("if (!process.env.ZEDRA_TERMINAL_ID) return;"));
        assert!(ext.contains("ctx?.hasUI === false"));
        // Detection in agent_setup relies on this marker substring.
        assert!(ext.contains("zedra-agent-hook"));
    }
}
