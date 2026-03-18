// PTY spawning for shell sessions
// Uses portable-pty for cross-platform PTY support

use anyhow::Result;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};

/// A spawned shell session with PTY
pub struct ShellSession {
    master: Box<dyn MasterPty + Send>,
    reader: Box<dyn Read + Send>,
    writer: Box<dyn Write + Send>,
}

/// Options for spawning a new shell session.
#[derive(Default)]
pub struct SpawnOptions {
    /// Working directory for the shell. Defaults to the process cwd if `None`.
    pub workdir: Option<std::path::PathBuf>,
    /// Shell command to inject into the PTY immediately after the shell starts.
    /// The command is written as `<cmd>\n` so the shell executes it on startup.
    /// Example: `"claude --resume"` to drop straight into a Claude session.
    pub launch_cmd: Option<String>,
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

        // Determine shell to use
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());

        let mut cmd = CommandBuilder::new(&shell);
        cmd.arg("-l"); // Login shell

        // Start in the session working directory if provided.
        if let Some(dir) = &opts.workdir {
            cmd.cwd(dir);
        }

        // Build a sanitized environment: start clean, allow only safe variables.
        // This prevents daemon secrets (AWS keys, tokens, etc.) from leaking into shells.
        cmd.env_clear();
        let allowed = [
            "HOME", "PATH", "SHELL", "TERM", "LANG", "USER", "LOGNAME",
            "COLORTERM", "XDG_RUNTIME_DIR",
        ];
        for key in &allowed {
            if let Ok(val) = std::env::var(key) {
                cmd.env(key, val);
            }
        }
        // Always set a known-good TERM; override any inherited value.
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");

        // Spawn the shell process
        let _child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| anyhow::anyhow!("Failed to spawn shell: {}", e))?;

        // Get read/write handles to the master side
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| anyhow::anyhow!("Failed to clone PTY reader: {}", e))?;
        let mut writer = pair
            .master
            .take_writer()
            .map_err(|e| anyhow::anyhow!("Failed to take PTY writer: {}", e))?;

        // Inject the launch command. The bytes sit in the kernel PTY buffer and
        // are processed once the shell finishes sourcing its init files.
        if let Some(cmd) = &opts.launch_cmd {
            let mut line = cmd.as_bytes().to_vec();
            line.push(b'\n');
            writer
                .write_all(&line)
                .map_err(|e| anyhow::anyhow!("Failed to inject launch command: {}", e))?;
        }

        Ok(Self {
            master: pair.master,
            reader,
            writer,
        })
    }

    /// Split the session into its raw components for async I/O.
    pub fn take_reader(
        self,
    ) -> (
        Box<dyn Read + Send>,
        Box<dyn Write + Send>,
        Box<dyn MasterPty + Send>,
    ) {
        (self.reader, self.writer, self.master)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let (mut reader, mut writer, _master) = session.take_reader();

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
        let (_reader, _writer, master) = session.take_reader();

        let result = master.resize(PtySize {
            rows: 50,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        });
        assert!(result.is_ok());
    }
}
