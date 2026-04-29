use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Subcommand};
use std::fs;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const PLUGIN_REPO: &str = "tanlethanh/zedra-plugin";
const CLAUDE_MARKETPLACE: &str = "zedra";
const CLAUDE_PLUGIN: &str = "zedra@zedra";
const PLUGIN_GIT_URL: &str = "https://github.com/tanlethanh/zedra-plugin.git";
const PLUGIN_RAW_BASE: &str = "https://raw.githubusercontent.com/tanlethanh/zedra-plugin/main";
const CODEX_TERMINAL_TITLE_CONFIG: &str = r#"terminal_title = ["thread-title", "project-name"]"#;
const SKILL_NAMES: &[&str] = &[
    "zedra-start",
    "zedra-status",
    "zedra-stop",
    "zedra-terminal",
];

#[derive(Clone, Copy, Debug, Subcommand)]
pub enum SetupAgent {
    /// Manage the Claude Code plugin from the Zedra marketplace
    Claude {
        #[command(flatten)]
        action: SetupActionArgs,
    },
    /// Manage Codex skills and terminal title config
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
    /// Manage Zedra skills with Gemini CLI
    Gemini {
        #[command(flatten)]
        action: SetupActionArgs,
    },
}

pub async fn run(agent: SetupAgent, assume_yes: bool) -> Result<()> {
    match agent {
        SetupAgent::Claude { action } => setup_claude(action.into()),
        SetupAgent::Codex { action } => setup_codex(action.into(), assume_yes).await,
        SetupAgent::OpenCode { action } => setup_opencode(action.into()).await,
        SetupAgent::Gemini { action } => setup_gemini(action.into()).await,
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

    println!();
    println!("Claude setup removed.");
    println!("In Claude Code, reload plugins to apply the change:");
    print_suggested_command("/reload-plugins");
    Ok(())
}

async fn setup_codex(action: SetupAction, assume_yes: bool) -> Result<()> {
    match action {
        SetupAction::Install => setup_codex_install(assume_yes).await,
        SetupAction::Remove => setup_codex_remove(),
    }
}

async fn setup_codex_install(assume_yes: bool) -> Result<()> {
    let skills_dir = codex_skills_dir()?;
    install_skills_from_raw("Codex", &skills_dir, "Installing").await?;
    configure_codex_terminal_title(assume_yes)?;

    println!();
    println!("Codex setup complete.");
    println!("In Codex, reload skills if this session is already open, then run:");
    print_suggested_command("$zedra-start");
    Ok(())
}

fn setup_codex_remove() -> Result<()> {
    let skills_dir = codex_skills_dir()?;
    remove_installed_skills("Codex", &skills_dir)?;
    remove_codex_terminal_title_config()?;

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

    println!();
    println!("OpenCode setup complete.");
    println!("In OpenCode, run:");
    print_suggested_command("/zedra-start");
    Ok(())
}

fn setup_opencode_remove() -> Result<()> {
    let skills_dir = opencode_skills_dir()?;
    remove_installed_skills("OpenCode", &skills_dir)?;

    println!();
    println!("OpenCode setup removed.");
    println!("Restart OpenCode or reload skills to apply the change.");
    Ok(())
}

async fn setup_gemini(action: SetupAction) -> Result<()> {
    require_command("gemini")?;
    match action {
        SetupAction::Install => setup_gemini_install().await,
        SetupAction::Remove => setup_gemini_remove(),
    }
}

async fn setup_gemini_install() -> Result<()> {
    println!("Installing Zedra skills for Gemini:");
    if !install_gemini_skills_with_cli()? {
        println!("Falling back to direct skill install.");
        let skills_dir = gemini_skills_dir()?;
        install_skills_from_raw("Gemini", &skills_dir, "Installing").await?;
    }

    println!();
    println!("Gemini setup complete.");
    println!("In Gemini CLI, run:");
    print_suggested_command("/zedra-start");
    Ok(())
}

fn install_gemini_skills_with_cli() -> Result<bool> {
    let skills_dir = gemini_skills_dir()?;
    let mut failed = Vec::new();

    for skill in SKILL_NAMES {
        if skills_dir.join(skill).join("SKILL.md").exists() {
            print_step_label(skill);
            print_success_detail("already installed");
            continue;
        }

        let skill_path = format!("plugins/zedra/skills/{skill}");
        let args = [
            "skills",
            "install",
            PLUGIN_GIT_URL,
            "--path",
            skill_path.as_str(),
            "--scope",
            "user",
            "--consent",
        ];

        if !run_command_step_status(skill, "gemini", &args, true)? {
            failed.push(*skill);
        }
    }

    if !failed.is_empty() {
        println!("Gemini CLI install failed for: {}", failed.join(", "));
    }

    Ok(failed.is_empty())
}

fn setup_gemini_remove() -> Result<()> {
    println!("Removing Zedra skills for Gemini:");
    let skills_dir = gemini_skills_dir()?;
    let mut removed = uninstall_gemini_skills(&skills_dir)?;

    removed += remove_remaining_skill_files(&skills_dir)?;
    if removed == 0 {
        println!("No Zedra skills were installed.");
    }

    println!();
    println!("Gemini setup removed.");
    println!("Restart Gemini CLI or reload skills to apply the change.");
    Ok(())
}

fn uninstall_gemini_skills(skills_dir: &Path) -> Result<usize> {
    let mut removed = 0usize;
    for skill in SKILL_NAMES {
        if !skills_dir.join(skill).join("SKILL.md").exists() {
            continue;
        }

        if run_optional_command_step(
            skill,
            "gemini",
            &["skills", "uninstall", skill, "--scope", "user"],
        )? {
            removed += 1;
        }
    }
    Ok(removed)
}

fn remove_remaining_skill_files(skills_dir: &Path) -> Result<usize> {
    let mut removed = 0usize;
    for skill in SKILL_NAMES {
        let path = skills_dir.join(skill);
        if remove_skill_path(&path)? {
            removed += 1;
            print_step_label(skill);
            print_success_detail(&format!("remove {}", path.display()));
        }
    }
    Ok(removed)
}

fn print_suggested_command(command: &str) {
    println!("  {}", highlighted_command(command));
}

fn print_step_label(label: &str) {
    println!("> {label}");
}

fn print_detail(detail: &str) {
    println!("{detail}");
}

fn print_success_detail(detail: &str) {
    println!("{}", success_line(detail));
}

fn print_error_detail(detail: &str) {
    println!("{}", error_line(detail));
}

fn highlighted_command(command: &str) -> String {
    color_text(command, "1;36")
}

fn success_line(line: &str) -> String {
    color_text(line, "1;32")
}

fn error_line(line: &str) -> String {
    color_text(line, "1;31")
}

fn color_text(text: &str, color: &str) -> String {
    if std::io::stdout().is_terminal() {
        format!("\x1b[{color}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
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
        bail!("downloaded file is not a Codex/Gemini skill");
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

fn configure_codex_terminal_title(assume_yes: bool) -> Result<()> {
    let config_file = codex_config_file()?;
    let current = fs::read_to_string(&config_file).unwrap_or_default();

    if codex_terminal_title_configured(&current) {
        println!();
        println!("Codex terminal title already configured:");
        println!("  {}", config_file.display());
        return Ok(());
    }

    println!();
    println!("Zedra can update Codex so terminal titles include the Codex thread title.");
    println!("  File: {}", config_file.display());
    println!("  Set:  [tui] {CODEX_TERMINAL_TITLE_CONFIG}");

    if !confirm_codex_config_update(assume_yes)? {
        println!("Skipped Codex terminal title config.");
        return Ok(());
    }

    if let Some(parent) = config_file.parent() {
        fs::create_dir_all(parent)?;
    }
    if config_file.exists() {
        let backup = backup_path(&config_file);
        fs::copy(&config_file, &backup)?;
        println!("Backup written to: {}", backup.display());
    }

    fs::write(&config_file, update_codex_terminal_title_config(&current))?;
    println!("Updated Codex terminal title config.");
    Ok(())
}

fn remove_codex_terminal_title_config() -> Result<()> {
    let config_file = codex_config_file()?;
    let current = match fs::read_to_string(&config_file) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            println!("Codex config not found; nothing to update.");
            return Ok(());
        }
        Err(err) => return Err(err).with_context(|| format!("read {}", config_file.display())),
    };

    let Some(updated) = remove_codex_terminal_title_config_value(&current) else {
        println!("Codex terminal title config was not changed.");
        return Ok(());
    };

    let backup = backup_path(&config_file);
    fs::copy(&config_file, &backup)?;
    println!("Backup written to: {}", backup.display());

    fs::write(&config_file, updated)?;
    println!("Removed Zedra-managed Codex terminal title config.");
    Ok(())
}

fn confirm_codex_config_update(assume_yes: bool) -> Result<bool> {
    if assume_yes {
        return Ok(true);
    }

    let stdin = std::io::stdin();
    if !stdin.is_terminal() {
        return Ok(false);
    }

    eprint!("Update Codex config now? [Y/n] ");
    std::io::stderr().flush()?;

    let mut input = String::new();
    stdin.read_line(&mut input)?;
    let input = input.trim();
    Ok(!(input.eq_ignore_ascii_case("n") || input.eq_ignore_ascii_case("no")))
}

fn codex_config_file() -> Result<PathBuf> {
    if let Some(codex_home) = std::env::var_os("CODEX_HOME") {
        return Ok(PathBuf::from(codex_home).join("config.toml"));
    }

    Ok(home_dir()?.join(".codex").join("config.toml"))
}

fn codex_skills_dir() -> Result<PathBuf> {
    Ok(home_dir()?.join(".agents").join("skills"))
}

fn opencode_skills_dir() -> Result<PathBuf> {
    Ok(home_dir()?.join(".config").join("opencode").join("skills"))
}

fn gemini_skills_dir() -> Result<PathBuf> {
    Ok(home_dir()?.join(".gemini").join("skills"))
}

fn home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| anyhow!("HOME is not set"))
}

fn backup_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config.toml");
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    path.with_file_name(format!("{file_name}.zedra.bak.{timestamp}"))
}

fn codex_terminal_title_configured(contents: &str) -> bool {
    let mut in_tui = false;
    for line in contents.lines() {
        let trimmed = line.trim();
        if is_table_header(trimmed) {
            in_tui = trimmed == "[tui]";
            continue;
        }
        if in_tui && is_zedra_terminal_title_config(trimmed) {
            return true;
        }
    }
    false
}

fn update_codex_terminal_title_config(contents: &str) -> String {
    let mut out = Vec::new();
    let mut in_tui = false;
    let mut saw_tui = false;
    let mut wrote_title = false;

    for line in contents.lines() {
        let trimmed = line.trim();
        if is_table_header(trimmed) {
            if in_tui && !wrote_title {
                out.push(CODEX_TERMINAL_TITLE_CONFIG.to_string());
                wrote_title = true;
            }

            in_tui = trimmed == "[tui]";
            saw_tui |= in_tui;
            out.push(line.to_string());
            continue;
        }

        if in_tui && trimmed.starts_with("terminal_title") {
            if !wrote_title {
                out.push(CODEX_TERMINAL_TITLE_CONFIG.to_string());
                wrote_title = true;
            }
            continue;
        }

        out.push(line.to_string());
    }

    if in_tui && !wrote_title {
        out.push(CODEX_TERMINAL_TITLE_CONFIG.to_string());
    }

    if !saw_tui {
        if !out.is_empty() && out.last().is_some_and(|line| !line.is_empty()) {
            out.push(String::new());
        }
        out.push("[tui]".to_string());
        out.push(CODEX_TERMINAL_TITLE_CONFIG.to_string());
    }

    let mut updated = out.join("\n");
    updated.push('\n');
    updated
}

fn remove_codex_terminal_title_config_value(contents: &str) -> Option<String> {
    let mut out = Vec::new();
    let mut in_tui = false;
    let mut removed = false;

    for line in contents.lines() {
        let trimmed = line.trim();
        if is_table_header(trimmed) {
            in_tui = trimmed == "[tui]";
            out.push(line.to_string());
            continue;
        }

        if in_tui && trimmed.starts_with("terminal_title") {
            if is_zedra_terminal_title_config(trimmed) {
                removed = true;
                continue;
            }
        }

        out.push(line.to_string());
    }

    if !removed {
        return None;
    }

    let mut updated = out.join("\n");
    updated.push('\n');
    Some(updated)
}

fn is_table_header(trimmed: &str) -> bool {
    trimmed.starts_with('[') && trimmed.ends_with(']')
}

fn is_zedra_terminal_title_config(trimmed: &str) -> bool {
    compact_toml_line(trimmed) == r#"terminal_title=["thread-title","project-name"]"#
}

fn compact_toml_line(line: &str) -> String {
    line.chars().filter(|ch| !ch.is_whitespace()).collect()
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
    let terminal = std::io::stdout().is_terminal();

    if terminal {
        print!("{}", highlighted_command(&command));
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
        eprintln!("status: {}", output.status);
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
    fn update_codex_config_adds_tui_table() {
        let updated = update_codex_terminal_title_config("model = \"gpt-5.5\"\n");
        assert_eq!(
            updated,
            "model = \"gpt-5.5\"\n\n[tui]\nterminal_title = [\"thread-title\", \"project-name\"]\n"
        );
    }

    #[test]
    fn update_codex_config_replaces_existing_title() {
        let updated = update_codex_terminal_title_config(
            "[tui]\nterminal_title = [\"activity\"]\nfoo = true\n",
        );
        assert_eq!(
            updated,
            "[tui]\nterminal_title = [\"thread-title\", \"project-name\"]\nfoo = true\n"
        );
    }

    #[test]
    fn update_codex_config_appends_to_existing_tui_table() {
        let updated =
            update_codex_terminal_title_config("[tui]\nfoo = true\n[features]\nbar = true\n");
        assert_eq!(
            updated,
            "[tui]\nfoo = true\nterminal_title = [\"thread-title\", \"project-name\"]\n[features]\nbar = true\n"
        );
    }

    #[test]
    fn detects_configured_title_with_flexible_spacing() {
        assert!(codex_terminal_title_configured(
            "[tui]\nterminal_title=[\"thread-title\",\"project-name\"]\n"
        ));
    }

    #[test]
    fn remove_codex_config_only_removes_zedra_title() {
        let updated = remove_codex_terminal_title_config_value(
            "[tui]\nfoo = true\nterminal_title = [\"thread-title\", \"project-name\"]\n[features]\nbar = true\n",
        );
        assert_eq!(
            updated.as_deref(),
            Some("[tui]\nfoo = true\n[features]\nbar = true\n")
        );
    }

    #[test]
    fn remove_codex_config_preserves_non_zedra_title() {
        let updated =
            remove_codex_terminal_title_config_value("[tui]\nterminal_title = [\"activity\"]\n");
        assert_eq!(updated, None);
    }

    #[test]
    fn setup_action_from_remove_flag() {
        assert_eq!(SetupAction::from_remove_flag(false), SetupAction::Install);
        assert_eq!(SetupAction::from_remove_flag(true), SetupAction::Remove);
    }
}
