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

impl ShellSession {
    /// Spawn a new shell with the given PTY size
    pub fn spawn(columns: u16, rows: u16) -> Result<Self> {
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

        // Set terminal type
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
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| anyhow::anyhow!("Failed to take PTY writer: {}", e))?;

        Ok(Self {
            master: pair.master,
            reader,
            writer,
        })
    }

    /// Write bytes to the PTY (stdin of the shell)
    pub fn write(&mut self, data: &[u8]) -> Result<()> {
        self.writer.write_all(data)?;
        self.writer.flush()?;
        Ok(())
    }

    /// Read bytes from the PTY (stdout/stderr of the shell)
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let n = self.reader.read(buf)?;
        Ok(n)
    }

    /// Resize the PTY
    pub fn resize(&self, columns: u16, rows: u16) -> Result<()> {
        self.master
            .resize(PtySize {
                rows,
                cols: columns,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| anyhow::anyhow!("Failed to resize PTY: {}", e))?;
        Ok(())
    }

    /// Get a clone of the reader for async I/O
    pub fn take_reader(self) -> (Box<dyn Read + Send>, Box<dyn Write + Send>, Box<dyn MasterPty + Send>) {
        (self.reader, self.writer, self.master)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spawn_shell() {
        let session = ShellSession::spawn(80, 24);
        assert!(session.is_ok(), "Failed to spawn shell: {:?}", session.err());
    }

    #[test]
    fn test_shell_write_read() {
        let mut session = ShellSession::spawn(80, 24).unwrap();

        // Write a command to the shell
        session.write(b"echo hello\n").unwrap();

        // Give it a moment to process
        std::thread::sleep(std::time::Duration::from_millis(200));

        // Read output
        let mut buf = [0u8; 4096];
        let n = session.read(&mut buf).unwrap();
        assert!(n > 0, "Expected some output from shell");
    }

    #[test]
    fn test_shell_resize() {
        let session = ShellSession::spawn(80, 24).unwrap();
        let result = session.resize(120, 40);
        assert!(result.is_ok());
    }

    #[test]
    fn test_take_reader_returns_all_parts() {
        let session = ShellSession::spawn(80, 24).unwrap();
        let (mut reader, mut writer, master) = session.take_reader();

        // Writer should accept data
        assert!(writer.write_all(b"echo test\n").is_ok());
        assert!(writer.flush().is_ok());

        std::thread::sleep(std::time::Duration::from_millis(200));

        // Reader should return data
        let mut buf = [0u8; 4096];
        let n = reader.read(&mut buf).unwrap();
        assert!(n > 0);

        // Master should accept resize
        let result = master.resize(PtySize {
            rows: 50,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        });
        assert!(result.is_ok());
    }
}
