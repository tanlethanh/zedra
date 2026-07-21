// PTY spawning for shell sessions
// Uses portable-pty for cross-platform PTY support

use crate::paths;
use anyhow::Result;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use zedra_rpc::proto::TerminalColorScheme;

pub type PtyParts = (
    Box<dyn Read + Send>,
    Box<dyn Write + Send>,
    Box<dyn MasterPty + Send>,
    Box<dyn Child + Send + Sync>,
);

/// A spawned shell session with PTY
pub struct ShellSession {
    master: Box<dyn MasterPty + Send>,
    reader: Box<dyn Read + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
}

/// Options for spawning a new shell session.
#[derive(Default)]
pub struct SpawnOptions {
    /// Working directory for the shell. Defaults to the process cwd if `None`.
    pub workdir: Option<std::path::PathBuf>,
    /// Shell command to run when the PTY starts.
    /// Example: `"claude --resume"` to drop straight into a Claude session.
    pub launch_cmd: Option<String>,
    /// Terminal appearance used for host-side OSC color query replies.
    pub color_scheme: Option<TerminalColorScheme>,
    /// Extra environment variables set on the spawned shell after sanitization.
    pub env: Vec<(String, String)>,
}

fn launch_script(launch_cmd: &str) -> String {
    format!("{launch_cmd}\nexec \"$SHELL\" -l")
}

impl ShellSession {
    /// Spawn a new shell with the given PTY size and options.
    pub fn spawn(columns: u16, rows: u16, opts: SpawnOptions) -> Result<Self> {
        let pty_system = native_pty_system();

        let pty_size = PtySize {
            rows,
            cols: columns,
            pixel_width: 0,
            pixel_height: 0,
        };

        let pair = pty_system
            .openpty(pty_size)
            .map_err(|e| anyhow::anyhow!("Failed to open PTY: {}", e))?;

        // Disable PTY echo before spawning. When the host (or client VTE) writes an OSC
        // color query reply to PTY master, an echo-on line discipline bounces those bytes
        // back out as caret-notation text (ESC → "^["), which the mobile VTE then renders
        // as garbage at the prompt. Disabling ECHO/ECHOCTL/ECHOKE fixes this without
        // touching OPOST/ONLCR, so newline translation stays intact for non-TUI output.
        #[cfg(unix)]
        if let Some(fd) = pair.master.as_raw_fd() {
            unsafe {
                let mut t: libc::termios = std::mem::MaybeUninit::zeroed().assume_init();
                if libc::tcgetattr(fd, &mut t) == 0 {
                    t.c_lflag &= !(libc::ECHO
                        | libc::ECHOE
                        | libc::ECHOK
                        | libc::ECHONL
                        | libc::ECHOCTL
                        | libc::ECHOKE);
                    if libc::tcsetattr(fd, libc::TCSANOW, &t) != 0 {
                        tracing::warn!(
                            "tcsetattr failed to disable PTY echo: {}",
                            std::io::Error::last_os_error()
                        );
                    }
                }
            }
        }

        let shell = default_shell();
        let mut cmd = CommandBuilder::new(&shell);
        configure_shell_command(&mut cmd, &shell, opts.launch_cmd.as_deref())?;

        // Start in the session working directory if provided.
        if let Some(dir) = &opts.workdir {
            cmd.cwd(paths::user_path(dir));
        }

        // Build a sanitized environment: start clean, allow only safe variables.
        // This prevents daemon secrets (AWS keys, tokens, etc.) from leaking into shells.
        cmd.env_clear();
        let config = crate::global_config::get();
        for key in allowed_env_vars() {
            if let Ok(val) = std::env::var(key) {
                cmd.env(key, val);
            }
        }
        // Opt-in passthrough: forward extra host env names into the shell.
        for key in &config.terminal.env_passthrough {
            if let Ok(val) = std::env::var(key) {
                cmd.env(key, val);
            }
        }
        set_shell_env(&mut cmd, &shell);
        // Always set a known-good TERM; override any inherited value.
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        // Configured env overrides the passthrough/allowlist; per-request env wins last.
        for (key, val) in &config.terminal.env {
            cmd.env(key, val);
        }
        for (key, val) in &opts.env {
            cmd.env(key, val);
        }

        // Spawn the shell process
        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| anyhow::anyhow!("Failed to spawn shell: {}", e))?;

        // Get read/write handles to the master side
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| anyhow::anyhow!("Failed to clone PTY reader: {}", e))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| anyhow::anyhow!("Failed to take PTY writer: {}", e))?;

        Ok(Self {
            master: pair.master,
            reader,
            writer,
            child,
        })
    }

    /// Split the session into its raw components for async I/O.
    pub fn take_reader(self) -> PtyParts {
        (self.reader, self.writer, self.master, self.child)
    }
}

#[cfg(windows)]
fn default_shell() -> String {
    windows_shell_from_env(&["ZEDRA_SHELL"])
        .or_else(|| windows_shell_from_env(&["ZEDRA_LAUNCH_SHELL"]))
        .or_else(|| crate::global_config::get().terminal.shell.clone())
        .or_else(detect_parent_shell)
        .or_else(|| windows_shell_from_env(&["SHELL"]))
        .or_else(|| windows_shell_from_env(&["COMSPEC", "ComSpec"]))
        .unwrap_or_else(|| "cmd.exe".to_string())
}

#[cfg(not(windows))]
fn default_shell() -> String {
    // Explicit config beats the inherited `$SHELL` so a user can pin a shell.
    crate::global_config::get()
        .terminal
        .shell
        .clone()
        .or_else(|| std::env::var("SHELL").ok())
        .unwrap_or_else(|| "/bin/bash".to_string())
}

#[cfg(windows)]
fn shell_from_env(keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        std::env::var(key)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

#[cfg(windows)]
fn windows_shell_from_env(keys: &[&str]) -> Option<String> {
    shell_from_env(keys).map(|shell| normalize_windows_shell_path(&shell))
}

#[cfg(windows)]
pub fn detect_parent_shell() -> Option<String> {
    detect_parent_shell_for_pid(std::process::id())
}

#[cfg(windows)]
fn detect_parent_shell_for_pid(pid: u32) -> Option<String> {
    use sysinfo::{Pid, System};

    let system = System::new_all();
    let mut current_pid = Pid::from_u32(pid);

    for _ in 0..8 {
        let parent_pid = system.process(current_pid)?.parent()?;
        let parent = system.process(parent_pid)?;
        if let Some(shell) = shell_from_process_names(
            parent.exe().and_then(|path| path.to_str()),
            parent.name().to_str(),
        ) {
            return Some(shell);
        }
        current_pid = parent_pid;
    }

    None
}

#[cfg(any(windows, test))]
fn shell_from_process_names(exe: Option<&str>, name: Option<&str>) -> Option<String> {
    exe.and_then(shell_from_process_name)
        .or_else(|| name.and_then(shell_from_process_name))
}

#[cfg(any(windows, test))]
fn shell_from_process_name(process_name: &str) -> Option<String> {
    (windows_shell_kind(process_name) != WindowsShellKind::Unknown)
        .then(|| normalize_windows_shell_path(process_name))
}

#[cfg(any(windows, test))]
fn normalize_windows_shell_path(shell: &str) -> String {
    if shell.starts_with('/') && !shell.starts_with("//") {
        let file_name = shell
            .rsplit('/')
            .find(|part| !part.is_empty())
            .unwrap_or(shell);
        if file_name.contains('.') {
            file_name.to_string()
        } else {
            format!("{file_name}.exe")
        }
    } else {
        shell.to_string()
    }
}

#[cfg(windows)]
fn configure_shell_command(
    cmd: &mut CommandBuilder,
    shell: &str,
    launch_cmd: Option<&str>,
) -> Result<()> {
    for arg in windows_shell_args(shell, launch_cmd)? {
        cmd.arg(arg);
    }
    Ok(())
}

#[cfg(not(windows))]
fn configure_shell_command(
    cmd: &mut CommandBuilder,
    _shell: &str,
    launch_cmd: Option<&str>,
) -> Result<()> {
    if let Some(launch_cmd) = launch_cmd.filter(|command| !command.is_empty()) {
        // Avoid typing into the PTY before the shell has drawn its prompt.
        cmd.arg("-l");
        cmd.arg("-c");
        cmd.arg(launch_script(launch_cmd));
    } else {
        cmd.arg("-l"); // Login shell
    }
    Ok(())
}

#[cfg(any(windows, test))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WindowsShellKind {
    Cmd,
    PowerShell,
    Posix,
    Unknown,
}

#[cfg(any(windows, test))]
fn windows_shell_args(shell: &str, launch_cmd: Option<&str>) -> Result<Vec<String>> {
    let launch_cmd = launch_cmd.filter(|command| !command.is_empty());

    match windows_shell_kind(shell) {
        WindowsShellKind::Cmd => {
            let mut args = vec!["/d".to_string()];
            if let Some(launch_cmd) = launch_cmd {
                args.push("/k".to_string());
                args.push(launch_cmd.to_string());
            }
            Ok(args)
        }
        WindowsShellKind::PowerShell => {
            let mut args = vec!["-NoLogo".to_string()];
            if let Some(launch_cmd) = launch_cmd {
                args.push("-NoExit".to_string());
                args.push("-Command".to_string());
                args.push(launch_cmd.to_string());
            }
            Ok(args)
        }
        WindowsShellKind::Posix => {
            let mut args = vec!["-l".to_string()];
            if let Some(launch_cmd) = launch_cmd {
                args.push("-c".to_string());
                args.push(launch_script(launch_cmd));
            }
            Ok(args)
        }
        WindowsShellKind::Unknown => {
            if launch_cmd.is_some() {
                anyhow::bail!(
                    "launch commands are not supported for Windows shell `{}`; \
                     set ZEDRA_SHELL to cmd.exe, pwsh.exe, powershell.exe, or bash.exe",
                    shell
                );
            }
            Ok(Vec::new())
        }
    }
}

#[cfg(any(windows, test))]
fn windows_shell_kind(shell: &str) -> WindowsShellKind {
    let file_name = shell
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(shell)
        .to_ascii_lowercase();
    let stem = file_name
        .strip_suffix(".exe")
        .or_else(|| file_name.strip_suffix(".cmd"))
        .or_else(|| file_name.strip_suffix(".bat"))
        .unwrap_or(&file_name);

    match stem {
        "cmd" => WindowsShellKind::Cmd,
        "pwsh" | "powershell" => WindowsShellKind::PowerShell,
        "bash" | "sh" | "zsh" | "fish" => WindowsShellKind::Posix,
        _ => WindowsShellKind::Unknown,
    }
}

#[cfg(windows)]
fn allowed_env_vars() -> &'static [&'static str] {
    &[
        "APPDATA",
        "COMSPEC",
        "ComSpec",
        "HOMEDRIVE",
        "HOMEPATH",
        "LANG",
        "LOCALAPPDATA",
        "PATH",
        "PATHEXT",
        "Path",
        "ProgramFiles",
        "SystemRoot",
        "TEMP",
        "TMP",
        "SHELL",
        "USERNAME",
        "USERPROFILE",
        "WINDIR",
    ]
}

#[cfg(not(windows))]
fn allowed_env_vars() -> &'static [&'static str] {
    &[
        "HOME",
        "PATH",
        "SHELL",
        "TERM",
        "LANG",
        "USER",
        "LOGNAME",
        "COLORTERM",
        "XDG_RUNTIME_DIR",
    ]
}

#[cfg(windows)]
fn set_shell_env(cmd: &mut CommandBuilder, shell: &str) {
    cmd.env("SHELL", shell);
    if windows_shell_kind(shell) == WindowsShellKind::Cmd {
        cmd.env("COMSPEC", shell);
        cmd.env("ComSpec", shell);
    }
}

#[cfg(not(windows))]
fn set_shell_env(cmd: &mut CommandBuilder, shell: &str) {
    cmd.env("SHELL", shell);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_script_runs_command_before_interactive_shell() {
        assert_eq!(
            launch_script("echo ready"),
            "echo ready\nexec \"$SHELL\" -l"
        );
    }

    #[test]
    fn windows_shell_kind_matches_common_shell_names() {
        assert_eq!(windows_shell_kind("cmd.exe"), WindowsShellKind::Cmd);
        assert_eq!(windows_shell_kind("/usr/bin/bash"), WindowsShellKind::Posix);
        assert_eq!(
            windows_shell_kind(r"C:\Program Files\PowerShell\7\pwsh.exe"),
            WindowsShellKind::PowerShell
        );
        assert_eq!(
            windows_shell_kind(r"C:\Program Files\Git\bin\bash.exe"),
            WindowsShellKind::Posix
        );
        assert_eq!(windows_shell_kind("nu.exe"), WindowsShellKind::Unknown);
    }

    #[test]
    fn windows_shell_normalizes_posix_paths_for_win32_spawn() {
        assert_eq!(normalize_windows_shell_path("/usr/bin/bash"), "bash.exe");
        assert_eq!(normalize_windows_shell_path("/bin/zsh"), "zsh.exe");
        assert_eq!(
            normalize_windows_shell_path(r"C:\Program Files\Git\bin\bash.exe"),
            r"C:\Program Files\Git\bin\bash.exe"
        );
    }

    #[test]
    fn windows_shell_uses_detected_process_path_before_name() {
        assert_eq!(
            shell_from_process_names(
                Some(r"C:\Program Files\PowerShell\7\pwsh.exe"),
                Some("WindowsTerminal.exe"),
            ),
            Some(r"C:\Program Files\PowerShell\7\pwsh.exe".to_string())
        );
        assert_eq!(
            shell_from_process_names(None, Some("powershell.exe")),
            Some("powershell.exe".to_string())
        );
        assert_eq!(shell_from_process_names(None, Some("cargo.exe")), None);
    }

    #[test]
    fn windows_shell_args_match_shell_family() {
        assert_eq!(
            windows_shell_args("cmd.exe", Some("echo ready")).unwrap(),
            vec!["/d", "/k", "echo ready"]
        );
        assert_eq!(
            windows_shell_args("pwsh.exe", Some("Write-Host ready")).unwrap(),
            vec!["-NoLogo", "-NoExit", "-Command", "Write-Host ready"]
        );
        assert_eq!(
            windows_shell_args(r"C:\Program Files\Git\bin\bash.exe", Some("echo ready")).unwrap(),
            vec!["-l", "-c", "echo ready\nexec \"$SHELL\" -l"]
        );
        assert!(windows_shell_args("nu.exe", Some("echo ready")).is_err());
    }

    #[cfg(not(windows))]
    #[test]
    fn launch_command_runs_without_typed_echo() {
        let session = ShellSession::spawn(
            80,
            24,
            SpawnOptions {
                workdir: None,
                launch_cmd: Some("printf 'ZEDRA_LAUNCH_OK\\n'; exit".to_string()),
                color_scheme: None,
                env: Vec::new(),
            },
        )
        .unwrap();
        let (mut reader, _writer, _master, _child) = session.take_reader();
        let mut output = String::new();
        reader.read_to_string(&mut output).unwrap();

        assert!(output.contains("ZEDRA_LAUNCH_OK"));
        assert!(
            !output.contains("printf 'ZEDRA_LAUNCH_OK"),
            "launch command was echoed into PTY output: {output:?}"
        );
    }

    #[test]
    fn test_spawn_shell() {
        let session = ShellSession::spawn(80, 24, SpawnOptions::default());
        assert!(
            session.is_ok(),
            "Failed to spawn shell: {:?}",
            session.err()
        );
    }

    #[test]
    fn test_shell_write_read() {
        let session = ShellSession::spawn(80, 24, SpawnOptions::default()).unwrap();
        let (mut reader, mut writer, _master, _child) = session.take_reader();

        // Writer should accept data
        assert!(writer.write_all(b"echo test\n").is_ok());
        assert!(writer.flush().is_ok());

        std::thread::sleep(std::time::Duration::from_millis(200));

        // Reader should return data
        let mut buf = [0u8; 4096];
        let n = reader.read(&mut buf).unwrap();
        assert!(n > 0);
    }

    #[test]
    fn test_shell_resize() {
        let session = ShellSession::spawn(80, 24, SpawnOptions::default()).unwrap();
        let (_reader, _writer, master, _child) = session.take_reader();

        let result = master.resize(PtySize {
            rows: 50,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        });
        assert!(result.is_ok());
    }
}
